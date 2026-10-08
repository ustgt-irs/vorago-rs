use core::marker::PhantomData;

cfg_if::cfg_if! {
    if #[cfg(feature = "vor1x")] {
        /// I2C A base address
        pub const BASE_ADDR_0: usize = 0x4006_0000;
        /// I2C B base address
        pub const BASE_ADDR_1: usize = 0x4006_1000;
    } else if #[cfg(feature = "vor4x")] {
        /// I2C 0 base address
        pub const BASE_ADDR_0: usize = 0x4001_6000;
        /// I2C 1 base address
        pub const BASE_ADDR_1: usize = 0x4001_6400;
        /// I2C 2 base address
        pub const BASE_ADDR_2: usize = 0x4001_6800;
    }
}

/// I2C peripheral bank.
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Bank {
    /// I2C0.
    I2c0 = 0,
    /// I2C1.
    I2c1 = 1,
    /// I2C2.
    #[cfg(feature = "vor4x")]
    I2c2 = 2,
}

impl Bank {
    /// Unsafely steal the I2C peripheral block for the given port.
    ///
    /// # Safety
    ///
    /// Circumvents ownership and safety guarantees by the HAL.
    pub unsafe fn steal_regs(&self) -> MmioRegisters<'static> {
        Registers::new_mmio(*self)
    }

    /// The interrupt vector used for master-mode interrupts on this bank.
    #[cfg(feature = "vor4x")]
    pub const fn interrupt_id_master(&self) -> va416xx::Interrupt {
        match self {
            Bank::I2c0 => va416xx::Interrupt::I2C0_MS,
            Bank::I2c1 => va416xx::Interrupt::I2C1_MS,
            Bank::I2c2 => va416xx::Interrupt::I2C2_MS,
        }
    }
}

pub use types::*;

/// Register helper types.
pub mod types {
    use arbitrary_int::{u4, u5, u9, u10, u11, u20};

    pub use crate::shared::{FifoClear, TriggerLevel};

