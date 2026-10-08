/// Async I2C support.
pub mod asynch;
/// Register definitions for the I2C peripheral.
pub mod regs;

use crate::{
    PeripheralSelect, enable_peripheral_clock, i2c::regs::InterruptControl, sealed::Sealed,
    sysconfig::reset_peripheral_for_cycles, time::Hertz,
};
use arbitrary_int::{traits::Integer, u4, u10, u11, u20};
use core::marker::PhantomData;
use embedded_hal::i2c::{self, Operation, SevenBitAddress, TenBitAddress};
use regs::ClockTimeoutLimit;
pub use regs::{Bank, I2cSpeed};

#[cfg(feature = "vor1x")]
use va108xx as pac;
#[cfg(feature = "vor4x")]
use va416xx as pac;

//==================================================================================================
// Definitions
//==================================================================================================

/// Standard mode bus frequency.
pub const CLK_100K: Hertz = Hertz::from_raw(100_000);
/// Fast mode bus frequency.
pub const CLK_400K: Hertz = Hertz::from_raw(400_000);
/// Minimum system clock frequency required for fast mode.
pub const MIN_CLK_400K: Hertz = Hertz::from_raw(8_000_000);

/// Depth of the TX and RX FIFOs, in words.
pub const FIFO_DEPTH: usize = 16;

/// Maximum word count for one transfer supported by the hardware. Consecutive operations with
/// the same direction are merged into one transfer, so this limit applies to their sum.
pub const MAX_WORD_COUNT: usize = 0x7fe;

/// The configured clock is too slow to reach the requested I2C fast mode speed.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[error("clock too slow for fast I2C mode")]
pub struct ClockTooSlowForFastI2cError;

/// Invalid timing parameters were passed.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid timing parameters")]
pub struct InvalidTimingParamsError;

/// Error type for I2C transactions.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// Arbitration was lost.
    #[error("arbitration lost")]
    ArbitrationLost,
    /// Address not acknowledged.
    #[error("nack address")]
    NackAddr,
    /// Data not acknowledged in write operation
    #[error("data not acknowledged in write operation")]
    NackData,
    /// Not enough data received in read operation
    #[error("insufficient data received")]
    InsufficientDataReceived,
    /// Number of bytes in transfer too large (larger than 0x7fe)
    #[error("data too large (larger than 0x7fe)")]
    DataTooLarge,
    /// The I2C clock was seen low for longer than the configured timeout.
    #[error("clock timeout, SCL was low for {0} clock cycles")]
    ClockTimeout(u20),
    /// The RX or TX FIFO overflowed.
    #[error("FIFO overflow")]
    Overflow,
}

/// Error type for [I2cMaster::new].
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum InitError {
    /// Wrong address used in constructor
    #[error("wrong address mode")]
    WrongAddrMode,
    /// APB1 clock is too slow for fast I2C mode.
    #[error("clock too slow for fast I2C mode: {0}")]
    ClockTooSlow(#[from] ClockTooSlowForFastI2cError),
}

impl embedded_hal::i2c::Error for Error {
    fn kind(&self) -> embedded_hal::i2c::ErrorKind {
        match self {
            Error::ArbitrationLost => embedded_hal::i2c::ErrorKind::ArbitrationLoss,
            Error::NackAddr => {
                embedded_hal::i2c::ErrorKind::NoAcknowledge(i2c::NoAcknowledgeSource::Address)
            }
            Error::NackData => {
                embedded_hal::i2c::ErrorKind::NoAcknowledge(i2c::NoAcknowledgeSource::Data)
            }
            Error::DataTooLarge | Error::InsufficientDataReceived | Error::ClockTimeout(_) => {
                embedded_hal::i2c::ErrorKind::Other
            }
            Error::Overflow => embedded_hal::i2c::ErrorKind::Overrun,
        }
    }
}

/// Command written to the COMMAND register to control a transaction.
#[derive(Debug, PartialEq, Copy, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Command {
    /// Issue a START condition.
    Start,
    /// Issue a STOP condition.
    Stop,
    /// Issue a START condition followed by a STOP condition.
    StartWithStop,
    /// Cancel the current transaction.
    Cancel,
}

