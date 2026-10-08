//! Vorago flashloader which can be used to flash image A and image B via a simple
//! low-level CCSDS memory interface via a UART interface.
#![no_main]
#![no_std]

use defmt_rtt as _; // global logger
use panic_probe as _;
use va108xx_hal::time::Hertz;

const SYSCLK_FREQ: Hertz = Hertz::from_raw(50_000_000);
const UART_BANK: va108xx_hal::uart::Bank = va108xx_hal::uart::Bank::Uart0;

pub const MAX_TC_SIZE: usize = 524;
pub const MAX_TC_FRAME_SIZE: usize = cobs::max_encoding_length(MAX_TC_SIZE);

const MAX_TM_SIZE: usize = 128;
const MAX_TM_FRAME_SIZE: usize = cobs::max_encoding_length(MAX_TM_SIZE);

const UART_BAUDRATE: u32 = 115200;

const TC_PIPE_SIZE: usize = 1024;
const TM_PIPE_SIZE: usize = 128;

pub const APP_A_START_ADDR: u32 = 0x3000;
pub const APP_A_END_ADDR: u32 = 0x117FC;
pub const APP_B_START_ADDR: u32 = APP_A_END_ADDR;
pub const APP_B_END_ADDR: u32 = 0x20000;

pub const PREFERRED_SLOT_OFFSET: u32 = 0x20000 - 1;