    /// Behavior when the TX FIFO is empty but a transaction is in progress.
    #[bitbybit::bitenum(u1, exhaustive = true)]
    #[derive(Default, Debug, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub enum TxFifoEmptyMode {
        /// I2C clock is stretched until data is available.
        #[default]
        Stall = 0,
        /// End the transaction.
        EndTransaction = 1,
    }

    /// Behavior when the RX FIFO is full but more data arrives.
    #[bitbybit::bitenum(u1, exhaustive = true)]
    #[derive(Default, Debug, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub enum RxFifoFullMode {
        /// I2C clock is stretched until data is available.
        #[default]
        Stall = 0,
        /// NACK further incoming data.
        Nack = 1,
    }

    /// CONTROL register.
    #[bitbybit::bitfield(u32, debug, defmt_fields(feature = "defmt"))]
    pub struct Control {
        /// The peripheral clock is enabled.
        #[bit(0, r)]
        clk_enabled: bool,
        /// The peripheral is enabled.
        #[bit(1, r)]
        enabled: bool,
        /// Enable the peripheral.
        #[bit(2, rw)]
        enable: bool,
        /// Behavior when the TX FIFO is empty.
        #[bit(3, rw)]
        tx_fifo_empty_mode: TxFifoEmptyMode,
        /// Behavior when the RX FIFO is full.
        #[bit(4, rw)]
        rx_fifo_full_mode: RxFifoFullMode,
        /// Enables the analog delay glitch filter.
        #[bit(5, rw)]
        analog_filter: bool,
        /// Enables the digital glitch filter.
        #[bit(6, rw)]
        digital_filter: bool,
        /// Enable loopback mode.
        #[bit(8, rw)]
        loopback: bool,
        /// Use the timing values from [TimingConfig] instead of the ones derived from [ClockScale].
        #[bit(9, rw)]
        enable_timing_config: bool,
    }

    /// I2C bus speed.
    #[derive(Debug, PartialEq, Eq)]
    #[bitbybit::bitenum(u1, exhaustive = true)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub enum I2cSpeed {
        /// Standard mode, 100 kHz.
        Regular100khz = 0,
        /// Fast mode, 400 kHz.
        Fast400khz = 1,
    }

    /// CLKSCALE register.
    #[bitbybit::bitfield(u32, default = 0x0, debug, defmt_fields(feature = "defmt"))]
    pub struct ClockScale {
        /// Clock divide value. Reset value: 0x18.
        #[bits(0..=7, rw)]
        div: u8,
        /// Bus speed.
        #[bit(31, rw)]
        fastmode: I2cSpeed,
    }

    /// WORDS register, configuring the number of words to transfer.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub struct Words(arbitrary_int::UInt<u32, 11>);

    impl Words {
        /// Create a new word count value.
        pub const fn new(value: u11) -> Self {
            Words(arbitrary_int::UInt::<u32, 11>::new(value.value() as u32))
        }

        /// The raw word count value.
        pub const fn value(&self) -> u11 {
            u11::new(self.0.value() as u16)
        }
    }

    /// Transfer direction of an I2C transaction.
    #[bitbybit::bitenum(u1, exhaustive = true)]
    #[derive(Default, Debug, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub enum Direction {
        /// The master sends data.
        #[default]
        Send = 0,
        /// The master receives data.
        Receive = 1,
    }

    /// ADDRESS register.
    #[bitbybit::bitfield(u32, default = 0x0, debug, defmt_bitfields(feature = "defmt"))]
    pub struct Address {
        /// Transfer direction.
        #[bit(0, rw)]
        direction: Direction,
        /// Target slave address.
        #[bits(1..=10, rw)]
        address: u10,
        /// Enables 10-bit addressing mode.
        #[bit(15, rw)]
        a10_mode: bool,
    }

    /// DATA register, used to read from and write to the FIFOs.
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub struct Data(arbitrary_int::UInt<u32, 8>);

    impl Data {
        /// Create a new data value.
        pub const fn new(value: u8) -> Self {
            Data(arbitrary_int::UInt::<u32, 8>::new(value as u32))
        }

        /// The raw data byte.
        pub const fn data(&self) -> u8 {
            self.0.value() as u8
        }
    }

    /// COMMAND register.
    #[bitbybit::bitfield(u32, default = 0x0)]
    #[derive(Debug)]
    pub struct Command {
        /// Issue a START condition.
        #[bit(0, w)]
        start: bool,
        /// Issue a STOP condition.
        #[bit(1, w)]
        stop: bool,
        /// Cancel the current transaction.
        #[bit(2, w)]
        cancel: bool,
    }

    /// STATUS register.
    #[bitbybit::bitfield(u32, debug, defmt_bitfields(feature = "defmt"))]
    pub struct Status {
        /// The I2C bus is idle.
        #[bit(0, r)]
        i2c_idle: bool,
        /// The peripheral is idle.
        #[bit(1, r)]
        idle: bool,
        /// The peripheral is waiting for a command.
        #[bit(2, r)]
        waiting: bool,
        /// The peripheral is stalling the clock.
        #[bit(3, r)]
        stalled: bool,
        /// Arbitration was lost.
        #[bit(4, r)]
        arb_lost: bool,
        /// The address byte was NACKed.
        #[bit(5, r)]
        nack_addr: bool,
        /// A data byte was NACKed.
        #[bit(6, r)]
        nack_data: bool,
        /// RX FIFO is not empty.
        #[bit(8, r)]
        rx_not_empty: bool,
        /// RX FIFO is full.
        #[bit(9, r)]
        rx_full: bool,
        /// RX FIFO fill level is at or above the trigger level.
        #[bit(11, r)]
        rx_trigger: bool,
        /// TX FIFO is empty.
        #[bit(12, r)]
        tx_empty: bool,
        /// TX FIFO is not full.
        #[bit(13, r)]
        tx_not_full: bool,
        /// TX FIFO fill level is at or below the trigger level.
        #[bit(15, r)]
        tx_trigger: bool,
        /// Raw SDA line state.
        #[bit(30, r)]
        raw_sda: bool,
        /// Raw SCL line state.
        #[bit(31, r)]
        raw_scl: bool,
    }

    /// STATE register, reflecting the internal state machine of the peripheral.
    #[bitbybit::bitfield(u32, debug, defmt_bitfields(feature = "defmt"))]
    pub struct State {
        /// Current state.
        #[bits(0..=3, rw)]
        state: u4,
        /// Current step within the state.
        #[bits(4..=7, rw)]
        step: u4,
        /// RX FIFO fill level.
        #[bits(8..=12, rw)]
        rx_fifo: u5,
        /// TX FIFO fill level.
        #[bits(14..=18, rw)]
        tx_fifo: u5,
        /// Internal bit state counter.
        #[bits(20..=28, rw)]
        bitstate: u9,
    }

    /// TXCOUNT/RXCOUNT register, reporting the number of words transferred.
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct DataCount(arbitrary_int::UInt<u32, 11>);

    impl DataCount {
        /// The raw word count value.
        pub const fn value(&self) -> u11 {
            u11::new(self.0.value() as u16)
        }
    }

    /// IRQENB register.
    #[bitbybit::bitfield(u32, default = 0x0, debug, defmt_bitfields(feature = "defmt"))]
    pub struct InterruptControl {
        /// I2C idle interrupt enable.
        #[bit(0, rw)]
        i2c_idle: bool,
        /// Idle interrupt enable.
        #[bit(1, rw)]
        idle: bool,
        /// Waiting interrupt enable.
        #[bit(2, rw)]
        waiting: bool,
        /// Stalled interrupt enable.
        #[bit(3, rw)]
        stalled: bool,
        /// Arbitration lost interrupt enable.
        #[bit(4, rw)]
        arb_lost: bool,
        /// Address NACK interrupt enable.
        #[bit(5, rw)]
        nack_addr: bool,
        /// Data NACK interrupt enable.
        #[bit(6, rw)]
        nack_data: bool,
        /// Clock timeout interrupt enable.
        #[bit(7, rw)]
        clock_timeout: bool,
        /// TX overflow interrupt enable.
        #[bit(10, rw)]
        tx_overflow: bool,
        /// RX overflow interrupt enable.
        #[bit(11, rw)]
        rx_overflow: bool,
        /// TX ready interrupt enable. TX is ready when the FIFO level is below the threshold.
        #[bit(12, rw)]
        tx_ready: bool,
        /// RX ready interrupt enable.
        #[bit(13, rw)]
        rx_ready: bool,
        /// TX empty interrupt enable.
        #[bit(14, rw)]
        tx_empty: bool,
        /// RX full interrupt enable.
        #[bit(15, rw)]
        rx_full: bool,
    }

    /// IRQ_RAW/IRQ_END register.
    #[bitbybit::bitfield(u32, debug, defmt_bitfields(feature = "defmt"))]
    pub struct InterruptStatus {
        /// I2C idle interrupt is pending.
        #[bit(0, r)]
        i2c_idle: bool,
        /// Idle interrupt is pending.
        #[bit(1, r)]
        idle: bool,
        /// Waiting interrupt is pending.
        #[bit(2, r)]
        waiting: bool,
        /// Stalled interrupt is pending.
        #[bit(3, r)]
        stalled: bool,
        /// Arbitration lost interrupt is pending.
        #[bit(4, r)]
        arb_lost: bool,
        /// Address NACK interrupt is pending.
        #[bit(5, r)]
        nack_addr: bool,
        /// Data NACK interrupt is pending.
        #[bit(6, r)]
        nack_data: bool,
        /// Clock timeout interrupt is pending.
        #[bit(7, r)]
        clock_timeout: bool,
        /// TX overflow interrupt is pending.
        #[bit(10, r)]
        tx_overflow: bool,
        /// RX overflow interrupt is pending.
        #[bit(11, r)]
        rx_overflow: bool,
        /// TX ready interrupt is pending.
        #[bit(12, r)]
        tx_ready: bool,
        /// RX ready interrupt is pending.
        #[bit(13, r)]
        rx_ready: bool,
        /// TX empty interrupt is pending.
        #[bit(14, r)]
        tx_empty: bool,
        /// RX full interrupt is pending.
        #[bit(15, r)]
        rx_full: bool,
    }

    /// IRQ_CLEAR register.
    ///
    /// The status interrupts in bits 0 to 6 latch on a rising edge of the matching [Status] bit
    /// and stay pending until cleared here. The VA416xx programmers guide claims writes to these
    /// bits have no effect, but clearing them works on the hardware.
    #[bitbybit::bitfield(u32, default = 0x0)]
    #[derive(Debug)]
    pub struct InterruptClear {
        /// Clear the I2C idle interrupt.
        #[bit(0, w)]
        i2c_idle: bool,
        /// Clear the idle interrupt.
        #[bit(1, w)]
        idle: bool,
        /// Clear the waiting interrupt.
        #[bit(2, w)]
        waiting: bool,
        /// Clear the stalled interrupt.
        #[bit(3, w)]
        stalled: bool,
        /// Clear the arbitration lost interrupt.
        #[bit(4, w)]
        arb_lost: bool,
        /// Clear the address NACK interrupt.
        #[bit(5, w)]
        nack_addr: bool,
        /// Clear the data NACK interrupt.
        #[bit(6, w)]
        nack_data: bool,
        /// Clear the clock timeout interrupt.
        #[bit(7, w)]
        clock_timeout: bool,
        /// Clear the TX overflow interrupt.
        #[bit(10, w)]
        tx_overflow: bool,
        /// Clear the RX overflow interrupt.
        #[bit(11, w)]
        rx_overflow: bool,
    }

    impl InterruptClear {
        /// Clear all interrupts.
        pub const ALL: Self = Self::builder()
            .with_i2c_idle(true)
            .with_idle(true)
            .with_waiting(true)
            .with_stalled(true)
            .with_arb_lost(true)
            .with_nack_addr(true)
            .with_nack_data(true)
            .with_clock_timeout(true)
            .with_tx_overflow(true)
            .with_rx_overflow(true)
            .build();
    }

    /// TIMINGCONFIG register, allowing manual override of the timing values derived from
    /// [ClockScale].
    #[bitbybit::bitfield(u32)]
    #[derive(Debug)]
    pub struct TimingConfig {
        /// Rise time.
        #[bits(0..=3, rw)]
        t_rise: u4,
        /// Fall time.
        #[bits(4..=7, rw)]
        t_fall: u4,
        /// Duty cycle high time of SCL.
        #[bits(8..=11, rw)]
        t_high: u4,
        /// Duty cycle low time of SCL.
        #[bits(12..=15, rw)]
        t_low: u4,
        /// Setup time for STOP.
        #[bits(16..=19, rw)]
        tsu_stop: u4,
        /// Setup time for START.
        #[bits(20..=23, rw)]
        tsu_start: u4,
        /// Data hold time.
        #[bits(24..=27, rw)]
        thd_start: u4,
        /// TBus free time between STOP and START.
        #[bits(28..=31, rw)]
        t_buf: u4,
    }

    /// CLKTOLIMIT register.
    pub struct ClockTimeoutLimit(pub arbitrary_int::UInt<u32, 20>);

    impl ClockTimeoutLimit {
        /// Create a new clock timeout limit value.
        pub fn new(value: u20) -> Self {
            ClockTimeoutLimit(arbitrary_int::UInt::<u32, 20>::new(value.value()))
        }

        /// The raw clock timeout limit value.
        pub fn value(&self) -> u20 {
            self.0
        }
    }
}