impl Command {
    /// COMMAND register value for this command.
    pub const fn reg_value(&self) -> regs::Command {
        match self {
            Command::Start => regs::Command::ZERO.with_start(true),
            Command::Stop => regs::Command::ZERO.with_stop(true),
            Command::StartWithStop => regs::Command::ZERO.with_start(true).with_stop(true),
            Command::Cancel => regs::Command::ZERO.with_cancel(true),
        }
    }
}

/// Slave address, either 7-bit or 10-bit.
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Address {
    /// 7-bit address.
    Regular(u8),
    /// 10-bit address.
    TenBit(u16),
}

impl Address {
    /// Whether this is a 10-bit address.
    pub fn ten_bit_addr(&self) -> bool {
        match self {
            Address::Regular(_) => false,
            Address::TenBit(_) => true,
        }
    }

    /// The raw address value.
    pub fn raw(&self) -> u16 {
        match self {
            Address::Regular(addr) => *addr as u16,
            Address::TenBit(addr) => *addr,
        }
    }
}

impl From<SevenBitAddress> for Address {
    fn from(addr: SevenBitAddress) -> Self {
        Address::Regular(addr)
    }
}

impl From<TenBitAddress> for Address {
    fn from(addr: TenBitAddress) -> Self {
        Address::TenBit(addr)
    }
}

/// Common trait implemented by all PAC peripheral access structures. The register block
/// format is the same for all I2C blocks.
pub trait I2cInstance: Sealed {
    /// I2C bank of the peripheral.
    const ID: Bank;
    /// Peripheral selector used for clock and reset control.
    const PERIPH_SEL: PeripheralSelect;
}

/// I2C0 peripheral instance.
#[cfg(feature = "vor1x")]
pub type I2c0 = pac::I2ca;
/// I2C0 peripheral instance.
#[cfg(feature = "vor4x")]
pub type I2c0 = pac::I2c0;

impl I2cInstance for I2c0 {
    const ID: Bank = Bank::I2c0;
    const PERIPH_SEL: PeripheralSelect = PeripheralSelect::I2c0;
}
impl Sealed for I2c0 {}

/// I2C1 peripheral instance.
#[cfg(feature = "vor1x")]
pub type I2c1 = pac::I2cb;
/// I2C1 peripheral instance.
#[cfg(feature = "vor4x")]
pub type I2c1 = pac::I2c1;

impl I2cInstance for I2c1 {
    const ID: Bank = Bank::I2c1;
    const PERIPH_SEL: PeripheralSelect = PeripheralSelect::I2c1;
}
impl Sealed for I2c1 {}

//==================================================================================================
// Config
//==================================================================================================

fn calc_clk_div_generic(
    ref_clk: Hertz,
    speed_mode: I2cSpeed,
) -> Result<u8, ClockTooSlowForFastI2cError> {
    if speed_mode == I2cSpeed::Regular100khz {
        Ok(((ref_clk.to_raw() / CLK_100K.to_raw() / 20) - 1) as u8)
    } else {
        if ref_clk.to_raw() < MIN_CLK_400K.to_raw() {
            return Err(ClockTooSlowForFastI2cError);
        }
        Ok(((ref_clk.to_raw() / CLK_400K.to_raw() / 25) - 1) as u8)
    }
}

#[cfg(feature = "vor4x")]
fn calc_clk_div(
    clks: &crate::clock::Clocks,
    speed_mode: I2cSpeed,
) -> Result<u8, ClockTooSlowForFastI2cError> {
    calc_clk_div_generic(clks.apb1(), speed_mode)
}

#[cfg(feature = "vor1x")]
fn calc_clk_div(sys_clk: Hertz, speed_mode: I2cSpeed) -> Result<u8, ClockTooSlowForFastI2cError> {
    calc_clk_div_generic(sys_clk, speed_mode)
}

/// Manual override for the I2C bus timing values normally derived from the clock scale.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TimingConfig {
    /// Rise time.
    pub t_rise: u4,
    /// Fall time.
    pub t_fall: u4,
    /// Duty cycle high time of SCL.
    pub t_high: u4,
    /// Duty cycle low time of SCL.
    pub t_low: u4,
    /// Setup time for STOP.
    pub tsu_stop: u4,
    /// Setup time for START.
    pub tsu_start: u4,
    /// Data hold time.
    pub thd_start: u4,
    /// Bus free time between STOP and START.
    pub t_buf: u4,
}