#[rtic::app(device = pac, dispatchers = [OC20, OC21, OC22])]
mod app {
    use super::*;
    use cobs::CobsDecoderHeapless;
    use cortex_m::asm;
    use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
    use embedded_io_async::Write as _;
    use flashloader_types::{create_encoded_tm_packet, va108xx::Request, AppSel, Response};
    use spacepackets::{CcsdsPacketReader, SpacePacketHeader};
    use va108xx_hal::pins::PinsA;
    use va108xx_hal::spi::ClockConfig;
    use va108xx_hal::uart::{self, asynch};
    use va108xx_hal::{pac, InterruptConfig};
    use vorago_reb1::m95m01::M95M01;

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
        uart_tx: asynch::Tx,
        nvm: M95M01,
        tc_tx: embassy_sync::pipe::Writer<'static, CriticalSectionRawMutex, TC_PIPE_SIZE>,
        tc_rx: embassy_sync::pipe::Reader<'static, CriticalSectionRawMutex, TC_PIPE_SIZE>,
        tm_tx: embassy_sync::pipe::Writer<'static, CriticalSectionRawMutex, TM_PIPE_SIZE>,
        tm_rx: embassy_sync::pipe::Reader<'static, CriticalSectionRawMutex, TM_PIPE_SIZE>,
    }

    #[shared]
    struct Shared {}

    #[init]
    fn init(cx: init::Context) -> (Shared, Local) {
        defmt::println!("-- Vorago flashloader --");

        let periphs = cx.device;
        va108xx_hal::embassy_time::init(periphs.tim14, periphs.tim15, SYSCLK_FREQ);

        let spi_clock_config = ClockConfig::new(2, 4);
        let nvm = M95M01::new(periphs.spic, spi_clock_config);

        let gpioa = PinsA::new(periphs.porta);
        let tx = gpioa.pa9;
        let rx = gpioa.pa8;

        let clock_config = uart::ClockConfig::calculate(
            SYSCLK_FREQ,
            fugit::HertzU32::from_raw(UART_BAUDRATE),
            uart::BaudMode::_16,
        );
        let uart_config = uart::Config::new_with_clock_config(clock_config);
        let irq_uart = uart::Uart::new_with_interrupt_uart0(
            periphs.uarta,
            tx,
            rx,
            uart_config,
            InterruptConfig::new(pac::Interrupt::OC0, true, true),
        );
        let (tx, rx) = irq_uart.split();
        // Unwrap is okay, we explicitely set the interrupt ID.
        let mut rx = rx.into_rx_with_interrupt();

        rx.start();
        tc_handler::spawn().unwrap();
        tm_tx_handler::spawn().unwrap();

        // Safety: We do not cancel futures.
        let tx_async = asynch::Tx::new(tx);

        static TC_PIPE: static_cell::ConstStaticCell<
            embassy_sync::pipe::Pipe<CriticalSectionRawMutex, TC_PIPE_SIZE>,
        > = static_cell::ConstStaticCell::new(embassy_sync::pipe::Pipe::new());
        static TM_PIPE: static_cell::ConstStaticCell<
            embassy_sync::pipe::Pipe<CriticalSectionRawMutex, TM_PIPE_SIZE>,
        > = static_cell::ConstStaticCell::new(embassy_sync::pipe::Pipe::new());
        let (tc_rx, tc_tx) = TC_PIPE.take().split();
        let (tm_rx, tm_tx) = TM_PIPE.take().split();
        (
            Shared {},
            Local {
                uart_rx: rx,
                uart_tx: tx_async,
                nvm,
                tc_tx,
                tc_rx,
                tm_tx,
                tm_rx,
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
        binds = OC0,
        local = [
            uart_rx,
            tc_tx
        ],
    )]
    fn uart_irq(cx: uart_irq::Context) {
        let mut buf: [u8; 16] = [0; 16];
        let result = cx.local.uart_rx.on_interrupt_owned(&mut buf);
        if result.bytes_read > 0 {
            let mut written_so_far = 0;
            while written_so_far < result.bytes_read {
                let write_result = cx
                    .local
                    .tc_tx
                    .try_write(&buf[written_so_far..result.bytes_read]);
                if write_result.is_err() {
                    defmt::warn!("TC pipe full, dropping bytes");
                    break;
                }
                written_so_far += write_result.unwrap();
            }
        }
        va108xx_hal::uart::asynch::Tx::on_interrupt(UART_BANK);
    }

    #[task(
        priority = 2,
        local=[
            nvm,
            tc_rx,
            tm_tx
        ],
    )]
    async fn tc_handler(cx: tc_handler::Context) {
        let mut read_buf: [u8; 64] = [0; 64];
        let mut tm_buf: [u8; MAX_TM_FRAME_SIZE] = [0; MAX_TM_FRAME_SIZE];
        let mut encoded_tm_buf: [u8; MAX_TM_FRAME_SIZE] = [0; MAX_TM_FRAME_SIZE];
        let mut cobs_decoder = CobsDecoderHeapless::<TC_PIPE_SIZE>::new();
        loop {
            let read_bytes = cx.local.tc_rx.read(&mut read_buf).await;
            for &byte in read_buf[0..read_bytes].iter() {
                match cobs_decoder.feed(byte) {
                    Ok(result) => {
                        if result.is_none() {
                            continue;
                        }
                        let frame = result.unwrap();
                        match CcsdsPacketReader::new_with_checksum(&cobs_decoder.dest()[0..frame]) {
                            Ok(packet) => {
                                let request =
                                    postcard::take_from_bytes::<Request>(packet.user_data());
                                if request.is_err() {
                                    defmt::warn!(
                                        "Failed to parse command: {}",
                                        request.err().unwrap()
                                    );
                                    continue;
                                }
                                let (request, remainder) = request.unwrap();
                                let response = match request {
                                    Request::Corrupt(slot) => {
                                        match slot {
                                            AppSel::A => {
                                                defmt::info!("corrupting App Image A");
                                                corrupt_image(APP_A_START_ADDR, cx.local.nvm);
                                            }
                                            AppSel::B => {
                                                defmt::info!("corrupting App Image B");
                                                corrupt_image(APP_B_START_ADDR, cx.local.nvm);
                                            }
                                        }
                                        Response::Ok
                                    }
                                    Request::WriteNvm { offset } => {
                                        defmt::info!(
                                            "writing {} bytes to NVM at offset 0x{:08x}",
                                            remainder.len(),
                                            offset
                                        );
                                        cx.local.nvm.write(offset as usize, remainder);
                                        defmt::info!("write complete");
                                        Response::Ok
                                    }
                                    Request::SetBootSlot(app_sel) => {
                                        defmt::info!(
                                            "received boot selection command with app select: {:?}",
                                            app_sel
                                        );
                                        cx.local.nvm.write(
                                            PREFERRED_SLOT_OFFSET as usize,
                                            &[app_sel as u8],
                                        );
                                        Response::Ok
                                    }
                                    Request::Ping => {
                                        defmt::info!("received ping TC");
                                        Response::Ok
                                    }
                                };
                                match create_encoded_tm_packet(
                                    &mut tm_buf,
                                    &mut encoded_tm_buf,
                                    SpacePacketHeader::new_from_apid(flashloader_types::APID),
                                    response,
                                ) {
                                    Ok(encoded_len) => {
                                        cx.local
                                            .tm_tx
                                            .write_all(&encoded_tm_buf[0..encoded_len])
                                            .await;
                                    }
                                    Err(e) => {
                                        defmt::warn!("Failed to create or encode TM packet: {}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                defmt::warn!("CCSDS packet error: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        defmt::warn!("COBS decoding error: {}", e);
                    }
                }
            }
        }
    }

    pub fn corrupt_image(base_addr: u32, nvm: &mut M95M01) {
        let mut buf = [0u8; 4];
        nvm.read(base_addr as usize + 32, &mut buf);
        buf[0] += 1;
        nvm.write(base_addr as usize + 32, &buf);
    }

    #[task(
        priority = 1,
        local=[
            uart_tx,
            tm_rx
        ],
    )]
    async fn tm_tx_handler(cx: tm_tx_handler::Context) {
        let mut buf: [u8; 256] = [0; 256];
        loop {
            let read_len = cx.local.tm_rx.read(&mut buf).await;
            // Safety: The buffer outlives the UART TX structure.
            if let Err(e) = cx.local.uart_tx.write_all(&buf[0..read_len]).await {
                defmt::warn!("UART TX overrun error: {}", e);
            }
        }
    }
}