/// Register definitions for the I2C peripheral in slave mode.
pub mod slave {
    use super::{Data, DataCount, FifoClear, RxFifoFullMode, TriggerLevel, TxFifoEmptyMode};

    pub use types::*;

    /// Register helper types.
    pub mod types {
        use super::{RxFifoFullMode, TxFifoEmptyMode};
        use arbitrary_int::{u3, u4, u5, u10, u11};

        /// S0_CTRL register.
        #[bitbybit::bitfield(u32)]
        #[derive(Debug)]
        pub struct Control {
            /// The peripheral clock is enabled.
            #[bit(0, r)]
            clk_enabled: bool,
            /// The peripheral is enabled.
            #[bit(1, r)]
            enabled: bool,
            /// Enable the peripheral.
            #[bit(2, rw)]
            enable: bool,
            /// Behavior when the TX FIFO is empty.
            #[bit(3, rw)]
            tx_fifo_empty_mode: TxFifoEmptyMode,
            /// Behavior when the RX FIFO is full.
            #[bit(4, rw)]
            rx_fifo_full_mode: RxFifoFullMode,
        }

        /// S0_MAXWORDS register.
        #[bitbybit::bitfield(u32)]
        #[derive(Debug)]
        pub struct Maxwords {
            /// Maximum number of words to accept in a single transaction.
            #[bits(0..=10, rw)]
            maxwords: u11,
            /// Enable the maximum word count limit.
            #[bit(31, rw)]
            enable: bool,
        }

