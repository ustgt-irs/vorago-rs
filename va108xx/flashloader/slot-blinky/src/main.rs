//! Blinky for the bootloader image slots. Each slot uses a different LED and blink rate,
//! so the running slot is visible on the board.
#![no_main]
#![no_std]

use cortex_m_rt::entry;
use defmt_rtt as _;
use embedded_hal::delay::DelayNs;
use panic_probe as _;
use va108xx_hal::{
    gpio::{Output, PinState},
    pac,
    pins::PinsA,
    prelude::*,
    timer::CountdownTimer,
};

#[cfg(feature = "slot-a")]
const SLOT: &str = "A";
#[cfg(feature = "slot-b")]
const SLOT: &str = "B";

#[cfg(feature = "slot-a")]
const BLINK_PERIOD_MS: u32 = 500;
#[cfg(feature = "slot-b")]
const BLINK_PERIOD_MS: u32 = 1000;

#[entry]
fn main() -> ! {
    defmt::println!("VA108xx HAL blinky example for App Slot {}", SLOT);

    let dp = pac::Peripherals::take().unwrap();
    let mut timer = CountdownTimer::new(dp.tim0, 50.MHz());
    let porta = PinsA::new(dp.porta);
    #[cfg(feature = "slot-a")]
    let mut led = Output::new(porta.pa10, PinState::Low);
    #[cfg(feature = "slot-b")]
    let mut led = Output::new(porta.pa7, PinState::Low);

    loop {
        led.toggle();
        timer.delay_ms(BLINK_PERIOD_MS);
    }
}
