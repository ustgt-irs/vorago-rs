//! Vorago flashloader which can be used to flash image A and image B via a simple
//! low-level CCSDS memory interface via a UART wire.
//!
//! This flash loader can be used after the bootloader was flashed to flash the images.
//! You can also use this as an starting application for a software update mechanism.
//!
//! Bootloader memory map
//!
//! * <0x0>     Bootloader start                         <code up to 0x3FFC bytes>
//! * <0x3FFC>  Bootloader CRC                           <word>
//! * <0x4000>  App image A start                        <code up to 0x1DFF8 (~120K) bytes>
//! * <0x21FF8> App image A CRC check length             <word>
//! * <0x21FFC> App image A CRC check value              <word>
//! * <0x22000> App image B start                        <code up to 0x1DFF8 (~120K) bytes>
//! * <0x3FFF8> App image B CRC check length             <word>
//! * <0x3FFFC> App image B CRC check value              <word>
//! * <0x40000>                                          <end>
#![no_main]
#![no_std]

use once_cell::sync::OnceCell;
use va416xx_hal::{clock::Clocks, edac, pac, time::Hertz, wdt::Wdt};

const EXTCLK_FREQ: u32 = 40_000_000;

const MAX_TC_SIZE: usize = 1024;
const MAX_TC_FRAME_SIZE: usize = cobs::max_encoding_length(MAX_TC_SIZE);

const MAX_TM_SIZE: usize = 128;
const MAX_TM_FRAME_SIZE: usize = cobs::max_encoding_length(MAX_TM_SIZE);

const UART_BAUDRATE: u32 = 115200;
const RX_DEBUGGING: bool = false;
const TX_DEBUGGING: bool = false;

pub trait WdtInterface {
    fn feed(&self);
}

pub struct OptWdt(Option<Wdt>);

impl WdtInterface for OptWdt {
    fn feed(&self) {
        if let Some(wdt) = &self.0 {
            wdt.feed();
        }
    }
}

use ringbuf::{
    traits::{Consumer, Observer, Producer, SplitRef},
    CachingCons, StaticProd, StaticRb,
};
use static_cell::StaticCell;

// Larger buffer for TC to be able to hold the possibly large memory write packets.
const BUF_RB_SIZE_TC: usize = 2048;
const SIZES_RB_SIZE_TC: usize = 16;

const BUF_RB_SIZE_TM: usize = 512;
const SIZES_RB_SIZE_TM: usize = 16;

// Ring buffers to handling variable sized telemetry
static BUF_RB_TM: StaticCell<StaticRb<u8, BUF_RB_SIZE_TM>> = StaticCell::new();
static SIZES_RB_TM: StaticCell<StaticRb<usize, SIZES_RB_SIZE_TM>> = StaticCell::new();

// Ring buffers to handling variable sized telecommands
static BUF_RB_TC: StaticCell<StaticRb<u8, BUF_RB_SIZE_TC>> = StaticCell::new();
static SIZES_RB_TC: StaticCell<StaticRb<usize, SIZES_RB_SIZE_TC>> = StaticCell::new();

pub struct DataProducer<const BUF_SIZE: usize, const SIZES_LEN: usize> {
    pub buf_prod: StaticProd<'static, u8, BUF_SIZE>,
    pub sizes_prod: StaticProd<'static, usize, SIZES_LEN>,
}

pub struct DataConsumer<const BUF_SIZE: usize, const SIZES_LEN: usize> {
    pub buf_cons: CachingCons<&'static StaticRb<u8, BUF_SIZE>>,
    pub sizes_cons: CachingCons<&'static StaticRb<usize, SIZES_LEN>>,
}

static CLOCKS: OnceCell<Clocks> = OnceCell::new();

#[rtic::app(device = pac, dispatchers = [U1, U2, U3])]
mod app {
    use super::*;
    use cortex_m::asm;
    use embassy_time::Timer;
    use embedded_io::Write;
    // Import panic provider.
    use panic_probe as _;
    // Import logger.
    use defmt_rtt as _;
    use flashloader_types::{create_tm_packet, va416xx::Request, AppSel, Response};
    use rtic::Mutex;
    use spacepackets::{CcsdsPacketReader, SpacePacketHeader};
    use va416xx_hal::clock::ClockConfigurator;
    use va416xx_hal::irq_router::enable_and_init_irq_router;
    use va416xx_hal::uart::InterruptContextTimeoutOrMaxSize;
    use va416xx_hal::{
        edac,
        gpio::{Output, PinState},
        nvm::Nvm,
        pac,
        pins::PinsG,
        uart::{self, Uart},
    };