/// The default configuration uses the register reset values.
impl Default for TimingConfig {
    fn default() -> Self {
        TimingConfig {
            t_rise: u4::new(0b0010),
            t_fall: u4::new(0b0001),
            t_high: u4::new(0b1000),
            t_low: u4::new(0b1001),
            tsu_stop: u4::new(0b1000),
            tsu_start: u4::new(0b1010),
            thd_start: u4::new(0b1000),
            t_buf: u4::new(0b1010),
        }
    }
}

/// Configuration for [I2cMaster::new].
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MasterConfig {
    /// Enable the analog delay glitch filter
    pub alg_filt: bool,
    /// Enable the digital glitch filter
    pub dlg_filt: bool,
    /// Manual override for the bus timing values.
    pub timing_config: Option<TimingConfig>,
    /// See [I2cMaster::set_clock_low_timeout] documentation.
    pub timeout: Option<u20>,
    // Loopback mode
    // lbm: bool,
}

impl Default for MasterConfig {
    fn default() -> Self {
        MasterConfig {
            alg_filt: false,
            dlg_filt: false,
            timeout: Some(u20::MAX),
            timing_config: None,
        }
    }
}

impl Sealed for MasterConfig {}

#[derive(Debug, PartialEq, Eq)]
enum CompletionCondition {
    Idle,
    Waiting,
}

impl CompletionCondition {
    fn for_command(cmd: Command) -> Self {
        match cmd {
            // Without a stop, the controller holds the bus and waits for the next command.
            Command::Start => Self::Waiting,
            Command::Stop | Command::StartWithStop | Command::Cancel => Self::Idle,
        }
    }

    fn is_met(&self, status: regs::Status) -> bool {
        match self {
            Self::Idle => status.idle(),
            Self::Waiting => status.waiting(),
        }
    }
}

struct TimeoutGuard {
    clk_timeout_enabled: bool,
    regs: regs::MmioRegisters<'static>,
}

impl TimeoutGuard {
    fn new(regs: &regs::MmioRegisters<'static>) -> Self {
        let clk_timeout_enabled = regs.read_clk_timeout_limit().value().value() > 0;
        let mut guard = TimeoutGuard {
            clk_timeout_enabled,
            regs: unsafe { regs.clone() },
        };
        if clk_timeout_enabled {
            // Clear any interrupts which might be pending.
            guard
                .regs
                .write_interrupt_clear(regs::InterruptClear::DEFAULT.with_clock_timeout(true));
            guard.regs.modify_interrupt_enable(|mut value| {
                value.set_clock_timeout(true);
                value
            });
        }
        guard
    }

    fn timeout_enabled(&self) -> bool {
        self.clk_timeout_enabled
    }
}

impl Drop for TimeoutGuard {
    fn drop(&mut self) {
        if self.clk_timeout_enabled {
            self.regs.modify_interrupt_enable(|mut value| {
                value.set_clock_timeout(false);
                value
            });
        }
    }
}
//==================================================================================================
// I2C Master
//==================================================================================================

/// I2C master driver.
pub struct I2cMaster<Addr = SevenBitAddress> {
    id: Bank,
    regs: regs::MmioRegisters<'static>,
    addr: PhantomData<Addr>,
}

