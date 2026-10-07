//! Blinky for the bootloader image slots. Each slot blinks at a different rate, so the running
//! slot is visible on the board.
#![no_main]
#![no_std]

use cortex_m_rt::entry;
use defmt_rtt as _;
use panic_probe as _;
use va416xx_hal::{
    gpio::{Output, PinState},
    pac,
    pins::PinsG,
};

#[cfg(feature = "slot-a")]
const SLOT: &str = "A";
#[cfg(feature = "slot-b")]
const SLOT: &str = "B";

#[cfg(feature = "slot-a")]
const DELAY_CYCLES: u32 = 1_000_000;
#[cfg(feature = "slot-b")]
const DELAY_CYCLES: u32 = 8_000_000;

#[entry]
fn main() -> ! {
    defmt::println!("VA416xx HAL blinky example for App Slot {}", SLOT);

    let dp = pac::Peripherals::take().unwrap();
    let portg = PinsG::new(dp.portg);
    let mut led = Output::new(portg.pg5, PinState::Low);
    loop {
        cortex_m::asm::delay(DELAY_CYCLES);
        led.toggle();
    }
}
