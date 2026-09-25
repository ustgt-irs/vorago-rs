use crate::FunctionSelect;
use crate::gpio::{DynPinId, IoPeriphPin};
use crate::{PeripheralSelect, enable_peripheral_clock, pins::AnyPin, sealed::Sealed, time::Hertz};
use core::{convert::Infallible, fmt::Debug, marker::PhantomData};
use embedded_hal::spi::Mode;

use regs::{ClockPrescaler, Data, FifoClear, WordSize};
#[cfg(feature = "vor1x")]
use va108xx as pac;
#[cfg(feature = "vor4x")]
use va416xx as pac;

pub use regs::{Bank, HwChipSelectId};

pub use embedded_hal::spi::{MODE_0, MODE_1, MODE_2, MODE_3};

/// Async SPI support.
pub mod asynch;
/// Register definitions for the SPI peripheral.
pub mod regs;

/// Depth of the TX and RX hardware FIFOs.
pub const FIFO_DEPTH: usize = 16;

/// Configure the given pin as a hardware chip select pin and return its ID.
pub fn configure_pin_as_hw_cs_pin<P: AnyPin + HwCsProvider>(_pin: P) -> HwChipSelectId {
    IoPeriphPin::new(P::ID, P::FUN_SEL, None);
    P::CS_ID
}

//==================================================================================================
// Pins and traits.
//==================================================================================================

/// Marker trait for pins usable as the SCK pin of SPI0.
pub trait PinSck0: AnyPin {
    /// SPI bank this pin belongs to.
    const SPI_ID: Bank = Bank::Spi0;
    /// Alternate function to select to route this pin to the SPI peripheral.
    const FUN_SEL: FunctionSelect;
}

/// Marker trait for pins usable as the MOSI pin of SPI0.
pub trait PinMosi0: AnyPin {
    /// SPI bank this pin belongs to.
    const SPI_ID: Bank = Bank::Spi0;
    /// Alternate function to select to route this pin to the SPI peripheral.
    const FUN_SEL: FunctionSelect;
}

/// Marker trait for pins usable as the MISO pin of SPI0.
pub trait PinMiso0: AnyPin {
    /// SPI bank this pin belongs to.
    const SPI_ID: Bank = Bank::Spi0;
    /// Alternate function to select to route this pin to the SPI peripheral.
    const FUN_SEL: FunctionSelect;
}

/// Marker trait for pins usable as the SCK pin of SPI1.
pub trait PinSck1: AnyPin {
    /// SPI bank this pin belongs to.
    const SPI_ID: Bank = Bank::Spi1;
    /// Alternate function to select to route this pin to the SPI peripheral.
    const FUN_SEL: FunctionSelect;
}

/// Marker trait for pins usable as the MOSI pin of SPI1.
pub trait PinMosi1: AnyPin {
    /// SPI bank this pin belongs to.
    const SPI_ID: Bank = Bank::Spi1;
    /// Alternate function to select to route this pin to the SPI peripheral.
    const FUN_SEL: FunctionSelect;
}

/// Marker trait for pins usable as the MISO pin of SPI1.
pub trait PinMiso1: AnyPin {
    /// SPI bank this pin belongs to.
    const SPI_ID: Bank = Bank::Spi1;
    /// Alternate function to select to route this pin to the SPI peripheral.
    const FUN_SEL: FunctionSelect;
}

/// Marker trait for pins usable as the SCK pin of SPI2.
pub trait PinSck2: AnyPin {
    /// SPI bank this pin belongs to.
    const SPI_ID: Bank = Bank::Spi2;
    /// Alternate function to select to route this pin to the SPI peripheral.
    const FUN_SEL: FunctionSelect;
}

/// Marker trait for pins usable as the MOSI pin of SPI2.
pub trait PinMosi2: AnyPin {
    /// SPI bank this pin belongs to.
    const SPI_ID: Bank = Bank::Spi2;
    /// Alternate function to select to route this pin to the SPI peripheral.
    const FUN_SEL: FunctionSelect;
}

/// Marker trait for pins usable as the MISO pin of SPI2.
pub trait PinMiso2: AnyPin {
    /// SPI bank this pin belongs to.
    const SPI_ID: Bank = Bank::Spi2;
    /// Alternate function to select to route this pin to the SPI peripheral.
    const FUN_SEL: FunctionSelect;
}

/// Trait implemented by pins usable as a hardware chip select pin.
pub trait HwCsProvider {
    /// Dynamic pin ID of the chip select pin.
    const PIN_ID: DynPinId;
    /// SPI bank this pin belongs to.
    const SPI_ID: Bank;
    /// Alternate function to select to route this pin to the SPI peripheral.
    const FUN_SEL: FunctionSelect;
    /// Hardware chip select ID controlled by this pin.
    const CS_ID: HwChipSelectId;
}

#[macro_use]
mod macros {
    #[cfg(not(feature = "va41628"))]
    macro_rules! hw_cs_multi_pin {
    (
        // name of the newtype wrapper struct
        $name:ident,
        // Pb0
        $pin_id:ident,
        // SpiId::B
        $spi_id:path,
        // FunSel::Sel1
        $fun_sel:path,
        // HwChipSelectId::Id2
        $cs_id:path
    ) => {
            #[doc = concat!(
                "Newtype wrapper to use [Pin] [`", stringify!($pin_id), "`] as a HW CS pin for [`", stringify!($spi_id), "`] with [`", stringify!($cs_id), "`]."
            )]
            pub struct $name(Pin<$pin_id>);

