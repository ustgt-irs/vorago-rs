//! SPI example application
#![no_main]
#![no_std]
use cortex_m_rt::entry;
use embedded_hal::{
    delay::DelayNs,
    spi::{Mode, MODE_0},
};
// Import panic provider.
use panic_probe as _;
// Import logger.
use defmt_rtt as _;
use va108xx_hal::{
    pac,
    pins::{PinsA, PinsB},
    prelude::*,
    spi::{self, configure_pin_as_hw_cs_pin, ClockConfig, Spi, TransferConfig},
    timer::CountdownTimer,
};

#[derive(PartialEq, Debug)]
pub enum ExampleSelect {
    // Enter loopback mode. It is not necessary to tie MOSI/MISO together for this
    Loopback,
    MosiMisoTiedTogetherManually,
}

#[derive(PartialEq, Debug)]
pub enum SpiBusSelect {
    SpiAPortA,
    SpiAPortB,
    SpiBPortB,
}

const EXAMPLE_SEL: ExampleSelect = ExampleSelect::Loopback;
const SPI_BUS_SEL: SpiBusSelect = SpiBusSelect::SpiBPortB;
const SPI_SPEED_KHZ: u32 = 1000;
const SPI_MODE: Mode = MODE_0;
const FILL_WORD: u8 = 0x0f;

#[entry]
fn main() -> ! {
    defmt::println!("-- VA108xx SPI example application--");
    let dp = pac::Peripherals::take().unwrap();
    let mut delay = CountdownTimer::new(dp.tim0, 50.MHz());

    let spi_clk_cfg = ClockConfig::from_clk(50.MHz(), SPI_SPEED_KHZ.kHz())
        .expect("creating SPI clock config failed");
    let pinsa = PinsA::new(dp.porta);
    let pinsb = PinsB::new(dp.portb);

    let mut spi_cfg = spi::Config::default();
    if EXAMPLE_SEL == ExampleSelect::Loopback {
        spi_cfg.loopback_mode = true
    }

    // Set up the SPI peripheral
    let mut spi = match SPI_BUS_SEL {
        SpiBusSelect::SpiAPortA => {
            let (sck, mosi, miso) = (pinsa.pa31, pinsa.pa30, pinsa.pa29);
            let mut spia = Spi::new_for_spi0(dp.spia, (sck, miso, mosi), spi_cfg);
            spia.set_fill_word(FILL_WORD);
            spia
        }
        SpiBusSelect::SpiAPortB => {
            let (sck, mosi, miso) = (pinsb.pb9, pinsb.pb8, pinsb.pb7);
            let mut spia = Spi::new_for_spi0(dp.spia, (sck, miso, mosi), spi_cfg);
            spia.set_fill_word(FILL_WORD);
            spia
        }
        SpiBusSelect::SpiBPortB => {
            let (sck, mosi, miso) = (pinsb.pb5, pinsb.pb4, pinsb.pb3);
            let mut spib = Spi::new_for_spi1(dp.spib, (sck, miso, mosi), spi_cfg);
            spib.set_fill_word(FILL_WORD);
            spib
        }
    };
    // Configure transfer specific properties here
    match SPI_BUS_SEL {
        SpiBusSelect::SpiAPortA | SpiBusSelect::SpiAPortB => {
            let transfer_cfg = TransferConfig {
                clk_cfg: Some(spi_clk_cfg),
                mode: Some(SPI_MODE),
                sod: true,
                hw_cs: None,
            };
            spi.configure_transfer(&transfer_cfg);
        }
        SpiBusSelect::SpiBPortB => {
            let hw_cs_pin = configure_pin_as_hw_cs_pin(pinsb.pb2);
            let transfer_cfg = TransferConfig {
                clk_cfg: Some(spi_clk_cfg),
                mode: Some(SPI_MODE),
                sod: false,
                hw_cs: Some(hw_cs_pin),
            };
            spi.configure_transfer(&transfer_cfg);
        }
    }

    // Application logic
    loop {
        let mut reply_buf: [u8; 8] = [0; 8];
        // Can't really verify correct reply here.
        spi.write(&[0x42]);
        // Because of the loopback mode, we should get back the fill word here.
        spi.read(&mut reply_buf[0..1]);
        assert_eq!(reply_buf[0], FILL_WORD);
        delay.delay_ms(500_u32);

        let tx_buf: [u8; 3] = [0x01, 0x02, 0x03];
        spi.transfer(&mut reply_buf[0..3], &tx_buf);
        assert_eq!(tx_buf, reply_buf[0..3]);
        defmt::info!(
            "Received reply: {}, {}, {}",
            reply_buf[0],
            reply_buf[1],
            reply_buf[2]
        );
        delay.delay_ms(500_u32);

        let mut tx_rx_buf: [u8; 3] = [0x03, 0x02, 0x01];
        spi.transfer_in_place(&mut tx_rx_buf);
        defmt::info!(
            "Received reply: {}, {}, {}",
            tx_rx_buf[0],
            tx_rx_buf[1],
            tx_rx_buf[2]
        );
        assert_eq!(&tx_rx_buf[0..3], &[0x03, 0x02, 0x01]);
    }
}