impl<Addr> I2cMaster<Addr> {
    /// Create a new I2C master driver, taking ownership of the given peripheral instance.
    pub fn new<I2c: I2cInstance>(
        _i2c: I2c,
        #[cfg(feature = "vor1x")] sysclk: Hertz,
        #[cfg(feature = "vor4x")] clks: &crate::clock::Clocks,
        cfg: MasterConfig,
        speed_mode: I2cSpeed,
    ) -> Result<Self, ClockTooSlowForFastI2cError> {
        reset_peripheral_for_cycles(I2c::PERIPH_SEL, 2);
        enable_peripheral_clock(I2c::PERIPH_SEL);
        let mut regs = regs::Registers::new_mmio(I2c::ID);
        #[cfg(feature = "vor1x")]
        let clk_div = calc_clk_div(sysclk, speed_mode)?;
        #[cfg(feature = "vor4x")]
        let clk_div = calc_clk_div(clks, speed_mode)?;
        regs.write_clkscale(
            regs::ClockScale::builder()
                .with_div(clk_div)
                .with_fastmode(speed_mode)
                .build(),
        );
        regs.modify_control(|mut value| {
            // Stalling is required to merge consecutive operations of the same direction into
            // one transfer, as the embedded-hal transaction contract demands.
            value.set_tx_fifo_empty_mode(regs::TxFifoEmptyMode::Stall);
            value.set_rx_fifo_full_mode(regs::RxFifoFullMode::Stall);
            value.set_analog_filter(cfg.alg_filt);
            value.set_digital_filter(cfg.dlg_filt);
            value
        });

        if let Some(ref timing_cfg) = cfg.timing_config {
            regs.modify_control(|mut value| {
                value.set_enable_timing_config(true);
                value
            });
            regs.write_timing_config(
                regs::TimingConfig::builder()
                    .with_t_rise(timing_cfg.t_rise)
                    .with_t_fall(timing_cfg.t_fall)
                    .with_t_high(timing_cfg.t_high)
                    .with_t_low(timing_cfg.t_low)
                    .with_tsu_stop(timing_cfg.tsu_stop)
                    .with_tsu_start(timing_cfg.tsu_start)
                    .with_thd_start(timing_cfg.thd_start)
                    .with_t_buf(timing_cfg.t_buf)
                    .build(),
            );
        }
        regs.write_fifo_clear(
            regs::FifoClear::builder()
                .with_tx_fifo(true)
                .with_rx_fifo(true)
                .build(),
        );
        if let Some(timeout) = cfg.timeout {
            regs.write_clk_timeout_limit(ClockTimeoutLimit::new(timeout));
        }
        let mut i2c_master = I2cMaster {
            addr: PhantomData,
            id: I2c::ID,
            regs,
        };
        i2c_master.enable();
        Ok(i2c_master)
    }

    /// I2C bank of this instance.
    pub const fn id(&self) -> Bank {
        self.id
    }

    /// Read the peripheral ID register.
    #[inline]
    pub fn perid(&self) -> u32 {
        self.regs.read_perid()
    }

    /// Configures the clock scale for a given speed mode setting
    pub fn set_clk_scale(
        &mut self,
        #[cfg(feature = "vor1x")] sys_clk: Hertz,
        #[cfg(feature = "vor4x")] clks: &crate::clock::Clocks,
        speed_mode: I2cSpeed,
    ) -> Result<(), ClockTooSlowForFastI2cError> {
        self.disable();
        #[cfg(feature = "vor1x")]
        let clk_div = calc_clk_div(sys_clk, speed_mode)?;
        #[cfg(feature = "vor4x")]
        let clk_div = calc_clk_div(clks, speed_mode)?;
        self.regs.write_clkscale(
            regs::ClockScale::builder()
                .with_div(clk_div)
                .with_fastmode(speed_mode)
                .build(),
        );
        self.enable();
        Ok(())
    }

    /// Cancel the currently ongoing transaction.
    #[inline]
    pub fn cancel_transfer(&mut self) {
        self.write_command(Command::Cancel);
    }

    /// Disable the interrupts.
    #[inline]
    pub fn disable_interrupts(&mut self) {
        self.regs
            .write_interrupt_enable(InterruptControl::new_with_raw_value(0));
    }

    /// Clear the TX FIFO.
    #[inline]
    pub fn clear_tx_fifo(&mut self) {
        self.regs.write_fifo_clear(
            regs::FifoClear::builder()
                .with_tx_fifo(true)
                .with_rx_fifo(false)
                .build(),
        );
    }

    /// Clear the RX FIFO.
    #[inline]
    pub fn clear_rx_fifo(&mut self) {
        self.regs.write_fifo_clear(
            regs::FifoClear::builder()
                .with_tx_fifo(false)
                .with_rx_fifo(true)
                .build(),
        );
    }

    /// Configure a timeout limit on the amount of time the I2C clock is seen to be low.
    /// The timeout is specified as I2C clock cycles.
    ///
    /// If the timeout is enabled, the blocking transaction handlers provided by the [I2cMaster]
    /// will poll the interrupt status register to check for timeouts. This can be used to avoid
    /// hang-ups of the I2C bus.
    #[inline]
    pub fn set_clock_low_timeout(&mut self, clock_cycles: u20) {
        self.regs
            .write_clk_timeout_limit(ClockTimeoutLimit::new(clock_cycles));
    }