    use crate::{setup_edac, EXTCLK_FREQ};

    #[derive(Default, Debug, Copy, Clone, PartialEq, Eq)]
    pub enum CobsReaderStates {
        #[default]
        WaitingForStart,
        WatingForEnd,
        FrameOverflow,
    }

    #[local]
    struct Local {
        uart_rx: uart::RxWithInterrupt,
        uart_tx: uart::Tx,
        rx_context: InterruptContextTimeoutOrMaxSize,
        rom_spi: Option<pac::Spi3>,
        // We handle all TM in one task.
        tm_cons: DataConsumer<BUF_RB_SIZE_TM, SIZES_RB_SIZE_TM>,
        // We consume all TC in one task.
        tc_cons: DataConsumer<BUF_RB_SIZE_TC, SIZES_RB_SIZE_TC>,
        // We produce all TC in one task.
        tc_prod: DataProducer<BUF_RB_SIZE_TC, SIZES_RB_SIZE_TC>,
        led: Output,
    }

    #[shared]
    struct Shared {
        // Having this shared allows multiple tasks to generate telemetry.
        tm_prod: DataProducer<BUF_RB_SIZE_TM, SIZES_RB_SIZE_TM>,
    }

    #[init]
    fn init(mut cx: init::Context) -> (Shared, Local) {
        defmt::println!("-- Vorago flashloader --");
        // Initialize the systick interrupt & obtain the token to prove that we did
        // Use the external clock connected to XTAL_N.
        let clocks = ClockConfigurator::new(cx.device.clkgen)
            .xtal_n_clk_with_src_freq(Hertz::from_raw(EXTCLK_FREQ))
            .freeze()
            .unwrap();

        enable_and_init_irq_router();
        setup_edac(&mut cx.device.sysconfig);

        let gpiog = PinsG::new(cx.device.portg);

        let clock_config = uart::ClockConfig::calculate_with_clocks(
            uart::Bank::Uart0,
            &clocks,
            fugit::HertzU32::from_raw(UART_BAUDRATE),
            uart::BaudMode::_16,
        );
        let uart_config = uart::Config::new_with_clock_config(clock_config);

        let uart0 = Uart::new_for_uart0(cx.device.uart0, gpiog.pg0, gpiog.pg1, uart_config);
        let (tx, rx) = uart0.split();
        let led = Output::new(gpiog.pg5, PinState::Low);

        let (buf_prod_tm, buf_cons_tm) = BUF_RB_TM
            .init(StaticRb::<u8, BUF_RB_SIZE_TM>::default())
            .split_ref();
        let (sizes_prod_tm, sizes_cons_tm) = SIZES_RB_TM
            .init(StaticRb::<usize, SIZES_RB_SIZE_TM>::default())
            .split_ref();

        let (buf_prod_tc, buf_cons_tc) = BUF_RB_TC
            .init(StaticRb::<u8, BUF_RB_SIZE_TC>::default())
            .split_ref();
        let (sizes_prod_tc, sizes_cons_tc) = SIZES_RB_TC
            .init(StaticRb::<usize, SIZES_RB_SIZE_TC>::default())
            .split_ref();

        va416xx_hal::embassy_time::init(cx.device.tim15, cx.device.tim14, &clocks);
        CLOCKS.set(clocks).unwrap();

        let mut rx = rx.into_rx_with_interrupt();
        let mut rx_context = InterruptContextTimeoutOrMaxSize::new(MAX_TC_FRAME_SIZE);
        rx.read_fixed_len_or_timeout_based_using_irq(&mut rx_context)
            .expect("initiating UART RX failed");
        tc_handler::spawn().unwrap();
        tm_tx_handler::spawn().unwrap();
        blinky::spawn().unwrap();
        (
            Shared {
                tm_prod: DataProducer {
                    buf_prod: buf_prod_tm,
                    sizes_prod: sizes_prod_tm,
                },
            },
            Local {
                uart_rx: rx,
                uart_tx: tx,
                rx_context,
                rom_spi: Some(cx.device.spi3),
                tm_cons: DataConsumer {
                    buf_cons: buf_cons_tm,
                    sizes_cons: sizes_cons_tm,
                },
                tc_cons: DataConsumer {
                    buf_cons: buf_cons_tc,
                    sizes_cons: sizes_cons_tc,
                },
                tc_prod: DataProducer {
                    buf_prod: buf_prod_tc,
                    sizes_prod: sizes_prod_tc,
                },
                led,
            },
        )
    }