        /// S0_ADDRESS register.
        #[bitbybit::bitfield(u32)]
        #[derive(Debug)]
        pub struct Address {
            /// Match direction.
            #[bit(0, rw)]
            rw: bool,
            /// Slave address to match.
            #[bits(1..=10, rw)]
            address: u10,
            /// Enables 10-bit addressing mode.
            #[bit(15, rw)]
            a10_mode: bool,
        }

        /// S0_ADDRESSMASK register.
        #[bitbybit::bitfield(u32)]
        #[derive(Debug)]
        pub struct AddressMask {
            /// Will normally be 0 to match both read and write addresses.
            #[bit(0, rw)]
            rw_mask: bool,
            /// Reset value 0x3FF.
            #[bits(1..=10, rw)]
            mask: u10,
        }

        /// Direction of the last matched address, see [LastAddress].
        #[bitbybit::bitenum(u1, exhaustive = true)]
        #[derive(Default, Debug, PartialEq, Eq)]
        pub enum Direction {
            /// The master sent data.
            #[default]
            MasterSend = 0,
            /// The master received data.
            MasterReceive = 1,
        }

        /// S0_LASTADDRESS register.
        #[bitbybit::bitfield(u32)]
        #[derive(Debug)]
        pub struct LastAddress {
            /// Direction of the last matched address.
            #[bit(0, rw)]
            direction: Direction,
            /// Last matched address.
            #[bits(1..=10, rw)]
            address: u10,
        }