    /// Disable the clock low timeout.
    #[inline]
    pub fn disable_clock_low_timeout(&mut self) {
        self.regs
            .write_clk_timeout_limit(ClockTimeoutLimit::new(u20::new(0)));
    }

    /// Enable the peripheral.
    #[inline]
    pub fn enable(&mut self) {
        self.regs.modify_control(|mut value| {
            value.set_enable(true);
            value
        });
    }

    /// Disable the peripheral.
    #[inline]
    pub fn disable(&mut self) {
        self.regs.modify_control(|mut value| {
            value.set_enable(false);
            value
        });
    }

    #[inline(always)]
    fn write_fifo_unchecked(&mut self, word: u8) {
        self.regs.write_data(regs::Data::new(word));
    }

    #[inline(always)]
    fn read_fifo_unchecked(&self) -> u8 {
        self.regs.read_data().data()
    }

    /// Read the STATUS register.
    #[inline]
    pub fn read_status(&self) -> regs::Status {
        self.regs.read_status()
    }

    /// Write a command to the COMMAND register.
    #[inline]
    pub fn write_command(&mut self, cmd: Command) {
        self.regs.write_command(cmd.reg_value());
    }

    /// Write the target address and transfer direction to the ADDRESS register.
    #[inline]
    pub fn write_address_and_direction(&mut self, addr: Address, dir: regs::Direction) {
        self.regs.write_address(
            regs::Address::builder()
                .with_direction(dir)
                .with_address(u10::new(addr.raw()))
                .with_a10_mode(addr.ten_bit_addr())
                .build(),
        );
    }

    /// A group started with [Command::Start] leaves the controller holding the bus after an
    /// error, so it has to be released with an explicit stop. For [Command::StartWithStop],
    /// the hardware already takes care of it.
    fn release_bus_on_error(&mut self, group_command: Command) {
        if group_command == Command::Start {
            self.write_command(Command::Stop);
        }
    }