            impl $name {
                /// Wrap the pin as a HW CS pin.
                pub fn new(pin: Pin<$pin_id>) -> Self {
                    Self(pin)
                }
            }

            impl crate::sealed::Sealed for $name {}

            impl AnyPin for $name {
                const ID: DynPinId = <$pin_id as PinId>::ID;
            }

            impl HwCsProvider for $name {
                const PIN_ID: DynPinId = <$pin_id as PinId>::ID;
                const SPI_ID: Bank = $spi_id;
                const FUN_SEL: FunctionSelect = $fun_sel;
                const CS_ID: HwChipSelectId = $cs_id;
            }
        };
    }

    /// Implement [`crate::spi::HwCsProvider`] for a set of pins on a given SPI bank.
    #[macro_export]
    macro_rules! hw_cs_pins {
        ($SpiId:path, $(($Px:ident, $FunSel:path, $HwCsIdent:path)$(,)?)+) => {
            $(
                impl HwCsProvider for Pin<$Px> {
                    const PIN_ID: DynPinId = $Px::ID;
                    const SPI_ID: Bank = $SpiId;
                    const FUN_SEL: FunctionSelect = $FunSel;
                    const CS_ID: HwChipSelectId = $HwCsIdent;
                }
            )+
        };
    }
}

/// SPI pin type aliases for Vorago 1x devices.
#[cfg(feature = "vor1x")]
pub mod pins_vor1x;
/// SPI pin type aliases for Vorago 4x devices.
#[cfg(feature = "vor4x")]
pub mod pins_vor4x;

//==================================================================================================
// Defintions
//==================================================================================================

// FIFO has a depth of 16.
const FILL_DEPTH: usize = 12;

/// Bit set on a written word to mark the start or stop of a blockmode frame.
pub const BMSTART_BMSTOP_MASK: u32 = 1 << 31;
/// Bit set on a written word to skip storing the received word in the RX FIFO.
pub const BMSKIPDATA_MASK: u32 = 1 << 30;

/// Default SPI clock divider used by [Config::default].
pub const DEFAULT_CLK_DIV: u16 = 2;

/// Common trait implemented by the PAC peripheral access structure for SPI0.
pub trait Spi0Instance: Sealed {
    /// SPI bank of the peripheral.
    const ID: Bank = Bank::Spi0;
    /// Peripheral selector used for clock and reset control.
    const PERIPH_SEL: PeripheralSelect;
}

/// Common trait implemented by the PAC peripheral access structure for SPI1.
pub trait Spi1Instance: Sealed {
    /// SPI bank of the peripheral.
    const ID: Bank = Bank::Spi1;
    /// Peripheral selector used for clock and reset control.
    const PERIPH_SEL: PeripheralSelect;
}

/// Common trait implemented by the PAC peripheral access structure for SPI2.
pub trait Spi2Instance: Sealed {
    /// SPI bank of the peripheral.
    const ID: Bank = Bank::Spi2;
    /// Peripheral selector used for clock and reset control.
    const PERIPH_SEL: PeripheralSelect;
}

/// Common trait implemented by the PAC peripheral access structure for SPI3.
#[cfg(feature = "vor4x")]
pub trait Spi3Instance: Sealed {
    /// SPI bank of the peripheral.
    const ID: Bank = Bank::Spi3;
    /// Peripheral selector used for clock and reset control.
    const PERIPH_SEL: PeripheralSelect;
}

/// SPI0 peripheral instance.
#[cfg(feature = "vor1x")]
pub type Spi0 = pac::Spia;
/// SPI0 peripheral instance.
#[cfg(feature = "vor4x")]
pub type Spi0 = pac::Spi0;

impl Spi0Instance for Spi0 {
    const ID: Bank = Bank::Spi0;
    const PERIPH_SEL: PeripheralSelect = PeripheralSelect::Spi0;
}
impl Sealed for Spi0 {}

/// SPI1 peripheral instance.
#[cfg(feature = "vor1x")]
pub type Spi1 = pac::Spib;
/// SPI1 peripheral instance.
#[cfg(feature = "vor4x")]
pub type Spi1 = pac::Spi1;

impl Spi1Instance for Spi1 {
    const ID: Bank = Bank::Spi1;
    const PERIPH_SEL: PeripheralSelect = PeripheralSelect::Spi1;
}
impl Sealed for Spi1 {}

/// SPI2 peripheral instance.
#[cfg(feature = "vor1x")]
pub type Spi2 = pac::Spic;
/// SPI2 peripheral instance.
#[cfg(feature = "vor4x")]
pub type Spi2 = pac::Spi2;

impl Spi2Instance for Spi2 {
    const ID: Bank = Bank::Spi2;
    const PERIPH_SEL: PeripheralSelect = PeripheralSelect::Spi2;
}
impl Sealed for Spi2 {}

#[cfg(feature = "vor4x")]
impl Spi3Instance for pac::Spi3 {
    const ID: Bank = Bank::Spi3;
    const PERIPH_SEL: PeripheralSelect = PeripheralSelect::Spi3;
}
#[cfg(feature = "vor4x")]
impl Sealed for pac::Spi3 {}

//==================================================================================================
// Config
//==================================================================================================