        /// S0_STATUS register.
        #[bitbybit::bitfield(u32, debug, defmt_fields(feature = "defmt"))]
        pub struct Status {
            /// The current transaction has completed.
            #[bit(0, r)]
            completed: bool,
            /// The peripheral is idle.
            #[bit(1, r)]
            idle: bool,
            /// The peripheral is waiting for a command.
            #[bit(2, r)]
            waiting: bool,
            /// The peripheral is stalling the TX side.
            #[bit(3, r)]
            tx_stalled: bool,
            /// The peripheral is stalling the RX side.
            #[bit(4, r)]
            rx_stalled: bool,
            /// The slave address was matched.
            #[bit(5, r)]
            address_match: bool,
            /// A data byte was NACKed.
            #[bit(6, r)]
            nack_data: bool,
            /// The first received data byte is available.
            #[bit(7, r)]
            rx_data_first: bool,
            /// RX FIFO is not empty.
            #[bit(8, r)]
            rx_not_empty: bool,
            /// RX FIFO is full.
            #[bit(9, r)]
            rx_full: bool,
            /// RX FIFO fill level is at or above the trigger level.
            #[bit(11, r)]
            rx_trigger: bool,
            /// TX FIFO is empty.
            #[bit(12, r)]
            tx_empty: bool,
            /// TX FIFO is not full.
            #[bit(13, r)]
            tx_not_full: bool,
            /// TX FIFO fill level is at or below the trigger level.
            #[bit(15, r)]
            tx_trigger: bool,
            /// The peripheral is busy.
            #[bit(28, r)]
            raw_busy: bool,
            /// Raw SDA line state.
            #[bit(30, r)]
            raw_sda: bool,
            /// Raw SCL line state.
            #[bit(31, r)]
            raw_scl: bool,
        }

        /// S0_STATE register, reflecting the internal state machine of the peripheral.
        #[bitbybit::bitfield(u32)]
        #[derive(Debug)]
        pub struct State {
            /// Current state.
            #[bits(0..=2, rw)]
            state: u3,
            /// Current step within the state.
            #[bits(4..=7, rw)]
            step: u4,
            /// RX FIFO fill level.
            #[bits(8..=12, rw)]
            rx_fifo: u5,
            /// TX FIFO fill level.
            #[bits(14..=18, rw)]
            tx_fifo: u5,
        }

        /// S0_IRQENB register.
        #[bitbybit::bitfield(u32, debug, defmt_fields(feature = "defmt"))]
        pub struct InterruptControl {
            /// Completed interrupt enable.
            #[bit(0, rw)]
            completed: bool,
            /// Idle interrupt enable.
            #[bit(1, rw)]
            idle: bool,
            /// Waiting interrupt enable.
            #[bit(2, rw)]
            waiting: bool,
            /// TX stalled interrupt enable.
            #[bit(3, rw)]
            tx_stalled: bool,
            /// RX stalled interrupt enable.
            #[bit(4, rw)]
            rx_stalled: bool,
            /// Address match interrupt enable.
            #[bit(5, rw)]
            address_match: bool,
            /// Data NACK interrupt enable.
            #[bit(6, rw)]
            nack_data: bool,
            /// First received data byte interrupt enable.
            #[bit(7, rw)]
            rx_data_first: bool,

