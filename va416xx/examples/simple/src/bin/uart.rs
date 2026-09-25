// UART example application. Sends a test string over a UART and then enters
// echo mode
#![no_main]
#![no_std]
// Import panic provider.
use panic_probe as _;
// Import logger.
use defmt_rtt as _;

use cortex_m_rt::entry;
use embedded_hal_nb::serial::Read;
use embedded_io::Write;
use simple_examples::peb1;
use va416xx_hal::clock::{ClockConfigurator, ClockSelect};
use va416xx_hal::pins::PinsG;
use va416xx_hal::time::Hertz;
use va416xx_hal::{pac, prelude::*, uart};

#[entry]
fn main() -> ! {
    defmt::println!("-- VA416xx UART example application--");

    let dp = pac::Peripherals::take().unwrap();

    // Feed the external clock connected to XTAL_N into the PLL to get 100 MHz.
    let clocks = ClockConfigurator::new(dp.clkgen)
        .xtal_n_clk_with_src_freq(peb1::EXTCLK_FREQ)
        .clksel_sys(ClockSelect::Pll)
        .pll_output_freq(100.MHz())
        .freeze()
        .unwrap();
    defmt::info!("System clocks: {}", clocks);
    defmt::info!("Priting hello world to UART and then entering echo mode");

    let gpiog = PinsG::new(dp.portg);

    let clock_config = uart::ClockConfig::calculate_with_clocks(
        uart::Bank::Uart0,
        &clocks,
        Hertz::from_raw(115200),
        uart::BaudMode::_16,
    );
    let uart_config = uart::Config::new_with_clock_config(clock_config);
    let uart0 = uart::Uart::new_for_uart0(dp.uart0, gpiog.pg0, gpiog.pg1, uart_config);
    let (mut tx, mut rx) = uart0.split();
    writeln!(tx, "Hello World\n\r").unwrap();
    loop {
        // Echo what is received on the serial link.
        match nb::block!(rx.read()) {
            Ok(recvd) => {
                // Infallible operation.
                embedded_hal_nb::serial::Write::write(&mut tx, recvd).unwrap();
            }
            Err(e) => {
                defmt::info!("UART RX error {:?}", e);
            }
        }
    }
}