/// Trait implemented by types which can supply per-transfer SPI configuration.
pub trait TransferConfigProvider {
    /// Set slave output disable.
    fn sod(&mut self, sod: bool);
    /// Set blockmode.
    fn blockmode(&mut self, blockmode: bool);
    /// Set the SPI mode.
    fn mode(&mut self, mode: Mode);
    /// Set the clock configuration.
    fn clk_cfg(&mut self, clk_cfg: ClockConfig);
    /// Hardware chip select ID.
    fn hw_cs_id(&self) -> u8;
}

/// Type erased variant of the transfer configuration. This is required to avoid generics in
/// the SPI constructor.
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TransferConfig {
    /// Clock configuration. If `None`, the previously configured clock is kept.
    pub clk_cfg: Option<ClockConfig>,
    /// SPI mode. If `None`, the previously configured mode is kept.
    pub mode: Option<Mode>,
    /// Slave output disable.
    pub sod: bool,
    /// If this is enabled, all data in the FIFO is transmitted in a single frame unless
    /// the BMSTOP bit is set on a dataword. A frame is defined as CSn being active for the
    /// duration of multiple data words
    pub blockmode: bool,
    /// Only used when blockmode is used. The SCK will be stalled until an explicit stop bit
    /// is set on a written word.
    pub bmstall: bool,
    /// Hardware chip select to use for this transfer.
    pub hw_cs: Option<HwChipSelectId>,
}

impl TransferConfig {
    /// Create a new transfer configuration using the given hardware chip select.
    pub fn new_with_hw_cs(
        clk_cfg: Option<ClockConfig>,
        mode: Option<Mode>,
        blockmode: bool,
        bmstall: bool,
        sod: bool,
        hw_cs_id: HwChipSelectId,
    ) -> Self {
        TransferConfig {
            clk_cfg,
            mode,
            sod,
            blockmode,
            bmstall,
            hw_cs: Some(hw_cs_id),
        }
    }
}

/// Configuration options for the whole SPI bus. See Programmer Guide p.92 for more details
#[derive(Debug, Copy, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub struct Config {
    /// Clock configuration.
    pub clock: ClockConfig,
    /// SPI mode configuration.
    pub mode: Mode,
    /// If this is enabled, all data in the FIFO is transmitted in a single frame unless
    /// the BMSTOP bit is set on a dataword. A frame is defined as CSn being active for the
    /// duration of multiple data words. Defaults to true.
    pub blockmode: bool,
    /// This enables the stalling of the SPI SCK if in blockmode and the FIFO is empty.
    /// Currently enabled by default.
    pub bmstall: bool,
    /// Slave output disable. Useful if separate GPIO pins or decoders are used for CS control
    pub slave_output_disable: bool,
    /// Loopback mode. If you use this, don't connect MISO to MOSI, they will be tied internally
    pub loopback_mode: bool,
    /// Enable Master Delayer Capture Mode. See Programmers Guide p.92 for more details
    pub master_delayer_capture: bool,
}

impl Config {
    /// Create a new configuration with the given mode and clock configuration.
    #[inline]
    pub const fn new(mode: Mode, clock: ClockConfig) -> Self {
        Self {
            clock,
            mode,
            blockmode: true,
            bmstall: true,
            slave_output_disable: false,
            loopback_mode: false,
            master_delayer_capture: false,
        }
    }
}

impl Default for Config {
    #[inline]
    fn default() -> Self {
        Self::new(MODE_0, ClockConfig::from_div(DEFAULT_CLK_DIV).unwrap())
    }
}

//==================================================================================================
// Word Size
//==================================================================================================

/// Configuration trait for the Word Size
/// used by the SPI peripheral
pub trait SpiWord: Copy + Default + Into<u32> + TryFrom<u32> + 'static {
    /// Bit mask covering the valid bits of a word of this size.
    const MASK: u32;
    /// Word size to write to the CTRL0 register.
    const WORD_SIZE: regs::WordSize;
    /// Raw word size register value.
    fn word_reg() -> u8;
}

impl SpiWord for u8 {
    const MASK: u32 = 0xff;
    const WORD_SIZE: regs::WordSize = regs::WordSize::EightBits;
    fn word_reg() -> u8 {
        0x07
    }
}

impl SpiWord for u16 {
    const MASK: u32 = 0xffff;
    const WORD_SIZE: regs::WordSize = regs::WordSize::SixteenBits;
    fn word_reg() -> u8 {
        0x0f
    }
}

//==================================================================================================
// Spi
//==================================================================================================

/// Low level access trait for the SPI peripheral.
pub trait SpiLowLevel {
    /// Low level function to write a word to the SPI FIFO but also checks whether
    /// there is actually data in the FIFO.
    ///
    /// Uses the [nb] API to allow usage in blocking and non-blocking contexts.
    fn write_fifo(&mut self, data: u32) -> nb::Result<(), Infallible>;

    /// Low level function to write a word to the SPI FIFO without checking whether
    /// there FIFO is full.
    ///
    /// This does not necesarily mean there is a space in the FIFO available.
    /// Use [Self::write_fifo] function to write a word into the FIFO reliably.
    fn write_fifo_unchecked(&mut self, data: u32);