            /// I2C start interrupt enable.
            #[bit(8, rw)]
            i2c_start: bool,
            /// I2C stop interrupt enable.
            #[bit(9, rw)]
            i2c_stop: bool,
            /// TX underflow interrupt enable.
            #[bit(10, rw)]
            tx_underflow: bool,
            /// RX underflow interrupt enable.
            #[bit(11, rw)]
            rx_underflow: bool,
            /// TX ready interrupt enable.
            #[bit(12, rw)]
            tx_ready: bool,
            /// RX ready interrupt enable.
            #[bit(13, rw)]
            rx_ready: bool,
            /// TX empty interrupt enable.
            #[bit(14, rw)]
            tx_empty: bool,
            /// RX full interrupt enable.
            #[bit(15, rw)]
            rx_full: bool,
        }

        /// S0_IRQ_RAW/S0_IRQ_END register.
        #[bitbybit::bitfield(u32, debug, defmt_bitfields(feature = "defmt"))]
        pub struct InterruptStatus {
            /// Completed interrupt is pending.
            #[bit(0, r)]
            completed: bool,
            /// Idle interrupt is pending.
            #[bit(1, r)]
            idle: bool,
            /// Waiting interrupt is pending.
            #[bit(2, r)]
            waiting: bool,
            /// TX stalled interrupt is pending.
            #[bit(3, r)]
            tx_stalled: bool,
            /// RX stalled interrupt is pending.
            #[bit(4, r)]
            rx_stalled: bool,
            /// Address match interrupt is pending.
            #[bit(5, r)]
            address_match: bool,
            /// Data NACK interrupt is pending.
            #[bit(6, r)]
            nack_data: bool,
            /// First received data byte interrupt is pending.
            #[bit(7, r)]
            rx_data_first: bool,

            /// I2C start interrupt is pending.
            #[bit(8, r)]
            i2c_start: bool,
            /// I2C stop interrupt is pending.
            #[bit(9, r)]
            i2c_stop: bool,
            /// TX underflow interrupt is pending.
            #[bit(10, r)]
            tx_underflow: bool,
            /// RX underflow interrupt is pending.
            #[bit(11, r)]
            rx_underflow: bool,
            /// TX ready interrupt is pending.
            #[bit(12, r)]
            tx_ready: bool,
            /// RX ready interrupt is pending.
            #[bit(13, r)]
            rx_ready: bool,
            /// TX empty interrupt is pending.
            #[bit(14, r)]
            tx_empty: bool,
            /// RX full interrupt is pending.
            #[bit(15, r)]
            rx_full: bool,
        }

        /// S0_IRQ_CLEAR register.
        #[bitbybit::bitfield(u32, default = 0x0)]
        #[derive(Debug)]
        pub struct InterruptClear {
            /// Clear the completed interrupt.
            #[bit(0, w)]
            completed: bool,
            /// Clear the idle interrupt.
            #[bit(1, w)]
            idle: bool,
            /// Clear the waiting interrupt.
            #[bit(2, w)]
            waiting: bool,
            /// Clear the TX stalled interrupt.
            #[bit(3, w)]
            tx_stalled: bool,
            /// Clear the RX stalled interrupt.
            #[bit(4, w)]
            rx_stalled: bool,
            /// Clear the address match interrupt.
            #[bit(5, w)]
            address_match: bool,
            /// Clear the data NACK interrupt.
            #[bit(6, w)]
            nack_data: bool,
            /// Clear the first received data byte interrupt.
            #[bit(7, w)]
            rx_data_first: bool,

