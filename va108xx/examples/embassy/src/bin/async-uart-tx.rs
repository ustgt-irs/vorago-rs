//! Asynchronous UART transmission example application.
//!
//! This application receives sends 4 strings with different sizes permanently using UART A.
//! Ports PA8 and PA9 are used for this.
//!
//! Instructions:
//!
//! 1. Tie a USB to UART converter with RX to PA9 and TX to PA8 for UART A.
//! 2. Connect to the serial interface by using an application like Putty or picocom. You can
//!    can verify the correctness of the sent strings.
#![no_std]
#![no_main]
// This imports the logger and the panic handler.
use embassy_example as _;

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Ticker};
use once_cell::sync::OnceCell;
use va108xx_hal::{
    gpio,
    pac::{self, interrupt},
    pins,
    prelude::*,
    uart, InterruptConfig,
};

const SYSCLK_FREQ: Hertz = Hertz::from_raw(50_000_000);

/// Token identifying the UART0 peripheral, set once at construction and read by the interrupt
/// handler, which does not have access to the [uart::asynch::Tx] driver itself.
static TX_TOKEN: OnceCell<uart::Bank> = OnceCell::new();

const STR_LIST: &[&str] = &[
    "Hello World\r\n",
    "Smoll\r\n",
    "A string which is larger than the FIFO size\r\n",
    "A really large string which is significantly larger than the FIFO size\r\n",
];

// main is itself an async function.
#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    defmt::println!("-- VA108xx Async UART TX Demo --");

    let dp = pac::Peripherals::take().unwrap();

    // Safety: Only called once here.
    va108xx_hal::embassy_time::init(dp.tim23, dp.tim22, SYSCLK_FREQ);

    let porta = pins::PinsA::new(dp.porta);

    let mut led0 = gpio::Output::new(porta.pa10, gpio::PinState::Low);
    let mut led1 = gpio::Output::new(porta.pa7, gpio::PinState::Low);
    let mut led2 = gpio::Output::new(porta.pa6, gpio::PinState::Low);

    let tx = porta.pa9;
    let rx = porta.pa8;

    let clock_config = uart::ClockConfig::calculate(50.MHz(), 115200.Hz(), uart::BaudMode::_16);
    let uart_config = uart::Config::new_with_clock_config(clock_config);
    let uarta = uart::Uart::new_with_interrupt_uart0(
        dp.uarta,
        tx,
        rx,
        uart_config,
        InterruptConfig::new(pac::Interrupt::OC2, true, true),
    );
    let (tx, _rx) = uarta.split();
    // Safety: We do not cancel futures.
    let mut async_tx = uart::asynch::Tx::new(tx);
    TX_TOKEN.set(async_tx.bank_id()).unwrap();
    let mut ticker = Ticker::every(Duration::from_secs(1));
    let mut idx = 0;
    loop {
        defmt::info!("Current time: {}", Instant::now().as_secs());
        led0.toggle();
        led1.toggle();
        led2.toggle();
        // Safety: We are sending static lifetime slices, and not cancelling the futures.
        async_tx.write_all(STR_LIST[idx].as_bytes()).await;
        idx += 1;
        if idx == STR_LIST.len() {
            idx = 0;
        }
        ticker.next().await;
    }
}

#[interrupt]
#[allow(non_snake_case)]
fn OC2() {
    uart::asynch::Tx::on_interrupt(*TX_TOKEN.get().unwrap());
}