    /// Low level function to read a word from the SPI FIFO. Must be preceeded by a
    /// [Self::write_fifo] call.
    ///
    /// Uses the [nb] API to allow usage in blocking and non-blocking contexts.
    fn read_fifo(&mut self) -> nb::Result<u32, Infallible>;

    /// Low level function to read a word from from the SPI FIFO.
    ///
    /// This does not necesarily mean there is a word in the FIFO available.
    /// Use the [Self::read_fifo] function to read a word from the FIFO reliably using the [nb]
    /// API.
    /// You might also need to mask the value to ignore the BMSTART/BMSTOP bit.
    fn read_fifo_unchecked(&mut self) -> u32;
}

/// Convert an [embedded_hal::spi::Mode] to the corresponding CPOL/CPHA bit pair.
#[inline(always)]
pub fn mode_to_cpo_cph_bit(mode: embedded_hal::spi::Mode) -> (bool, bool) {
    match mode {
        embedded_hal::spi::MODE_0 => (false, false),
        embedded_hal::spi::MODE_1 => (false, true),
        embedded_hal::spi::MODE_2 => (true, false),
        embedded_hal::spi::MODE_3 => (true, true),
    }
}

/// SPI clock configuration, expressed as a prescaler and a serial clock rate divider.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ClockConfig {
    prescale_val: u8,
    scrdv: u8,
}

impl ClockConfig {
    /// Raw clock prescaler value.
    pub fn prescale_val(&self) -> u8 {
        self.prescale_val
    }

    /// Raw serial clock rate divider value.
    pub fn scrdv(&self) -> u8 {
        self.scrdv
    }
}

impl ClockConfig {
    /// Create a clock configuration from raw prescaler and divider values.
    pub fn new(prescale_val: u8, scrdv: u8) -> Self {
        Self {
            prescale_val,
            scrdv,
        }
    }

    /// Create a clock configuration from a total clock division value.
    pub fn from_div(div: u16) -> Result<Self, SpiClockConfigError> {
        spi_clk_config_from_div(div)
    }

    /// Create a clock configuration from the given system clock and a target SPI clock speed.
    #[cfg(feature = "vor1x")]
    pub fn from_clk(sys_clk: Hertz, spi_clk: Hertz) -> Option<Self> {
        clk_div_for_target_clock(sys_clk, spi_clk).map(|div| spi_clk_config_from_div(div).unwrap())
    }

    /// Create a clock configuration from the APB1 clock and a target SPI clock speed.
    #[cfg(feature = "vor4x")]
    pub fn from_clks(clks: &crate::clock::Clocks, spi_clk: Hertz) -> Option<Self> {
        Self::from_apb1_clk(clks.apb1(), spi_clk)
    }

    /// Create a clock configuration from the given APB1 clock and a target SPI clock speed.
    #[cfg(feature = "vor4x")]
    pub fn from_apb1_clk(apb1_clk: Hertz, spi_clk: Hertz) -> Option<Self> {
        clk_div_for_target_clock(apb1_clk, spi_clk).map(|div| spi_clk_config_from_div(div).unwrap())
    }
}

/// Error type for [ClockConfig::from_div] and [spi_clk_config_from_div].
#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SpiClockConfigError {
    /// Division value was zero.
    #[error("division by zero")]
    DivIsZero,
    /// Division value is not even.
    #[error("divide value is not even")]
    DivideValueNotEven,
    /// The resulting SCRDV value is too large to fit into 8 bits.
    #[error("scrdv value is too large")]
    ScrdvValueTooLarge,
}

/// Derive a [ClockConfig] from a total clock division value.
#[inline]
pub fn spi_clk_config_from_div(mut div: u16) -> Result<ClockConfig, SpiClockConfigError> {
    if div == 0 {
        return Err(SpiClockConfigError::DivIsZero);
    }
    if !div.is_multiple_of(2) {
        return Err(SpiClockConfigError::DivideValueNotEven);
    }
    let mut prescale_val = 0;

    // find largest (even) prescale value that divides into div
    for i in (2..=0xfe).rev().step_by(2) {
        if div.is_multiple_of(i) {
            prescale_val = i;
            break;
        }
    }

    if prescale_val == 0 {
        return Err(SpiClockConfigError::DivideValueNotEven);
    }

    div /= prescale_val;
    if div > u8::MAX as u16 + 1 {
        return Err(SpiClockConfigError::ScrdvValueTooLarge);
    }
    Ok(ClockConfig {
        prescale_val: prescale_val as u8,
        scrdv: (div - 1) as u8,
    })
}

/// Calculate the total clock division value needed to reach a target SPI clock from a source
/// clock, rounding up conservatively.
#[inline]
pub fn clk_div_for_target_clock(sys_clk: Hertz, spi_clk: Hertz) -> Option<u16> {
    if spi_clk > sys_clk {
        return None;
    }

    // Step 1: Calculate raw divider.
    let raw_div = sys_clk.to_raw() / spi_clk.to_raw();
    let remainder = sys_clk.to_raw() % spi_clk.to_raw();

    // Step 2: Round up if necessary.
    let mut rounded_div = if remainder * 2 >= spi_clk.to_raw() {
        raw_div + 1
    } else {
        raw_div
    };

    if !rounded_div.is_multiple_of(2) {
        // Take slower clock conservatively.
        rounded_div += 1;
    }
    if rounded_div > u16::MAX as u32 {
        return None;
    }
    Some(rounded_div as u16)
}

