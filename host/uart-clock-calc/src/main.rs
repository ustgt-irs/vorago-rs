use arbitrary_int::{u6, u18};

#[derive(Debug, Copy, Clone)]
pub struct ClockConfig {
    pub frac: u6,
    pub int: u18,
}

#[derive(Debug, Copy, Clone)]
pub enum BaudMultiplier {
    _8 = 8,
    _16 = 16,
}

pub fn uart_clock_calc(ref_clk: u32, baudrate: u32, baud_mult: BaudMultiplier) -> ClockConfig {
    // This is the calculation: (64.0 * (x - integer_part as f32) + 0.5) as u32 without floating
    // point calculations.
    let multiplier = baud_mult as u32;
    let frac = ((ref_clk % (baudrate * multiplier)) * 64 + (baudrate * (multiplier / 2)))
        / (baudrate * multiplier);
    // Calculations here are derived from chapter 4.8.5 (p.79) of the datasheet.
    let integer_part = ref_clk / (baudrate * multiplier);
    ClockConfig {
        frac: u6::new(frac as u8),
        int: u18::new(integer_part),
    }
}

const SYS_CLK_50_MHZ: u32 = 50_000_000;

fn main() {
    println!("UART Clock Configuration App");
    let clock_config = uart_clock_calc(SYS_CLK_50_MHZ, 38400, BaudMultiplier::_16);
    println!(
        "For a reference clock of {} Hz and baud rate of {} bps with multiplier {}, the clock configuration is: {:?}",
        SYS_CLK_50_MHZ,
        38400,
        BaudMultiplier::_16 as u32,
        clock_config
    );
    let clock_config = uart_clock_calc(SYS_CLK_50_MHZ, 38400, BaudMultiplier::_8);
    println!(
        "For a reference clock of {} Hz and baud rate of {} bps with multiplier {}, the clock configuration is: {:?}",
        SYS_CLK_50_MHZ,
        38400,
        BaudMultiplier::_8 as u32,
        clock_config
    );
}