            /// Clear the I2C start interrupt.
            #[bit(8, w)]
            i2c_start: bool,
            /// Clear the I2C stop interrupt.
            #[bit(9, w)]
            i2c_stop: bool,
            /// Clear the TX underflow interrupt.
            #[bit(10, w)]
            tx_underflow: bool,
            /// Clear the RX underflow interrupt.
            #[bit(11, w)]
            rx_underflow: bool,
            /// Clear the TX ready interrupt.
            #[bit(12, w)]
            tx_ready: bool,
            /// Clear the RX ready interrupt.
            #[bit(13, w)]
            rx_ready: bool,
            /// Clear the TX empty interrupt.
            #[bit(14, w)]
            tx_empty: bool,
            /// Clear the RX full interrupt.
            #[bit(15, w)]
            rx_full: bool,
        }
    }

    /// I2C slave-mode peripheral register block.
    #[derive(derive_mmio::Mmio)]
    #[repr(C)]
    pub struct Registers {
        s0_ctrl: Control,
        s0_maxwords: Maxwords,
        s0_address: Address,
        s0_addressmask: AddressMask,
        s0_data: Data,
        s0_lastaddress: LastAddress,
        #[mmio(PureRead)]
        s0_status: Status,
        #[mmio(PureRead)]
        s0_state: State,
        #[mmio(PureRead)]
        s0_tx_count: DataCount,
        #[mmio(PureRead)]
        s0_rx_count: DataCount,
        s0_irq_enb: InterruptControl,
        #[mmio(PureRead)]
        s0_irq_raw: InterruptStatus,
        #[mmio(PureRead)]
        s0_irq_status: InterruptStatus,
        #[mmio(Write)]
        s0_irq_clear: InterruptClear,
        s0_rx_fifo_trigger: TriggerLevel,
        s0_tx_fifo_trigger: TriggerLevel,
        #[mmio(Write)]
        s0_fifo_clear: FifoClear,
        s0_address_b: Address,
        s0_addressmask_b: AddressMask,
    }
}

/// I2C peripheral register block.
#[derive(derive_mmio::Mmio)]
#[mmio(no_ctors)]
#[repr(C)]
pub struct Registers {
    control: Control,
    clkscale: ClockScale,
    words: Words,
    address: Address,
    data: Data,
    #[mmio(Write)]
    command: Command,
    #[mmio(PureRead)]
    status: Status,
    #[mmio(PureRead)]
    state: State,
    #[mmio(PureRead)]
    tx_count: DataCount,
    #[mmio(PureRead)]
    rx_count: DataCount,
    interrupt_enable: InterruptControl,
    #[mmio(PureRead)]
    interrupt_raw: InterruptStatus,
    #[mmio(PureRead)]
    interrupt_status: InterruptStatus,
    #[mmio(Write)]
    interrupt_clear: InterruptClear,
    rx_fifo_trigger: TriggerLevel,
    tx_fifo_trigger: TriggerLevel,
    #[mmio(Write)]
    fifo_clear: FifoClear,
    timing_config: TimingConfig,
    clk_timeout_limit: ClockTimeoutLimit,

    _reserved_0: [u32; 0x2D],

    #[mmio(Inner)]
    slave: slave::Registers,

    #[cfg(feature = "vor1x")]
    _reserved_1: [u32; 0x3AC],
    #[cfg(feature = "vor4x")]
    _reserved_1: [u32; 0xAC],

    /// Vorago 4x: 0x0214_07E9. Vorago 1x: 0x0014_07E1.
    #[mmio(PureRead)]
    perid: u32,
}

cfg_if::cfg_if! {
    if #[cfg(feature = "vor1x")] {
        static_assertions::const_assert_eq!(core::mem::size_of::<Registers>(), 0x1000);
    } else if #[cfg(feature = "vor4x")] {
        static_assertions::const_assert_eq!(core::mem::size_of::<Registers>(), 0x400);
    }
}

impl Registers {
    fn new_mmio_at(base: usize) -> MmioRegisters<'static> {
        MmioRegisters {
            ptr: base as *mut _,
            phantom: PhantomData,
        }
    }

    /// Get an MMIO accessor for the I2C register block of the given bank.
    pub fn new_mmio(bank: Bank) -> MmioRegisters<'static> {
        match bank {
            Bank::I2c0 => Self::new_mmio_at(BASE_ADDR_0),
            Bank::I2c1 => Self::new_mmio_at(BASE_ADDR_1),
            #[cfg(feature = "vor4x")]
            Bank::I2c2 => Self::new_mmio_at(BASE_ADDR_2),
        }
    }
}