    /// Every I2C transfer goes through this function which makes it easier to meet all
    /// I2C HAL trait contracts.
    ///
    /// Empty reads are ignored. Empty writes address the target without data, which can be
    /// used to probe for a device.
    pub fn transaction(&mut self, addr: Addr, operations: &mut [Operation<'_>]) -> Result<(), Error>
    where
        Addr: Into<Address>,
    {
        let addr = addr.into();
        if operations.is_empty() {
            return Ok(());
        }
        // Checked up front so an invalid transaction does not leave the bus held by an
        // earlier group.
        check_group_lengths(operations)?;
        let ops_len = operations.len();
        self.clear_tx_fifo();
        self.clear_rx_fifo();
        let timeout_guard = TimeoutGuard::new(&self.regs);

        // Empty reads are skipped entirely. The hardware can not do a zero-length read, and
        // skipping them in all lookups below ensures they never split or end a merged transfer.
        let mut prev_dir = None;
        let mut group_command = Command::Start;
        for i in 0..ops_len {
            if is_empty_read(&operations[i]) {
                continue;
            }
            // Consecutive operations with the same direction are merged into one group, which
            // is a single hardware transfer. The command is only written for the first operation
            // of a group:
            //  - ST = Start condition
            //  - SR = Repeated start
            //  - SP = Stop condition
            //
            // [W] -> ST + SP on W
            // [W, R] -> ST on W, SR + SP on R
            // [W, W] -> ST + SP on first W, the group covers both
            // [W, R, W] -> ST on first W, SR on R, SR + SP on last W
            //
            // The last group uses StartWithStop, so the hardware issues the stop condition once
            // the word count is done. All other groups use Start, which leaves the controller
            // waiting for the repeated start of the next group.
            let dir = op_direction(&operations[i]);
            let next_dir = operations[i + 1..]
                .iter()
                .find(|op| !is_empty_read(op))
                .map(op_direction);

            // Either the first operation or a direction change.
            let first_op_in_group = prev_dir != Some(dir);
            // Only used for the first operation of a group. The look ahead below adds the lengths
            // of the other operations in the group.
            let mut group_len = op_len(&operations[i]);
            let more_ops_in_group = next_dir == Some(dir);

            if first_op_in_group {
                // Assume this is the last group until the look ahead finds a direction change.
                group_command = Command::StartWithStop;
                for next_op in operations[i + 1..].iter().filter(|op| !is_empty_read(op)) {
                    if op_direction(next_op) != dir {
                        group_command = Command::Start;
                        break;
                    }

                    group_len += op_len(next_op);
                }
                // A zero-length write group is still sent. It addresses the target without data,
                // which is a valid way to probe for a device.
                self.regs
                    .write_words(regs::Words::new(u11::new(group_len as u16)));
                self.write_address_and_direction(addr, dir);
            }
            match &mut operations[i] {
                Operation::Read(items) => self.read_internal(
                    items,
                    group_command,
                    first_op_in_group,
                    timeout_guard.timeout_enabled(),
                    more_ops_in_group,
                )?,
                Operation::Write(items) => self.write_internal(
                    items,
                    group_command,
                    first_op_in_group,
                    timeout_guard.timeout_enabled(),
                    more_ops_in_group,
                )?,
            }
            prev_dir = Some(dir);
        }
        Ok(())
    }

    fn write_internal(
        &mut self,
        output: &[u8],
        group_command: Command,
        first_op_of_group: bool,
        timeout_enabled: bool,
        more_ops_in_group: bool,
    ) -> Result<(), Error> {
        let mut bytes = output.iter().copied().peekable();
        // Pre-fill the FIFO so the controller has data as soon as the command starts the transfer.
        while self.read_status().tx_not_full()
            && let Some(byte) = bytes.next()
        {
            self.write_fifo_unchecked(byte);
        }
        if first_op_of_group {
            self.write_command(group_command);
        }
        let completion = CompletionCondition::for_command(group_command);
        loop {
            let status = self.regs.read_status();
            if status.arb_lost() {
                self.release_bus_on_error(group_command);
                self.clear_tx_fifo();
                return Err(Error::ArbitrationLost);
            }
            if status.nack_addr() {
                self.release_bus_on_error(group_command);
                self.clear_tx_fifo();
                return Err(Error::NackAddr);
            }
            if status.nack_data() {
                self.release_bus_on_error(group_command);
                self.clear_tx_fifo();
                return Err(Error::NackData);
            }
            if completion.is_met(status) {
                return Ok(());
            }
            if timeout_enabled && self.regs.read_interrupt_status().clock_timeout() {
                self.cancel_transfer();
                self.clear_tx_fifo();
                return Err(Error::ClockTimeout(
                    self.regs.read_clk_timeout_limit().value(),
                ));
            }
            if status.tx_not_full()
                && let Some(byte) = bytes.next()
            {
                self.write_fifo_unchecked(byte);
            }
            if more_ops_in_group && bytes.peek().is_none() {
                return Ok(());
            }
        }
    }

    fn read_internal(
        &mut self,
        buffer: &mut [u8],
        group_command: Command,
        first_op_in_group: bool,
        timeout_enabled: bool,
        more_ops_in_group: bool,
    ) -> Result<(), Error> {
        let full_len = buffer.len();
        let mut byte_index = 0;
        let completion = CompletionCondition::for_command(group_command);
        if first_op_in_group {
            self.write_command(group_command);
        }
        loop {
            let status = self.read_status();

            if status.arb_lost() {
                self.release_bus_on_error(group_command);
                self.clear_rx_fifo();
                return Err(Error::ArbitrationLost);
            }
            if status.nack_addr() {
                self.release_bus_on_error(group_command);
                self.clear_rx_fifo();
                return Err(Error::NackAddr);
            }
            if timeout_enabled && self.regs.read_interrupt_status().clock_timeout() {
                self.cancel_transfer();
                self.clear_rx_fifo();
                return Err(Error::ClockTimeout(
                    self.regs.read_clk_timeout_limit().value(),
                ));
            }
            let drain_rx_fifo = |buffer: &mut [u8], byte_index: &mut usize| {
                while self.read_status().rx_not_empty() && *byte_index < full_len {
                    let byte = self.read_fifo_unchecked();
                    buffer[*byte_index] = byte;
                    *byte_index += 1;
                }
            };

            drain_rx_fifo(buffer, &mut byte_index);
            if byte_index == full_len && more_ops_in_group {
                return Ok(());
            }

            if completion.is_met(status) {
                // The controller goes idle once the last byte is on the wire, but earlier bytes
                // can still sit in the FIFO if this loop was preempted.
                drain_rx_fifo(buffer, &mut byte_index);
                if byte_index != full_len {
                    return Err(Error::InsufficientDataReceived);
                }
                return Ok(());
            }
        }
    }
}

fn op_direction(op: &Operation<'_>) -> regs::Direction {
    match op {
        Operation::Read(_) => regs::Direction::Receive,
        Operation::Write(_) => regs::Direction::Send,
    }
}

fn op_len(op: &Operation<'_>) -> usize {
    match op {
        Operation::Read(items) => items.len(),
        Operation::Write(items) => items.len(),
    }
}

fn is_empty_read(op: &Operation<'_>) -> bool {
    matches!(op, Operation::Read(buf) if buf.is_empty())
}

/// Index of the next operation at or after `from`, skipping empty reads.
fn next_op(ops: &[Operation<'_>], from: usize) -> Option<usize> {
    (from..ops.len()).find(|&i| !is_empty_read(&ops[i]))
}

/// Index of the first operation of the group after the one containing `idx`, skipping empty
/// reads.
fn next_group(ops: &[Operation<'_>], idx: usize) -> Option<usize> {
    let dir = op_direction(&ops[idx]);
    (idx + 1..ops.len()).find(|&i| !is_empty_read(&ops[i]) && op_direction(&ops[i]) != dir)
}

fn check_group_lengths(operations: &[Operation<'_>]) -> Result<(), Error> {
    let mut prev_dir = None;
    let mut group_len = 0;
    for op in operations.iter().filter(|op| !is_empty_read(op)) {
        let dir = op_direction(op);
        if prev_dir != Some(dir) {
            group_len = 0;
        }
        group_len += op_len(op);
        if group_len > MAX_WORD_COUNT {
            return Err(Error::DataTooLarge);
        }
        prev_dir = Some(dir);
    }
    Ok(())
}

impl I2cMaster<SevenBitAddress> {
    /// Convert this blocking driver into an asynchronous one.
    ///
    /// See [asynch::I2c::new] for more details.
    #[cfg(feature = "vor1x")]
    pub fn into_async(self, opt_irq_cfg: Option<crate::InterruptConfig>) -> asynch::I2c {
        asynch::I2c::new(self, opt_irq_cfg)
    }

    /// Convert this blocking driver into an asynchronous one.
    ///
    /// See [asynch::I2c::new] for more details.
    #[cfg(feature = "vor4x")]
    pub fn into_async(self) -> asynch::I2c {
        asynch::I2c::new(self)
    }
}

/// Inherent versions of the [embedded_hal::i2c::I2c] methods, so they can be used without
/// importing the trait.
impl<Addr: i2c::AddressMode> I2cMaster<Addr>
where
    Self: i2c::I2c<Addr, Error = Error>,
{
    /// Blocking write transaction on the I2C bus.
    pub fn write(&mut self, addr: Addr, output: &[u8]) -> Result<(), Error> {
        <Self as i2c::I2c<Addr>>::write(self, addr, output)
    }

    /// Blocking read transaction on the I2C bus.
    pub fn read(&mut self, addr: Addr, buf: &mut [u8]) -> Result<(), Error> {
        <Self as i2c::I2c<Addr>>::read(self, addr, buf)
    }

    /// Blocking write-read transaction on the I2C bus.
    pub fn write_read(&mut self, addr: Addr, write: &[u8], read: &mut [u8]) -> Result<(), Error> {
        <Self as i2c::I2c<Addr>>::write_read(self, addr, write, read)
    }
}

//======================================================================================
// Embedded HAL I2C implementations
//======================================================================================

impl embedded_hal::i2c::ErrorType for I2cMaster<SevenBitAddress> {
    type Error = Error;
}

impl embedded_hal::i2c::I2c for I2cMaster<SevenBitAddress> {
    fn transaction(
        &mut self,
        address: SevenBitAddress,
        operations: &mut [Operation<'_>],
    ) -> Result<(), Self::Error> {
        self.transaction(address, operations)
    }
}

impl embedded_hal::i2c::ErrorType for I2cMaster<TenBitAddress> {
    type Error = Error;
}

impl embedded_hal::i2c::I2c<TenBitAddress> for I2cMaster<TenBitAddress> {
    fn transaction(
        &mut self,
        address: TenBitAddress,
        operations: &mut [Operation<'_>],
    ) -> Result<(), Self::Error> {
        self.transaction(address, operations)
    }
}