/// SPI peripheral driver structure.
pub struct Spi<Word = u8> {
    id: Bank,
    regs: regs::MmioRegisters<'static>,
    /// Fill word for read-only SPI transactions.
    fill_word: Word,
    blockmode: bool,
    bmstall: bool,
    word: PhantomData<Word>,
}

impl<Word: SpiWord> Spi<Word>
where
    <Word as TryFrom<u32>>::Error: core::fmt::Debug,
{
    /// Create a new SPI struct for using SPI with the fixed ROM SPI pins.
    ///
    /// ## Arguments
    ///
    /// * `spi` - SPI bus to use
    /// * `spi_cfg` - Configuration specific to the SPI bus
    #[cfg(feature = "vor1x")]
    pub fn new_for_rom<Spi: Spi2Instance>(_spi: Spi, spi_cfg: Config) -> Self {
        Self::new_generic(Spi::ID, Spi::PERIPH_SEL, spi_cfg)
    }

    /// Create a new SPI struct for using SPI with the fixed ROM SPI pins.
    ///
    /// ## Arguments
    ///
    /// * `spi` - SPI bus to use
    /// * `spi_cfg` - Configuration specific to the SPI bus
    #[cfg(feature = "vor4x")]
    pub fn new_for_rom<Spi: Spi3Instance>(_spi: Spi, spi_cfg: Config) -> Self {
        Self::new_generic(Spi::ID, Spi::PERIPH_SEL, spi_cfg)
    }

    /// Create a new SPI peripheral driver for SPI 0.
    ///
    /// ## Arguments
    ///
    /// * `spi` - SPI bus to use
    /// * `pins` - Pins to be used for SPI transactions. These pins are consumed
    ///   to ensure the pins can not be used for other purposes anymore
    /// * `spi_cfg` - Configuration specific to the SPI bus
    pub fn new_for_spi0<Spi: Spi0Instance, Sck: PinSck0, Miso: PinMiso0, Mosi: PinMosi0>(
        _spi: Spi,
        _pins: (Sck, Miso, Mosi),
        spi_cfg: Config,
    ) -> Self {
        IoPeriphPin::new(Sck::ID, Sck::FUN_SEL, None);
        IoPeriphPin::new(Miso::ID, Miso::FUN_SEL, None);
        IoPeriphPin::new(Mosi::ID, Mosi::FUN_SEL, None);
        Self::new_generic(Spi::ID, Spi::PERIPH_SEL, spi_cfg)
    }

    /// Create a new SPI peripheral driver for SPI 1.
    ///
    /// ## Arguments
    ///
    /// * `spi` - SPI bus to use
    /// * `pins` - Pins to be used for SPI transactions. These pins are consumed
    ///   to ensure the pins can not be used for other purposes anymore
    /// * `spi_cfg` - Configuration specific to the SPI bus
    pub fn new_for_spi1<Spi: Spi1Instance, Sck: PinSck1, Miso: PinMiso1, Mosi: PinMosi1>(
        _spi: Spi,
        _pins: (Sck, Miso, Mosi),
        spi_cfg: Config,
    ) -> Self {
        IoPeriphPin::new(Sck::ID, Sck::FUN_SEL, None);
        IoPeriphPin::new(Miso::ID, Miso::FUN_SEL, None);
        IoPeriphPin::new(Mosi::ID, Mosi::FUN_SEL, None);
        Self::new_generic(Spi::ID, Spi::PERIPH_SEL, spi_cfg)
    }

    /// Create a new SPI peripheral driver for SPI 2.
    ///
    /// ## Arguments
    ///
    /// * `spi` - SPI bus to use
    /// * `pins` - Pins to be used for SPI transactions. These pins are consumed
    ///   to ensure the pins can not be used for other purposes anymore
    /// * `spi_cfg` - Configuration specific to the SPI bus
    pub fn new_for_spi2<Spi: Spi2Instance, Sck: PinSck2, Miso: PinMiso2, Mosi: PinMosi2>(
        _spi: Spi,
        _pins: (Sck, Miso, Mosi),
        spi_cfg: Config,
    ) -> Self {
        IoPeriphPin::new(Sck::ID, Sck::FUN_SEL, None);
        IoPeriphPin::new(Miso::ID, Miso::FUN_SEL, None);
        IoPeriphPin::new(Mosi::ID, Mosi::FUN_SEL, None);
        Self::new_generic(Spi::ID, Spi::PERIPH_SEL, spi_cfg)
    }

    /// Create a new SPI peripheral driver for the given bank and peripheral selector, without
    /// configuring any pins.
    pub fn new_generic(spi_sel: Bank, periph_sel: PeripheralSelect, spi_cfg: Config) -> Self {
        enable_peripheral_clock(periph_sel);
        let mut regs = regs::Registers::new_mmio(spi_sel);
        let (cpo_bit, cph_bit) = mode_to_cpo_cph_bit(spi_cfg.mode);
        regs.write_ctrl0(
            regs::Control0::builder()
                .with_scrdv(spi_cfg.clock.scrdv)
                .with_sph(cph_bit)
                .with_spo(cpo_bit)
                .with_word_size(Word::WORD_SIZE)
                .build(),
        );
        regs.write_ctrl1(
            regs::Control1::builder()
                .with_mtxpause(false)
                .with_mdlycap(spi_cfg.master_delayer_capture)
                .with_bm_stall(spi_cfg.bmstall)
                .with_bm_start(false)
                .with_blockmode(spi_cfg.blockmode)
                .with_ss(HwChipSelectId::Id0)
                .with_sod(spi_cfg.slave_output_disable)
                .with_slave_mode(false)
                .with_enable(false)
                .with_lbm(spi_cfg.loopback_mode)
                .build(),
        );
        regs.write_clkprescale(ClockPrescaler::new(spi_cfg.clock.prescale_val));
        regs.write_fifo_clear(
            FifoClear::builder()
                .with_tx_fifo(true)
                .with_rx_fifo(true)
                .build(),
        );
        // Enable the peripheral as the last step as recommended in the
        // programmers guide
        regs.modify_ctrl1(|mut value| {
            value.set_enable(true);
            value
        });
        Spi {
            id: spi_sel,
            regs: regs::Registers::new_mmio(spi_sel),
            fill_word: Default::default(),
            bmstall: spi_cfg.bmstall,
            blockmode: spi_cfg.blockmode,
            word: PhantomData,
        }
    }

    /// Apply the given clock configuration.
    #[inline]
    pub fn cfg_clock(&mut self, cfg: ClockConfig) {
        self.regs.modify_ctrl0(|mut value| {
            value.set_scrdv(cfg.scrdv);
            value
        });
        self.regs
            .write_clkprescale(regs::ClockPrescaler::new(cfg.prescale_val));
    }

    /// Set the fill word used for read-only SPI transactions.
    pub fn set_fill_word(&mut self, fill_word: Word) {
        self.fill_word = fill_word;
    }

    /// Apply a clock configuration derived from the given total clock division value.
    #[inline]
    pub fn configure_clock_from_div(&mut self, div: u16) -> Result<(), SpiClockConfigError> {
        let val = spi_clk_config_from_div(div)?;
        self.cfg_clock(val);
        Ok(())
    }

    /// Apply the given SPI mode.
    #[inline]
    pub fn configure_mode(&mut self, mode: Mode) {
        let (cpo_bit, cph_bit) = mode_to_cpo_cph_bit(mode);
        self.regs.modify_ctrl0(|mut value| {
            value.set_spo(cpo_bit);
            value.set_sph(cph_bit);
            value
        });
    }

    /// Fill word used for read-only SPI transactions.
    #[inline]
    pub fn fill_word(&self) -> Word {
        self.fill_word
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

    /// Read the peripheral ID register.
    #[inline]
    pub fn peripheral_id(&self) -> u32 {
        self.regs.read_perid()
    }

    /// Configure the hardware chip select given a hardware chip select ID.
    ///
    /// The pin also needs to be configured to be used as a HW CS pin. This can be done
    /// by using the [configure_pin_as_hw_cs_pin] function which also returns the
    /// corresponding [HwChipSelectId].
    #[inline]
    pub fn configure_hw_cs(&mut self, hw_cs: HwChipSelectId) {
        self.regs.modify_ctrl1(|mut value| {
            value.set_sod(false);
            value.set_ss(hw_cs);
            value
        });
    }

    /// Disables the hardware chip select functionality. This can be used when performing
    /// external chip select handling, for example with GPIO pins.
    #[inline]
    pub fn disable_hw_cs(&mut self) {
        self.regs.modify_ctrl1(|mut value| {
            value.set_sod(true);
            value
        });
    }

    /// Utility function to configure all relevant transfer parameters in one go.
    /// This is useful if multiple devices with different clock and mode configurations
    /// are connected to one bus.
    pub fn configure_transfer(&mut self, transfer_cfg: &TransferConfig) {
        if let Some(trans_clk_div) = transfer_cfg.clk_cfg {
            self.cfg_clock(trans_clk_div);
        }
        if let Some(mode) = transfer_cfg.mode {
            self.configure_mode(mode);
        }
        self.blockmode = transfer_cfg.blockmode;
        self.regs.modify_ctrl1(|mut value| {
            if transfer_cfg.sod {
                value.set_sod(transfer_cfg.sod);
            } else {
                value.set_sod(false);
                if let Some(hw_cs) = transfer_cfg.hw_cs {
                    value.set_ss(hw_cs);
                }
            }
            value.set_blockmode(transfer_cfg.blockmode);
            value.set_bm_stall(transfer_cfg.bmstall);
            value
        });
    }

    fn flush_internal(&mut self) {
        let mut status_reg = self.regs.read_status();
        while !status_reg.tx_empty() || status_reg.rx_not_empty() || status_reg.busy() {
            if status_reg.rx_not_empty() {
                self.read_fifo_unchecked();
            }
            status_reg = self.regs.read_status();
        }
    }

    fn transfer_preparation(&mut self, words: &[Word]) {
        if words.is_empty() {
            return;
        }
        self.flush_internal();
    }

    // The FIFO can hold a guaranteed amount of data, so we can pump it on transfer
    // initialization. Returns the amount of written bytes.
    fn initial_send_fifo_pumping_with_words(&mut self, words: &[Word]) -> usize {
        //let reg_block = self.reg_block();
        if self.blockmode {
            self.regs.modify_ctrl1(|mut value| {
                value.set_mtxpause(true);
                value
            });
        }
        // Fill the first half of the write FIFO
        let mut current_write_idx = 0;
        let smaller_idx = core::cmp::min(FILL_DEPTH, words.len());
        for _ in 0..smaller_idx {
            if current_write_idx == smaller_idx.saturating_sub(1) && self.bmstall {
                self.write_fifo_unchecked(words[current_write_idx].into() | BMSTART_BMSTOP_MASK);
            } else {
                self.write_fifo_unchecked(words[current_write_idx].into());
            }
            current_write_idx += 1;
        }
        if self.blockmode {
            self.regs.modify_ctrl1(|mut value| {
                value.set_mtxpause(false);
                value
            });
        }
        current_write_idx
    }

    // The FIFO can hold a guaranteed amount of data, so we can pump it on transfer
    // initialization.
    fn initial_send_fifo_pumping_with_fill_words(&mut self, send_len: usize) -> usize {
        if self.blockmode {
            self.regs.modify_ctrl1(|mut value| {
                value.set_mtxpause(true);
                value
            });
        }
        // Fill the first half of the write FIFO
        let mut current_write_idx = 0;
        let smaller_idx = core::cmp::min(FILL_DEPTH, send_len);
        for _ in 0..smaller_idx {
            if current_write_idx == smaller_idx.saturating_sub(1) && self.bmstall {
                self.write_fifo_unchecked(self.fill_word.into() | BMSTART_BMSTOP_MASK);
            } else {
                self.write_fifo_unchecked(self.fill_word.into());
            }
            current_write_idx += 1;
        }
        if self.blockmode {
            self.regs.modify_ctrl1(|mut value| {
                value.set_mtxpause(false);
                value
            });
        }
        current_write_idx
    }

    /// Blocking read transaction, sending the configured fill word for each received word.
    pub fn read(&mut self, words: &mut [Word]) {
        self.transfer_preparation(words);
        let mut current_read_idx = 0;
        let mut current_write_idx = self.initial_send_fifo_pumping_with_fill_words(words.len());
        loop {
            if current_read_idx < words.len() {
                words[current_read_idx] = (nb::block!(self.read_fifo()).unwrap() & Word::MASK)
                    .try_into()
                    .unwrap();
                current_read_idx += 1;
            }
            if current_write_idx < words.len() {
                if current_write_idx == words.len() - 1 && self.bmstall {
                    nb::block!(self.write_fifo(self.fill_word.into() | BMSTART_BMSTOP_MASK))
                        .unwrap();
                } else {
                    nb::block!(self.write_fifo(self.fill_word.into())).unwrap();
                }
                current_write_idx += 1;
            }
            if current_read_idx >= words.len() && current_write_idx >= words.len() {
                break;
            }
        }
    }

    /// Blocking write transaction, discarding all received words.
    pub fn write(&mut self, words: &[Word]) {
        self.transfer_preparation(words);
        let mut current_write_idx = self.initial_send_fifo_pumping_with_words(words);
        while current_write_idx < words.len() {
            if current_write_idx == words.len() - 1 && self.bmstall {
                nb::block!(self.write_fifo(words[current_write_idx].into() | BMSTART_BMSTOP_MASK))
                    .unwrap();
            } else {
                nb::block!(self.write_fifo(words[current_write_idx].into())).unwrap();
            }
            current_write_idx += 1;
            // Ignore received words.
            if self.regs.read_status().rx_not_empty() {
                self.clear_rx_fifo();
            }
        }
    }

    /// Blocking full-duplex transaction with independent read and write buffers.
    pub fn transfer(&mut self, read: &mut [Word], write: &[Word]) {
        self.transfer_preparation(write);
        let mut current_read_idx = 0;
        let mut current_write_idx = self.initial_send_fifo_pumping_with_words(write);
        let max_idx = core::cmp::max(read.len(), write.len());
        while current_read_idx < read.len() || current_write_idx < write.len() {
            if current_write_idx < max_idx {
                if current_write_idx == write.len() - 1 && self.bmstall {
                    nb::block!(
                        self.write_fifo(write[current_write_idx].into() | BMSTART_BMSTOP_MASK)
                    )
                    .unwrap();
                } else if current_write_idx < write.len() {
                    nb::block!(self.write_fifo(write[current_write_idx].into())).unwrap();
                } else {
                    nb::block!(self.write_fifo(0)).unwrap();
                }
                current_write_idx += 1;
            }
            if current_read_idx < max_idx {
                if current_read_idx < read.len() {
                    read[current_read_idx] = (nb::block!(self.read_fifo()).unwrap() & Word::MASK)
                        .try_into()
                        .unwrap();
                } else {
                    nb::block!(self.read_fifo()).unwrap();
                }
                current_read_idx += 1;
            }
        }
    }

    /// Blocking full-duplex transaction, writing and reading back into the same buffer.
    pub fn transfer_in_place(&mut self, words: &mut [Word]) {
        self.transfer_preparation(words);
        let mut current_read_idx = 0;
        let mut current_write_idx = self.initial_send_fifo_pumping_with_words(words);

        while current_read_idx < words.len() || current_write_idx < words.len() {
            if current_write_idx < words.len() {
                if current_write_idx == words.len() - 1 && self.bmstall {
                    nb::block!(
                        self.write_fifo(words[current_write_idx].into() | BMSTART_BMSTOP_MASK)
                    )
                    .unwrap();
                } else {
                    nb::block!(self.write_fifo(words[current_write_idx].into())).unwrap();
                }
                current_write_idx += 1;
            }
            if current_read_idx < words.len() && current_read_idx < current_write_idx {
                words[current_read_idx] = (nb::block!(self.read_fifo()).unwrap() & Word::MASK)
                    .try_into()
                    .unwrap();
                current_read_idx += 1;
            }
        }
    }

    /// Block until the TX FIFO is empty, the RX FIFO is empty, and the bus is idle.
    pub fn flush(&mut self) {
        self.flush_internal();
    }
}

impl Spi<u8> {
    /// Convert this blocking driver into an asynchronous one.
    ///
    /// See [asynch::Spi::new] for more details.
    #[cfg(feature = "vor1x")]
    pub fn into_async(self, opt_irq_cfg: Option<crate::InterruptConfig>) -> asynch::Spi {
        asynch::Spi::new(self, opt_irq_cfg)
    }

    /// Convert this blocking driver into an asynchronous one.
    ///
    /// See [asynch::Spi::new] for more details.
    #[cfg(feature = "vor4x")]
    pub fn into_async(self) -> asynch::Spi {
        asynch::Spi::new(self)
    }
}

impl<W: SpiWord> SpiLowLevel for Spi<W>
where
    <W as TryFrom<u32>>::Error: core::fmt::Debug,
{
    #[inline(always)]
    fn write_fifo(&mut self, data: u32) -> nb::Result<(), Infallible> {
        if !self.regs.read_status().tx_not_full() {
            return Err(nb::Error::WouldBlock);
        }
        self.write_fifo_unchecked(data);
        Ok(())
    }

    #[inline(always)]
    fn write_fifo_unchecked(&mut self, data: u32) {
        self.regs.write_data(Data::new_with_raw_value(data));
    }

    #[inline(always)]
    fn read_fifo(&mut self) -> nb::Result<u32, Infallible> {
        if !self.regs.read_status().rx_not_empty() {
            return Err(nb::Error::WouldBlock);
        }
        Ok(self.read_fifo_unchecked())
    }

    #[inline(always)]
    fn read_fifo_unchecked(&mut self) -> u32 {
        self.regs.read_data().raw_value()
    }
}

impl<Word: SpiWord> embedded_hal::spi::ErrorType for Spi<Word> {
    type Error = Infallible;
}

impl<Word: SpiWord> embedded_hal::spi::SpiBus<Word> for Spi<Word>
where
    <Word as TryFrom<u32>>::Error: core::fmt::Debug,
{
    fn read(&mut self, words: &mut [Word]) -> Result<(), Self::Error> {
        self.read(words);
        Ok(())
    }

    fn write(&mut self, words: &[Word]) -> Result<(), Self::Error> {
        self.write(words);
        Ok(())
    }

    fn transfer(&mut self, read: &mut [Word], write: &[Word]) -> Result<(), Self::Error> {
        self.transfer(read, write);
        Ok(())
    }

    fn transfer_in_place(&mut self, words: &mut [Word]) -> Result<(), Self::Error> {
        self.transfer_in_place(words);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.flush();
        Ok(())
    }
}

/// Changing the word size also requires a type conversion
impl From<Spi<u8>> for Spi<u16> {
    fn from(mut old_spi: Spi<u8>) -> Self {
        old_spi.regs.modify_ctrl0(|mut value| {
            value.set_word_size(WordSize::SixteenBits);
            value
        });
        Spi {
            id: old_spi.id,
            regs: old_spi.regs,
            blockmode: old_spi.blockmode,
            fill_word: Default::default(),
            bmstall: old_spi.bmstall,
            word: PhantomData,
        }
    }
}

impl From<Spi<u16>> for Spi<u8> {
    fn from(mut old_spi: Spi<u16>) -> Self {
        old_spi.regs.modify_ctrl0(|mut value| {
            value.set_word_size(WordSize::EightBits);
            value
        });
        Spi {
            id: old_spi.id,
            regs: old_spi.regs,
            blockmode: old_spi.blockmode,
            fill_word: Default::default(),
            bmstall: old_spi.bmstall,
            word: PhantomData,
        }
    }
}

/// This abstraction which can be used to map a hardware chip select pin
/// to [embedded_hal::digital::OutputPin]. This is useful for creating physical chip select
/// pins required by the [embedded_hal_bus](https://docs.rs/embedded-hal-bus/latest/embedded_hal_bus/)
/// API.
pub struct HwCsPin {
    regs: regs::MmioRegisters<'static>,
    id: HwChipSelectId,
}

impl HwCsPin {
    /// Configure the given pin as a hardware chip select pin and wrap it.
    pub fn new<P: HwCsProvider + AnyPin>(pin: P) -> Self {
        configure_pin_as_hw_cs_pin(pin);
        Self {
            regs: unsafe { P::SPI_ID.steal_regs() },
            id: P::CS_ID,
        }
    }
}

impl embedded_hal::digital::ErrorType for HwCsPin {
    type Error = Infallible;
}

impl embedded_hal::digital::OutputPin for HwCsPin {
    fn set_low(&mut self) -> Result<(), Self::Error> {
        self.regs
            .modify_ctrl1(|value| value.with_sod(false).with_ss(self.id));
        Ok(())
    }

    fn set_high(&mut self) -> Result<(), Self::Error> {
        self.regs.modify_ctrl1(|value| value.with_sod(true));
        Ok(())
    }
}