    // `shared` cannot be accessed from this context
    #[idle]
    fn idle(_cx: idle::Context) -> ! {
        loop {
            asm::nop();
        }
    }

    // This is the interrupt handler to read all bytes received on the UART0.
    #[task(
        binds = UART0_RX,
        local = [
            cnt: u32 = 0,
            rx_buf: [u8; MAX_TC_FRAME_SIZE] = [0; MAX_TC_FRAME_SIZE],
            rx_context,
            uart_rx,
            tc_prod
        ],
    )]
    fn uart_rx_irq(cx: uart_rx_irq::Context) {
        match cx
            .local
            .uart_rx
            .on_interrupt_max_size_or_timeout_based(cx.local.rx_context, cx.local.rx_buf)
        {
            Ok(result) => {
                if RX_DEBUGGING {
                    defmt::info!("RX Info: {:?}", cx.local.rx_context);
                    defmt::info!("RX Result: {:?}", result);
                }
                if result.complete() {
                    // Check frame validity (must have COBS format) and decode the frame.
                    // Currently, we expect a full frame or a frame received through a timeout
                    // to be one COBS frame. We could parse for multiple COBS packets in one
                    // frame, but the additional complexity is not necessary here..
                    if cx.local.rx_buf[0] == 0 && cx.local.rx_buf[result.bytes_read - 1] == 0 {
                        let decoded_size =
                            cobs::decode_in_place(&mut cx.local.rx_buf[1..result.bytes_read]);
                        if decoded_size.is_err() {
                            defmt::warn!("COBS decoding failed");
                        } else {
                            let decoded_size = decoded_size.unwrap();
                            if cx.local.tc_prod.sizes_prod.vacant_len() >= 1
                                && cx.local.tc_prod.buf_prod.vacant_len() >= decoded_size
                            {
                                // Should never fail, we checked there is enough space.
                                cx.local.tc_prod.sizes_prod.try_push(decoded_size).unwrap();
                                cx.local
                                    .tc_prod
                                    .buf_prod
                                    .push_slice(&cx.local.rx_buf[1..1 + decoded_size]);
                            } else {
                                defmt::warn!("COBS TC queue full");
                            }
                        }
                    } else {
                        defmt::warn!(
                            "COBS frame with invalid format, start and end bytes are not 0"
                        );
                    }

                    // Initiate next transfer.
                    cx.local
                        .uart_rx
                        .read_fixed_len_or_timeout_based_using_irq(cx.local.rx_context)
                        .expect("read operation failed");
                }
                if result.has_errors() {
                    defmt::warn!("UART error: {:?}", result.errors.unwrap());
                }
            }
            Err(e) => {
                defmt::warn!("UART error: {:?}", e);
            }
        }
    }

    #[task(
        priority = 2,
        local=[
            tc_buf: [u8; MAX_TC_SIZE] = [0; MAX_TC_SIZE],
            tm_buf: [u8; MAX_TM_SIZE] = [0; MAX_TM_SIZE],
            tc_cons,
            rom_spi,
        ],
        shared=[tm_prod]
    )]
    async fn tc_handler(mut cx: tc_handler::Context) {
        loop {
            // Try to read a TC from the ring buffer.
            let packet_len = cx.local.tc_cons.sizes_cons.try_pop();
            if packet_len.is_none() {
                // Small delay, TCs might arrive very quickly.
                Timer::after_millis(20).await;
                continue;
            }
            let packet_len = packet_len.unwrap();
            defmt::info!("received packet with length {}", packet_len);
            assert_eq!(
                cx.local
                    .tc_cons
                    .buf_cons
                    .pop_slice(&mut cx.local.tc_buf[0..packet_len]),
                packet_len
            );
            handle_tc(&mut cx, packet_len);
        }
    }

    fn handle_tc(cx: &mut tc_handler::Context, packet_len: usize) {
        let packet = match CcsdsPacketReader::new_with_checksum(&cx.local.tc_buf[0..packet_len]) {
            Ok(packet) => packet,
            Err(e) => {
                defmt::warn!("CCSDS packet error: {}", e);
                return;
            }
        };
        let (request, remainder) = match postcard::take_from_bytes::<Request>(packet.user_data()) {
            Ok(result) => result,
            Err(e) => {
                defmt::warn!("failed to parse request: {}", e);
                return;
            }
        };
        let rom_spi = cx.local.rom_spi.take().unwrap();
        let nvm = Nvm::new(rom_spi, CLOCKS.get().unwrap());
        match request {
            Request::Ping => defmt::info!("received ping TC"),
            Request::Corrupt(app_sel) => {
                defmt::info!("corrupting App Image {}", app_sel);
                let base_addr = match app_sel {
                    AppSel::A => flashloader_types::va416xx::APP_A_START_ADDR,
                    AppSel::B => flashloader_types::va416xx::APP_B_START_ADDR,
                };
                let mut buf = [0u8; 4];
                nvm.read_data(base_addr + 32, &mut buf);
                buf[0] = buf[0].wrapping_add(1);
                nvm.write_data(base_addr + 32, &buf);
            }
            Request::WriteNvm { offset } => {
                defmt::info!(
                    "writing {} bytes at offset {:#x} to NVM",
                    remainder.len(),
                    offset
                );
                nvm.write_data(offset, remainder);
                defmt::info!("NVM operation done");
            }
        }
        *cx.local.rom_spi = Some(nvm.release());

        let tm_len = create_tm_packet(
            cx.local.tm_buf,
            SpacePacketHeader::new_from_apid(flashloader_types::APID),
            Response::Ok,
        )
        .expect("creating TM packet failed");
        let tm = &cx.local.tm_buf[0..tm_len];
        cx.shared.tm_prod.lock(|prod| {
            if prod.sizes_prod.vacant_len() >= 1 && prod.buf_prod.vacant_len() >= tm_len {
                prod.sizes_prod.try_push(tm_len).unwrap();
                prod.buf_prod.push_slice(tm);
            } else {
                defmt::warn!("TM queue full");
            }
        });
    }

    #[task(
        priority = 1,
        local=[
            read_buf: [u8;MAX_TM_SIZE] = [0; MAX_TM_SIZE],
            encoded_buf: [u8;MAX_TM_FRAME_SIZE] = [0; MAX_TM_FRAME_SIZE],
            uart_tx,
            tm_cons
        ],
        shared=[]
    )]
    async fn tm_tx_handler(cx: tm_tx_handler::Context) {
        loop {
            while cx.local.tm_cons.sizes_cons.occupied_len() > 0 {
                let next_size = cx.local.tm_cons.sizes_cons.try_pop().unwrap();
                cx.local
                    .tm_cons
                    .buf_cons
                    .pop_slice(&mut cx.local.read_buf[0..next_size]);
                cx.local.encoded_buf[0] = 0;
                let send_size = cobs::encode(
                    &cx.local.read_buf[0..next_size],
                    &mut cx.local.encoded_buf[1..],
                );
                cx.local.encoded_buf[send_size + 1] = 0;
                if TX_DEBUGGING {
                    defmt::debug!("UART TX: Sending data with size {}", send_size + 2);
                }
                cx.local
                    .uart_tx
                    .write_all(&cx.local.encoded_buf[0..send_size + 2])
                    .unwrap();
                Timer::after_millis(2).await;
            }
            Timer::after_millis(50).await;
        }
    }

    /// Heartbeat, shows that the flashloader is still running.
    #[task(priority = 1, local = [led])]
    async fn blinky(cx: blinky::Context) {
        loop {
            cx.local.led.toggle();
            Timer::after_millis(500).await;
        }
    }

    #[task(binds = EDAC_SBE, priority = 1)]
    fn edac_sbe_isr(_cx: edac_sbe_isr::Context) {
        // TODO: Send some command via UART for notification purposes. Also identify the problematic
        // memory.
        edac::clear_sbe_irq();
    }

    #[task(binds = EDAC_MBE, priority = 1)]
    fn edac_mbe_isr(_cx: edac_mbe_isr::Context) {
        // TODO: Send some command via UART for notification purposes.
        edac::clear_mbe_irq();
        // TODO: Reset like the vorago example?
    }

    #[task(binds = WATCHDOG, priority = 1)]
    fn watchdog_isr(_cx: watchdog_isr::Context) {
        let wdt = unsafe { pac::WatchDog::steal() };
        // Clear interrupt.
        wdt.wdogintclr().write(|w| unsafe { w.bits(1) });
    }
}

fn setup_edac(syscfg: &mut pac::Sysconfig) {
    // The scrub values are based on the Vorago provided bootloader.
    edac::enable_rom_scrub(syscfg, 125);
    edac::enable_ram0_scrub(syscfg, 1000);
    edac::enable_ram1_scrub(syscfg, 1000);
    edac::enable_sbe_irq();
    edac::enable_mbe_irq();
}
