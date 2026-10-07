//! SPI example application.
//!
//! If you do not use the loopback mode, MOSI and MISO need to be tied together on the board.
#![no_main]
#![no_std]
use embedded_hal::delay::DelayNs;
// Import panic provider.
use panic_probe as _;
// Import logger.
use defmt_rtt as _;

use cortex_m_rt::entry;
use embedded_hal::spi::{Mode, MODE_0};
use simple_examples::peb1;
use va416xx_hal::clock::ClockConfigurator;
use va416xx_hal::timer::CountdownTimer;
use va416xx_hal::{
    pac,
    pins::{PinsB, PinsC},
    prelude::*,
    spi,
};

#[derive(PartialEq, Debug)]
pub enum ExampleSelect {
    // Enter loopback mode. It is not necessary to tie MOSI/MISO together for this
    Loopback,
    // You need to tie together MOSI/MISO in this mode.
    MosiMisoTiedTogether,
}

const EXAMPLE_SEL: ExampleSelect = ExampleSelect::Loopback;
const SPI_SPEED_KHZ: u32 = 1000;
const SPI_MODE: Mode = MODE_0;
const FILL_WORD: u8 = 0x0f;

#[entry]
fn main() -> ! {
    defmt::println!("-- VA416xx SPI example application --");
    let dp = pac::Peripherals::take().unwrap();
    // Use the external clock connected to XTAL_N.
    let clocks = ClockConfigurator::new(dp.clkgen)
        .xtal_n_clk_with_src_freq(peb1::EXTCLK_FREQ)
        .freeze()
        .unwrap();
    let mut delay = CountdownTimer::new(dp.tim1, &clocks);

    let pins_b = PinsB::new(dp.portb);
    let pins_c = PinsC::new(dp.portc);

    let mut spi_cfg = spi::Config::default();
    spi_cfg.clock =
        spi::ClockConfig::from_clks(&clocks, SPI_SPEED_KHZ.kHz()).expect("invalid target clock");
    spi_cfg.mode = SPI_MODE;

    if EXAMPLE_SEL == ExampleSelect::Loopback {
        spi_cfg.loopback_mode = true;
    }
    // Create SPI peripheral.
    let mut spi0 = spi::Spi::new_for_spi0(dp.spi0, (pins_b.pb15, pins_c.pc0, pins_c.pc1), spi_cfg);
    spi0.set_fill_word(FILL_WORD);
    loop {
        let tx_buf: [u8; 4] = [1, 2, 3, 0];
        let mut rx_buf: [u8; 4] = [0; 4];
        // Can't really verify correct behaviour here. Just verify nothing crazy happens or it hangs up.
        spi0.write(&[0x42, 0x43]);

        // Can't really verify correct behaviour here. Just verify nothing crazy happens or it hangs up.
        spi0.read(&mut rx_buf[0..2]);

        // If the pins are tied together, we should received exactly what we send.

        let mut inplace_buf = tx_buf;
        spi0.transfer_in_place(&mut inplace_buf);
        assert_eq!([1, 2, 3, 0], inplace_buf);

        spi0.transfer(&mut rx_buf, &tx_buf);
        assert_eq!(rx_buf, [1, 2, 3, 0]);
        defmt::info!("SPI transfers successful");
        delay.delay_ms(500);
    }
}
