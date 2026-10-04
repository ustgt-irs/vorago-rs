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
pub use regs::{Bank, I2cSpeed, RxFifoFullMode, TxFifoEmptyMode};

#[cfg(feature = "vor1x")]
use va108xx as pac;
#[cfg(feature = "vor4x")]
use va416xx as pac;

//==================================================================================================
// Defintions
//==================================================================================================

/// Standard mode bus frequency.
pub const CLK_100K: Hertz = Hertz::from_raw(100_000);
/// Fast mode bus frequency.
pub const CLK_400K: Hertz = Hertz::from_raw(400_000);
/// Minimum system clock frequency required for fast mode.
pub const MIN_CLK_400K: Hertz = Hertz::from_raw(8_000_000);

/// Depth of the TX and RX FIFOs, in words.
pub const FIFO_DEPTH: usize = 16;

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
pub enum I2cCommand {
    /// Issue a START condition.
    Start = 0b01,
    /// Issue a STOP condition.
    Stop = 0b10,
    /// Issue a START condition followed by a STOP condition.
    StartWithStop = 0b11,
    /// Cancel the current transaction.
    Cancel = 0b100,
}

/// Slave address, either 7-bit or 10-bit.
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum I2cAddress {
    /// 7-bit address.
    Regular(u8),
    /// 10-bit address.
    TenBit(u16),
}

impl I2cAddress {
    /// Whether this is a 10-bit address.
    pub fn ten_bit_addr(&self) -> bool {
        match self {
            I2cAddress::Regular(_) => false,
            I2cAddress::TenBit(_) => true,
        }
    }

    /// The raw address value.
    pub fn raw(&self) -> u16 {
        match self {
            I2cAddress::Regular(addr) => *addr as u16,
            I2cAddress::TenBit(addr) => *addr,
        }
    }
}

/// Common trait implemented by all PAC peripheral access structures. The register block
/// format is the same for all SPI blocks.
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

/// Default configuration are the register reset value which are used by default.
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
    /// Behavior when the TX FIFO is empty.
    pub tx_empty_mode: TxFifoEmptyMode,
    /// Behavior when the RX FIFO is full.
    pub rx_full_mode: RxFifoFullMode,
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
            tx_empty_mode: TxFifoEmptyMode::Stall,
            rx_full_mode: RxFifoFullMode::Stall,
            alg_filt: false,
            dlg_filt: false,
            timeout: Some(u20::MAX),
            timing_config: None,
        }
    }
}

impl Sealed for MasterConfig {}

#[derive(Debug, PartialEq, Eq)]
enum WriteCompletionCondition {
    Idle,
    Waiting,
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
            guard.regs.write_interrupt_clear(
                regs::InterruptClear::builder()
                    .with_clock_timeout(true)
                    .with_tx_overflow(false)
                    .with_rx_overflow(false)
                    .build(),
            );
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

fn operation_run(operations: &[Operation<'_>], start: usize) -> Result<(usize, usize), Error> {
    let Some(first) = operations.get(start) else {
        return Ok((start, 0));
    };

    let is_read = matches!(first, Operation::Read(_));
    let mut end = start;
    let mut total_len = 0usize;

    while let Some(operation) = operations.get(end) {
        let same_direction = matches!(
            (operation, is_read),
            (Operation::Read(_), true) | (Operation::Write(_), false)
        );

        if !same_direction {
            break;
        }

        let len = match operation {
            Operation::Read(buf) => buf.len(),
            Operation::Write(buf) => buf.len(),
        };

        total_len = total_len.checked_add(len).ok_or(Error::DataTooLarge)?;

        if total_len > 0x7fe {
            return Err(Error::DataTooLarge);
        }

        end += 1;
    }

    Ok((end, total_len))
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
            value.set_tx_fifo_empty_mode(cfg.tx_empty_mode);
            value.set_rx_fifo_full_mode(cfg.rx_full_mode);
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
        self.regs.write_command(
            regs::Command::builder()
                .with_start(false)
                .with_stop(false)
                .with_cancel(true)
                .build(),
        );
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
    pub fn read_status(&mut self) -> regs::Status {
        self.regs.read_status()
    }

    /// Write a command to the COMMAND register.
    #[inline]
    pub fn write_command(&mut self, cmd: I2cCommand) {
        self.regs
            .write_command(regs::Command::new_with_raw_value(cmd as u32));
    }

    /// Write the target address and transfer direction to the ADDRESS register.
    #[inline]
    pub fn write_address(&mut self, addr: I2cAddress, dir: regs::Direction) {
        self.regs.write_address(
            regs::Address::builder()
                .with_direction(dir)
                .with_address(u10::new(addr.raw()))
                .with_a10_mode(addr.ten_bit_addr())
                .build(),
        );
    }

    fn error_handler_write(&mut self, init_cmd: I2cCommand) {
        if init_cmd == I2cCommand::Start {
            self.write_command(I2cCommand::Stop);
        }
        // The other case is start with stop where, so a CANCEL command should not be necessary
        // because the hardware takes care of it.
        self.clear_tx_fifo();
    }

    /// Blocking write transaction on the I2C bus.
    pub fn write_blocking(&mut self, addr: I2cAddress, output: &[u8]) -> Result<(), Error> {
        self.write_blocking_generic(
            I2cCommand::StartWithStop,
            addr,
            output,
            WriteCompletionCondition::Idle,
        )
    }

    /// Blocking read transaction on the I2C bus.
    pub fn read_blocking(&mut self, addr: I2cAddress, buffer: &mut [u8]) -> Result<(), Error> {
        let len = buffer.len();
        if len > 0x7fe {
            return Err(Error::DataTooLarge);
        }
        // Clear the receive FIFO
        self.clear_rx_fifo();

        let timeout_guard = TimeoutGuard::new(&self.regs);

        // Load number of words
        self.regs
            .write_words(regs::Words::new(u11::new(len as u16)));
        // Load address
        self.write_address(addr, regs::Direction::Receive);

        let mut buf_iter = buffer.iter_mut();
        let mut read_bytes = 0;
        // Start receive transfer
        self.write_command(I2cCommand::StartWithStop);
        loop {
            let status = self.read_status();
            if status.arb_lost() {
                self.clear_rx_fifo();
                return Err(Error::ArbitrationLost);
            }
            if status.nack_addr() {
                self.clear_rx_fifo();
                return Err(Error::NackAddr);
            }
            if status.idle() {
                // The controller goes idle once the last byte is on the wire, but earlier bytes
                // can still sit in the FIFO if this loop was preempted.
                while self.read_status().rx_not_empty() {
                    self.read_next_byte(&mut buf_iter, &mut read_bytes);
                }
                if read_bytes != len {
                    return Err(Error::InsufficientDataReceived);
                }
                return Ok(());
            }
            if timeout_guard.timeout_enabled() && self.regs.read_interrupt_status().clock_timeout()
            {
                self.clear_rx_fifo();
                return Err(Error::ClockTimeout(
                    self.regs.read_clk_timeout_limit().value(),
                ));
            }
            if status.rx_not_empty() {
                self.read_next_byte(&mut buf_iter, &mut read_bytes);
            }
        }
    }

    #[inline(always)]
    fn read_next_byte<'a>(
        &self,
        buf_iter: &mut impl Iterator<Item = &'a mut u8>,
        read_bytes: &mut usize,
    ) {
        let byte = self.read_fifo_unchecked();
        if let Some(next_byte) = buf_iter.next() {
            *next_byte = byte;
        }
        *read_bytes += 1;
    }

    fn write_blocking_generic(
        &mut self,
        init_cmd: I2cCommand,
        addr: I2cAddress,
        output: &[u8],
        end_condition: WriteCompletionCondition,
    ) -> Result<(), Error> {
        let len = output.len();
        if len > 0x7fe {
            return Err(Error::DataTooLarge);
        }
        // Clear the send FIFO
        self.clear_tx_fifo();

        let timeout_guard = TimeoutGuard::new(&self.regs);

        // Load number of words
        self.regs
            .write_words(regs::Words::new(u11::new(len as u16)));
        let mut bytes = output.iter();
        // FIFO has a depth of 16. We load slightly above the trigger level
        // but not all of it because the transaction might fail immediately
        const FILL_DEPTH: usize = 12;

        let mut current_index = core::cmp::min(FILL_DEPTH, len);
        // load the FIFO
        for _ in 0..current_index {
            self.write_fifo_unchecked(*bytes.next().unwrap());
        }
        self.write_address(addr, regs::Direction::Send);
        self.write_command(init_cmd);
        loop {
            let status = self.regs.read_status();
            if status.arb_lost() {
                self.error_handler_write(init_cmd);
                return Err(Error::ArbitrationLost);
            }
            if status.nack_addr() {
                self.error_handler_write(init_cmd);
                return Err(Error::NackAddr);
            }
            if status.nack_data() {
                self.error_handler_write(init_cmd);
                return Err(Error::NackData);
            }
            match end_condition {
                WriteCompletionCondition::Idle => {
                    if status.idle() {
                        return Ok(());
                    }
                }

                WriteCompletionCondition::Waiting => {
                    if status.waiting() {
                        return Ok(());
                    }
                }
            }
            if timeout_guard.timeout_enabled() && self.regs.read_interrupt_status().clock_timeout()
            {
                return Err(Error::ClockTimeout(
                    self.regs.read_clk_timeout_limit().value(),
                ));
            }
            if status.tx_not_full() && current_index < len {
                self.write_fifo_unchecked(output[current_index]);
                current_index += 1;
            }
        }
    }

    /// Execute one `embedded-hal` transaction while preserving transaction
    /// boundaries across adjacent operations.
    ///
    /// Adjacent operations with the same direction are emitted as one hardware
    /// phase. A direction change issues a repeated START, and only the final
    /// phase emits STOP.
    fn transaction_blocking(
        &mut self,
        address: I2cAddress,
        operations: &mut [Operation<'_>],
    ) -> Result<(), Error> {
        let mut start = 0;

        while start < operations.len() {
            let is_read = matches!(&operations[start], Operation::Read(_));

            let (end, total_len) = operation_run(operations, start)?;

            // Empty operations carry no data and do not require a hardware phase.
            if total_len == 0 {
                start = end;
                continue;
            }

            let is_last = end == operations.len();

            if is_read {
                self.read_operation_run(address, &mut operations[start..end], total_len, is_last)?;
            } else {
                self.write_operation_run(address, &operations[start..end], total_len, is_last)?;
            }

            start = end;
        }

        Ok(())
    }

    /// Execute a contiguous run of write operations as a single I2C phase.
    fn write_operation_run(
        &mut self,
        address: I2cAddress,
        operations: &[Operation<'_>],
        total_len: usize,
        is_last: bool,
    ) -> Result<(), Error> {
        if total_len > 0x7fe {
            return Err(Error::DataTooLarge);
        }

        self.clear_tx_fifo();

        let timeout_guard = TimeoutGuard::new(&self.regs);

        self.regs
            .write_words(regs::Words::new(u11::new(total_len as u16)));

        let mut operation_index = 0usize;
        let mut byte_index = 0usize;

        let mut next_byte = || -> Option<u8> {
            loop {
                if operation_index >= operations.len() {
                    return None;
                }

                match &operations[operation_index] {
                    Operation::Write(buf) => {
                        if byte_index < buf.len() {
                            let byte = buf[byte_index];
                            byte_index += 1;
                            return Some(byte);
                        }

                        operation_index += 1;
                        byte_index = 0;
                    }
                    Operation::Read(_) => {
                        unreachable!("write_operation_run received a read operation")
                    }
                }
            }
        };

        const FILL_DEPTH: usize = 12;
        let initial_fill = core::cmp::min(FILL_DEPTH, total_len);

        for _ in 0..initial_fill {
            let byte = next_byte().expect("write operation run length must match buffers");
            self.write_fifo_unchecked(byte);
        }

        let mut written = initial_fill;

        self.write_address(address, regs::Direction::Send);

        let init_cmd = if is_last {
            I2cCommand::StartWithStop
        } else {
            I2cCommand::Start
        };

        self.write_command(init_cmd);

        loop {
            let status = self.regs.read_status();

            if status.arb_lost() {
                self.error_handler_write(init_cmd);
                return Err(Error::ArbitrationLost);
            }

            if status.nack_addr() {
                self.error_handler_write(init_cmd);
                return Err(Error::NackAddr);
            }

            if status.nack_data() {
                self.error_handler_write(init_cmd);
                return Err(Error::NackData);
            }

            if is_last {
                if status.idle() {
                    return Ok(());
                }
            } else if status.waiting() {
                return Ok(());
            }

            if timeout_guard.timeout_enabled() && self.regs.read_interrupt_status().clock_timeout()
            {
                self.error_handler_write(init_cmd);
                return Err(Error::ClockTimeout(
                    self.regs.read_clk_timeout_limit().value(),
                ));
            }

            if status.tx_not_full() && written < total_len {
                let byte = next_byte().expect("write operation run length must match buffers");
                self.write_fifo_unchecked(byte);
                written += 1;
            }
        }
    }

    /// Store one received byte in a contiguous run of read operations.
    fn store_transaction_read_byte(
        operations: &mut [Operation<'_>],
        operation_index: &mut usize,
        byte_index: &mut usize,
        byte: u8,
    ) {
        loop {
            match &mut operations[*operation_index] {
                Operation::Read(buf) => {
                    if *byte_index < buf.len() {
                        buf[*byte_index] = byte;
                        *byte_index += 1;
                        return;
                    }

                    *operation_index += 1;
                    *byte_index = 0;
                }
                Operation::Write(_) => {
                    unreachable!("read_operation_run received a write operation")
                }
            }
        }
    }

    /// Execute a contiguous run of read operations as a single I2C phase.
    fn read_operation_run(
        &mut self,
        address: I2cAddress,
        operations: &mut [Operation<'_>],
        total_len: usize,
        is_last: bool,
    ) -> Result<(), Error> {
        if total_len > 0x7fe {
            return Err(Error::DataTooLarge);
        }

        self.clear_rx_fifo();

        let timeout_guard = TimeoutGuard::new(&self.regs);

        self.regs
            .write_words(regs::Words::new(u11::new(total_len as u16)));

        self.write_address(address, regs::Direction::Receive);

        let init_cmd = if is_last {
            I2cCommand::StartWithStop
        } else {
            I2cCommand::Start
        };

        self.write_command(init_cmd);

        let mut operation_index = 0usize;
        let mut byte_index = 0usize;
        let mut read_bytes = 0usize;

        loop {
            let status = self.read_status();

            if status.arb_lost() {
                if !is_last {
                    self.write_command(I2cCommand::Stop);
                }
                self.clear_rx_fifo();
                return Err(Error::ArbitrationLost);
            }

            if status.nack_addr() {
                if !is_last {
                    self.write_command(I2cCommand::Stop);
                }
                self.clear_rx_fifo();
                return Err(Error::NackAddr);
            }

            let complete = if is_last {
                status.idle()
            } else {
                status.waiting()
            };

            if complete {
                while self.read_status().rx_not_empty() && read_bytes < total_len {
                    let byte = self.read_fifo_unchecked();

                    Self::store_transaction_read_byte(
                        operations,
                        &mut operation_index,
                        &mut byte_index,
                        byte,
                    );

                    read_bytes += 1;
                }

                if read_bytes != total_len {
                    return Err(Error::InsufficientDataReceived);
                }

                return Ok(());
            }

            if timeout_guard.timeout_enabled() && self.regs.read_interrupt_status().clock_timeout()
            {
                if !is_last {
                    self.write_command(I2cCommand::Stop);
                }
                self.clear_rx_fifo();

                return Err(Error::ClockTimeout(
                    self.regs.read_clk_timeout_limit().value(),
                ));
            }

            if status.rx_not_empty() && read_bytes < total_len {
                let byte = self.read_fifo_unchecked();

                Self::store_transaction_read_byte(
                    operations,
                    &mut operation_index,
                    &mut byte_index,
                    byte,
                );

                read_bytes += 1;
            }
        }
    }

    /// Blocking write-read transaction on the I2C bus.
    pub fn write_read_blocking(
        &mut self,
        address: I2cAddress,
        write: &[u8],
        read: &mut [u8],
    ) -> Result<(), Error> {
        self.write_blocking_generic(
            I2cCommand::Start,
            address,
            write,
            WriteCompletionCondition::Waiting,
        )?;
        self.read_blocking(address, read)
    }
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
        self.transaction_blocking(I2cAddress::Regular(address), operations)
    }

    fn write_read(
        &mut self,
        address: u8,
        write: &[u8],
        read: &mut [u8],
    ) -> Result<(), Self::Error> {
        let addr = I2cAddress::Regular(address);
        self.write_read_blocking(addr, write, read)
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
        self.transaction_blocking(I2cAddress::TenBit(address), operations)
    }

    fn write_read(
        &mut self,
        address: TenBitAddress,
        write: &[u8],
        read: &mut [u8],
    ) -> Result<(), Self::Error> {
        let addr = I2cAddress::TenBit(address);
        self.write_read_blocking(addr, write, read)
    }
}

#[cfg(test)]
mod transaction_tests {
    use super::*;

    #[test]
    fn groups_adjacent_writes() {
        let a = [1u8, 2];
        let b = [3u8, 4, 5];
        let operations = [Operation::Write(&a), Operation::Write(&b)];

        assert_eq!(operation_run(&operations, 0), Ok((2, 5)));
    }

    #[test]
    fn groups_adjacent_reads() {
        let mut a = [0u8; 2];
        let mut b = [0u8; 3];
        let operations = [Operation::Read(&mut a), Operation::Read(&mut b)];

        assert_eq!(operation_run(&operations, 0), Ok((2, 5)));
    }

    #[test]
    fn stops_at_direction_change() {
        let write = [1u8, 2];
        let mut read = [0u8; 3];

        let operations = [Operation::Write(&write), Operation::Read(&mut read)];

        assert_eq!(operation_run(&operations, 0), Ok((1, 2)));
        assert_eq!(operation_run(&operations, 1), Ok((2, 3)));
    }

    #[test]
    fn handles_multiple_direction_changes() {
        let w1 = [1u8; 2];
        let w2 = [2u8; 3];
        let mut r1 = [0u8; 4];
        let mut r2 = [0u8; 5];
        let w3 = [3u8; 1];

        let operations = [
            Operation::Write(&w1),
            Operation::Write(&w2),
            Operation::Read(&mut r1),
            Operation::Read(&mut r2),
            Operation::Write(&w3),
        ];

        assert_eq!(operation_run(&operations, 0), Ok((2, 5)));
        assert_eq!(operation_run(&operations, 2), Ok((4, 9)));
        assert_eq!(operation_run(&operations, 4), Ok((5, 1)));
    }

    #[test]
    fn accepts_maximum_supported_run_length() {
        let a = [0u8; 1024];
        let b = [0u8; 1022];

        let operations = [Operation::Write(&a), Operation::Write(&b)];

        assert_eq!(operation_run(&operations, 0), Ok((2, 0x7fe)));
    }

    #[test]
    fn rejects_oversized_run_length() {
        let a = [0u8; 1024];
        let b = [0u8; 1023];

        let operations = [Operation::Write(&a), Operation::Write(&b)];

        assert_eq!(operation_run(&operations, 0), Err(Error::DataTooLarge));
    }

    #[test]
    fn handles_empty_operation_slice() {
        let operations: [Operation<'_>; 0] = [];

        assert_eq!(operation_run(&operations, 0), Ok((0, 0)));
    }

    #[test]
    fn groups_all_empty_operations_as_zero_length_run() {
        let a: [u8; 0] = [];
        let b: [u8; 0] = [];

        let operations = [Operation::Write(&a), Operation::Write(&b)];

        assert_eq!(operation_run(&operations, 0), Ok((2, 0)));
    }

    #[test]
    fn includes_empty_buffers_in_same_run() {
        let empty: [u8; 0] = [];
        let data = [1u8, 2, 3];

        let operations = [Operation::Write(&empty), Operation::Write(&data)];

        assert_eq!(operation_run(&operations, 0), Ok((2, 3)));
    }
}
