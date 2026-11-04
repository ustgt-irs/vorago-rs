use core::{marker::PhantomData, sync::atomic::Ordering};

use arbitrary_int::u5;
use portable_atomic::{AtomicBool, AtomicU8};

use crate::{
    shared::{FifoClear, TriggerLevel, asynch::TransferState},
    spi::{
        BMSKIPDATA_MASK, BMSTART_BMSTOP_MASK, FIFO_DEPTH,
        regs::{Data, InterruptClear, InterruptControl},
    },
};

/// Builds a FIFO data word, marking the last word of the transfer with the BMSTART_BMSTOP bit.
///
/// That bit is what makes the peripheral end the frame and deassert a hardware chip select after
/// the word. Without it, blockmode keeps CS asserted and stalls the clock after the transfer.
#[inline]
fn data_word(value: u32, is_last: bool) -> Data {
    if is_last {
        Data::new_with_raw_value(value | BMSTART_BMSTOP_MASK)
    } else {
        Data::new_with_raw_value(value)
    }
}

/// Number of SPI peripherals.
#[cfg(feature = "vor1x")]
pub const NUM_SPIS: usize = 3;
/// Number of SPI peripherals.
#[cfg(feature = "vor4x")]
pub const NUM_SPIS: usize = 4;

static TRANSFER_CONTEXTS: [TransferContext; NUM_SPIS] =
    [const { TransferContext::new() }; NUM_SPIS];

/// The RX FIFO overran during an async SPI transfer.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[error("SPI RX FIFO overrun")]
pub struct RxOverrunError;

impl embedded_hal_async::spi::Error for RxOverrunError {
    fn kind(&self) -> embedded_hal::spi::ErrorKind {
        embedded_hal::spi::ErrorKind::Overrun
    }
}

fn on_interrupt(peripheral: super::Bank) {
    let mut spi = unsafe { peripheral.steal_regs() };
    let index = peripheral as usize;
    let enabled_irqs = spi.read_interrupt_control();
    let interrupt_status = spi.read_interrupt_status();
    spi.write_interrupt_clear(InterruptClear::ALL);
    // Prevent spurious interrupts from messing with out logic here.
    spi.write_interrupt_control(InterruptControl::DISABLE_ALL);
    // IRQ is not related.
    if enabled_irqs.raw_value() == 0 {
        reset_trigger_levels(&mut spi);
        spi.write_fifo_clear(FifoClear::ALL);
        return;
    }
    let context = &TRANSFER_CONTEXTS[index];
    // `Acquire` pairs with the `Release` store in the `start_*` methods: observing an active
    // transfer type here guarantees the buffers and counters read below belong to that transfer.
    let Some(transfer_type) = context.active_transfer_type() else {
        // No transfer active.
        return;
    };
    if interrupt_status.rx_overrun() {
        // Not sure how to otherwise handle this cleanly..
        return handle_rx_overrun(&mut spi, context);
    }
    match transfer_type {
        TransferType::Read => on_interrupt_read(context, &mut spi, enabled_irqs),
        TransferType::Write => on_interrupt_write(context, &mut spi, enabled_irqs),
        TransferType::Transfer => on_interrupt_transfer(context, &mut spi, enabled_irqs),
        TransferType::TransferInPlace => {
            on_interrupt_transfer_in_place(context, &mut spi, enabled_irqs)
        }
    };
}

fn handle_rx_overrun(spi: &mut super::regs::MmioRegisters<'static>, context: &TransferContext) {
    context.rx_overrun.store(true, Ordering::Relaxed);
    // Clean up, restore clean state.
    reset_trigger_levels(spi);
    spi.write_fifo_clear(FifoClear::ALL);
    // Interrupts were already disabled and cleared.
    context.state.signal_done();
}

fn on_interrupt_read(
    context: &TransferContext,
    spi: &mut super::regs::MmioRegisters<'static>,
    enabled_irqs: InterruptControl,
) {
    // Safety: The transfer is active, so the slice published with it is still valid.
    let read_slice = unsafe { context.state.rx_slice() };
    let transfer_len = read_slice.len();
    let mut rx_progress = context.state.rx_progress.load(Ordering::Relaxed);
    let mut tx_progress = context.state.tx_progress.load(Ordering::Relaxed);

    // Read data from RX FIFO first.
    while spi.read_status().rx_not_empty() {
        let data = spi.read_data();
        if rx_progress < transfer_len {
            read_slice[rx_progress] = (data.data() & 0xFF) as u8;
            rx_progress += 1;
        }
    }

    // The FIFO still needs to be pumped.
    while tx_progress < transfer_len && spi.read_status().tx_not_full() {
        spi.write_data(data_word(0, tx_progress == transfer_len - 1));
        tx_progress += 1;
    }

    isr_finish_handler(
        spi,
        context,
        Progress::new(tx_progress, rx_progress),
        transfer_len,
        enabled_irqs,
    )
}

fn on_interrupt_write(
    context: &TransferContext,
    spi: &mut super::regs::MmioRegisters<'static>,
    enabled_irqs: InterruptControl,
) {
    // Safety: The transfer is active, so the slice published with it is still valid.
    let write_slice = unsafe { context.state.tx_slice() };
    let transfer_len = write_slice.len();
    let mut rx_progress = context.state.rx_progress.load(Ordering::Relaxed);
    let mut tx_progress = context.state.tx_progress.load(Ordering::Relaxed);

    // Read data from RX FIFO first.
    while spi.read_status().rx_not_empty() {
        spi.read_data();
        if rx_progress < transfer_len {
            rx_progress += 1;
        }
    }

    // Data still needs to be sent
    while tx_progress < transfer_len && spi.read_status().tx_not_full() {
        spi.write_data(data_word(
            write_slice[tx_progress] as u32,
            tx_progress == transfer_len - 1,
        ));
        tx_progress += 1;
    }

    isr_finish_handler(
        spi,
        context,
        Progress::new(tx_progress, rx_progress),
        transfer_len,
        enabled_irqs,
    )
}

fn on_interrupt_transfer(
    context: &TransferContext,
    spi: &mut super::regs::MmioRegisters<'static>,
    enabled_irqs: InterruptControl,
) {
    // Safety: The transfer is active, so the slices published with it are still valid.
    let read_slice = unsafe { context.state.rx_slice() };
    let read_len = read_slice.len();
    let write_slice = unsafe { context.state.tx_slice() };
    let transfer_len = core::cmp::max(read_len, write_slice.len());
    let mut rx_progress = context.state.rx_progress.load(Ordering::Relaxed);
    let mut tx_progress = context.state.tx_progress.load(Ordering::Relaxed);

    // Send data first to avoid overwriting data that still needs to be sent.
    while tx_progress < transfer_len && spi.read_status().tx_not_full() {
        spi.write_data(data_word(
            write_slice.get(tx_progress).copied().unwrap_or(0) as u32,
            tx_progress == transfer_len - 1,
        ));
        // Always increment this.
        tx_progress += 1;
    }

    // Read data from RX FIFO.
    while spi.read_status().rx_not_empty() {
        let data = spi.read_data();
        if rx_progress < read_len {
            read_slice[rx_progress] = (data.data() & 0xFF) as u8;
        }
        // Always increment this.
        rx_progress += 1;
    }

    isr_finish_handler(
        spi,
        context,
        Progress::new(tx_progress, rx_progress),
        transfer_len,
        enabled_irqs,
    )
}

fn on_interrupt_transfer_in_place(
    context: &TransferContext,
    spi: &mut super::regs::MmioRegisters<'static>,
    enabled_irqs: InterruptControl,
) {
    // Safety: The transfer is active, so the slice published with it is still valid.
    let transfer_slice = unsafe { context.state.rx_slice() };
    let transfer_len = transfer_slice.len();
    let mut rx_progress = context.state.rx_progress.load(Ordering::Relaxed);
    let mut tx_progress = context.state.tx_progress.load(Ordering::Relaxed);

    // Send data first to avoid overwriting data that still needs to be sent.
    while tx_progress < transfer_len && spi.read_status().tx_not_full() {
        spi.write_data(data_word(
            transfer_slice[tx_progress] as u32,
            tx_progress == transfer_len - 1,
        ));
        tx_progress += 1;
    }
    // Read data from RX FIFO.
    while spi.read_status().rx_not_empty() {
        let data = spi.read_data();
        if rx_progress < transfer_len {
            transfer_slice[rx_progress] = (data.data() & 0xFF) as u8;
            rx_progress += 1;
        }
    }

    isr_finish_handler(
        spi,
        context,
        Progress::new(tx_progress, rx_progress),
        transfer_len,
        enabled_irqs,
    )
}

/// TX and RX progress of the running transfer, as tracked inside one interrupt handler run.
///
/// The handlers keep the counters in locals and only write them back once, so the shared state
/// is touched a single time per interrupt instead of on every FIFO word.
#[derive(Debug, Clone, Copy)]
struct Progress {
    tx: usize,
    rx: usize,
}

impl Progress {
    #[inline]
    const fn new(tx: usize, rx: usize) -> Self {
        Self { tx, rx }
    }
}

/// Generic handler after RX FIFO and TX FIFO were handled. Checks and handles finished
/// and unfinished conditions.
fn isr_finish_handler(
    spi: &mut super::regs::MmioRegisters<'static>,
    context: &TransferContext,
    progress: Progress,
    transfer_len: usize,
    enabled: InterruptControl,
) {
    // Transfer finish condition.
    if progress.rx == progress.tx && progress.rx == transfer_len {
        finish_transfer(spi, context);
        return;
    }
    // Write back the updated counters. The transfer stays active, so the ISR keeps servicing it
    // on the following interrupts.
    context
        .state
        .tx_progress
        .store(progress.tx, Ordering::Relaxed);
    context
        .state
        .rx_progress
        .store(progress.rx, Ordering::Relaxed);
    unfinished_transfer(spi, transfer_len, progress, enabled);
}

fn finish_transfer(spi: &mut super::regs::MmioRegisters<'static>, context: &TransferContext) {
    // Clean up, restore clean state.
    reset_trigger_levels(spi);
    spi.write_fifo_clear(FifoClear::ALL);
    // Interrupts were already disabled and cleared.
    context.state.signal_done();
}

#[inline]
fn unfinished_transfer(
    spi: &mut super::regs::MmioRegisters<'static>,
    transfer_len: usize,
    progress: Progress,
    enabled_irqs: InterruptControl,
) {
    // Take 8 as a conservative value to make sure that the FIFO does not overflow even if there
    // is a significant delay between the interrupt being triggered and the handler being executed.
    let new_trig_level = core::cmp::min(8, transfer_len - progress.rx);
    spi.write_rx_fifo_trigger(TriggerLevel::new(u5::new(new_trig_level as u8)));

    // If TX was already enabled and the transfer is finished, stop enabling it. Otherwise, we can
    // become stuck in an interrupt loop. In any other case, enable it. I am not fully sure
    // why this is necessary and why we can not stop interrupts as soon as we have the full
    // TX progress, but tests with ADCs have shown that not doing this causes timeouts.
    let enable_tx = !(enabled_irqs.tx() && progress.tx == transfer_len);

    // Re-enable interrupts with the new RX FIFO trigger level.
    spi.write_interrupt_control(
        InterruptControl::builder()
            .with_tx(enable_tx)
            .with_rx(true)
            .with_rx_timeout(true)
            .with_rx_overrun(true)
            .build(),
    );
}

#[inline]
fn reset_trigger_levels(spi: &mut super::regs::MmioRegisters<'static>) {
    spi.write_rx_fifo_trigger(TriggerLevel::new(u5::new(0x08)));
    spi.write_tx_fifo_trigger(TriggerLevel::new(u5::new(0x00)));
}

/// Kind of transfer the async [Spi] driver is performing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum TransferType {
    /// A read transfer, see [Spi::read].
    Read = 0,
    /// A write transfer, see [Spi::write].
    Write = 1,
    /// A simultaneous read and write transfer, see [Spi::transfer].
    Transfer = 2,
    /// A simultaneous read and write transfer into the same buffer, see [Spi::transfer_in_place].
    TransferInPlace = 3,
}

impl TransferType {
    /// Stored in [TransferContext::transfer_type] while no transfer is active.
    const NONE: u8 = 0xff;

    const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Read),
            1 => Some(Self::Write),
            2 => Some(Self::Transfer),
            3 => Some(Self::TransferInPlace),
            _ => None,
        }
    }
}

/// Transfer context structure.
///
/// `transfer_type` doubles as the "transfer active" flag: it is always published last
/// (`Release`) after the buffers and progress counters in [TransferState], and read first
/// (`Acquire`) before them. A reader which observes an active transfer type is therefore
/// guaranteed to see the matching buffers and counters, rather than stale ones from a previous
/// transfer.
struct TransferContext {
    transfer_type: AtomicU8,
    state: TransferState,
    rx_overrun: AtomicBool,
}

impl TransferContext {
    const fn new() -> Self {
        Self {
            transfer_type: AtomicU8::new(TransferType::NONE),
            state: TransferState::new(),
            rx_overrun: AtomicBool::new(false),
        }
    }

    /// Marks the transfer as active. Must be called after all other fields were stored.
    #[inline]
    fn set_active(&self, transfer_type: TransferType) {
        self.transfer_type
            .store(transfer_type as u8, Ordering::Release);
    }

    /// Clears the active transfer, so a spurious interrupt can never observe the stale buffer
    /// pointers.
    #[inline]
    fn clear_active(&self) {
        self.transfer_type
            .store(TransferType::NONE, Ordering::Release);
    }

    /// Returns the active transfer type. All other fields may only be read if this returns [Some].
    #[inline]
    fn active_transfer_type(&self) -> Option<TransferType> {
        TransferType::from_raw(self.transfer_type.load(Ordering::Acquire))
    }

    /// Clears the active transfer and restores the initial state, so the slot can be reused.
    #[inline]
    fn reset(&self) {
        // Must come first in program order: a live interrupt (e.g. during Drop's
        // cancellation path) must never see the fields below cleared while still
        // observing an active transfer.
        self.clear_active();
        self.state.reset();
        self.rx_overrun.store(false, Ordering::Relaxed);
    }
}

/// Returns the transfer result once the interrupt handler signals completion.
fn poll_transfer_done(
    bank: super::Bank,
    cx: &mut core::task::Context<'_>,
) -> core::task::Poll<Result<(), RxOverrunError>> {
    let context = &TRANSFER_CONTEXTS[bank as usize];
    if !context.state.poll_done(cx.waker()) {
        return core::task::Poll::Pending;
    }
    let rx_overrun = context.rx_overrun.load(Ordering::Relaxed);
    // Clearing the active transfer tells the guard that the future saw the completion.
    context.reset();
    if rx_overrun {
        return core::task::Poll::Ready(Err(RxOverrunError));
    }
    core::task::Poll::Ready(Ok(()))
}

/// Stops an in-flight transfer on drop. The interrupt handler accesses the buffers until then.
struct TransferGuard<'spi, 'buf>(&'spi mut Spi, PhantomData<&'buf [u8]>);

impl Drop for TransferGuard<'_, '_> {
    fn drop(&mut self) {
        let context = &TRANSFER_CONTEXTS[self.0.bank_id() as usize];
        // When the future is polled to completion, the transfer type is cleared. Shutdown handling
        // was done by the interrupt handler itself.
        if context.active_transfer_type().is_none() {
            return;
        }
        // Clear the active transfer first, so the interrupt handler can not mark it as done
        // anymore.
        context.reset();
        let regs = &mut self.0.0.regs;
        regs.write_interrupt_clear(InterruptClear::ALL);
        regs.write_interrupt_control(InterruptControl::DISABLE_ALL);
        regs.write_fifo_clear(FifoClear::ALL);
        // Only needed if the transfer was cancelled while in flight.
        // This check ensures that we only send the STOP signal once.
        if !context.state.is_done() {
            // Ends the blockmode frame so CS is deasserted, without clocking out a word.
            // Must come after the FIFO clear, which would discard it.
            regs.write_data(Data::new_with_raw_value(
                BMSTART_BMSTOP_MASK | BMSKIPDATA_MASK,
            ));
        }
    }
}

/// Asynchronous SPI driver.
///
/// This is the primary data structure used to perform non-blocking SPI operations.
/// It implements the [embedded_hal_async::spi::SpiBus] as well.
pub struct Spi(pub super::Spi<u8>);

impl Spi {
    /// Construct an asynchronous SPI driver for the given SPI peripheral.
    ///
    /// # Safety
    ///
    /// The user MUST ensure that the `Drop` method of all futures generated with this driver
    /// is called on transfer cancellation. By default, this does not require any special handling.
    /// Do *NOT* use `core::mem::forget` on a started future. The interrupt handler would keep
    /// accessing the buffers after they are gone.
    /// This case was considered exotic enough to justify not making the function `unsafe`.
    pub fn new(
        mut spi: super::Spi<u8>,
        #[cfg(feature = "vor1x")] opt_irq_cfg: Option<crate::InterruptConfig>,
    ) -> Self {
        #[cfg(feature = "vor1x")]
        if let Some(irq_cfg) = opt_irq_cfg {
            spi.regs
                .write_interrupt_control(InterruptControl::DISABLE_ALL);
            spi.regs.write_interrupt_clear(InterruptClear::ALL);
            if irq_cfg.route {
                crate::enable_peripheral_clock(crate::PeripheralSelect::Irqsel);
                unsafe { va108xx::Irqsel::steal() }
                    .spi(spi.id as usize)
                    .write(|w| unsafe { w.bits(irq_cfg.id as u32) });
            }
            if irq_cfg.enable_in_nvic {
                // Safety: User has specifically configured this.
                unsafe { crate::enable_nvic_interrupt(irq_cfg.id) };
            }
        }
        // The async driver always drives blockmode frames explicitly by setting the
        // BMSTART_BMSTOP bit on the last word of each transfer, which is what deasserts a
        // hardware chip select at the end of the transfer.
        spi.regs
            .modify_ctrl1(|v| v.with_bm_stall(true).with_blockmode(true));
        Self(spi)
    }

    /// Token identifying the SPI peripheral driven by this instance.
    ///
    /// Pass this to [Self::on_interrupt] to service the peripheral's interrupts. Since it is
    /// `Copy`, it can be stashed in a `Mutex<Cell<_>>` or similar and handed to the interrupt
    /// handler without needing access to the [Spi] instance itself.
    #[inline]
    pub fn bank_id(&self) -> super::Bank {
        self.0.id
    }

    /// Generic interrupt handler to handle asynchronous SPI operations for a given SPI
    /// peripheral.
    ///
    /// The user has to call this once in the interrupt handler responsible for the SPI
    /// interrupts on the given SPI bank. Takes the token returned by [Self::bank_id] rather than
    /// the [Spi] instance itself, so it can be called from interrupt context without needing
    /// access to the driver.
    pub fn on_interrupt(bank_id: super::Bank) {
        on_interrupt(bank_id);
    }

    /// Read `words` from the slave.
    ///
    /// Returns immediately if the provided buffer is empty.
    pub async fn read(&mut self, words: &mut [u8]) -> Result<(), RxOverrunError> {
        if words.is_empty() {
            return Ok(());
        }
        let bank = self.bank_id();
        let _guard = self.start_read(words);
        core::future::poll_fn(|cx| poll_transfer_done(bank, cx)).await
    }

    /// Write `words` to the slave, ignoring all the incoming words.
    ///
    /// Returns immediately if the provided buffer is empty.
    pub async fn write(&mut self, words: &[u8]) -> Result<(), RxOverrunError> {
        if words.is_empty() {
            return Ok(());
        }
        let bank = self.bank_id();
        let _guard = self.start_write(words);
        core::future::poll_fn(|cx| poll_transfer_done(bank, cx)).await
    }

    /// Write and read simultaneously. `write` is written to the slave on MOSI and
    /// words received on MISO are stored in `read`.
    ///
    /// It is allowed for `read` and `write` to have different lengths, even zero length.
    /// The transfer runs for `max(read.len(), write.len())` words. If `read` is shorter,
    /// incoming words after `read` has been filled will be discarded. If `write` is shorter,
    /// the value of words sent in MOSI after all `write` has been sent is 0.
    ///
    /// Returns immediately if either of the provided buffers is empty.
    pub async fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), RxOverrunError> {
        if read.is_empty() || write.is_empty() {
            return Ok(());
        }
        let bank = self.bank_id();
        let _guard = self.start_transfer(read, write);
        core::future::poll_fn(|cx| poll_transfer_done(bank, cx)).await
    }

    /// Write and read simultaneously. The contents of `words` are written to the slave, and the
    /// received words are stored into the same `words` buffer, overwriting it.
    ///
    /// Returns immediately if the provided buffer is empty.
    pub async fn transfer_in_place(&mut self, words: &mut [u8]) -> Result<(), RxOverrunError> {
        if words.is_empty() {
            return Ok(());
        }
        let bank = self.bank_id();
        let _guard = self.start_transfer_in_place(words);
        core::future::poll_fn(|cx| poll_transfer_done(bank, cx)).await
    }

    fn start_read<'spi, 'buf>(&'spi mut self, words: &'buf mut [u8]) -> TransferGuard<'spi, 'buf> {
        self.init_transfer();

        let len = words.len();
        let write_index = core::cmp::min(super::FIFO_DEPTH, len);
        // Send dummy bytes.
        (0..write_index).for_each(|idx| {
            self.0.regs.write_data(data_word(0, idx == len - 1));
        });

        self.set_triggers(write_index, len);

        let context = &TRANSFER_CONTEXTS[self.bank_id() as usize];
        // Publish the guarded fields before marking the transfer active, see [TransferContext].
        // Safety: The guard borrows `words` and clears the active transfer on drop.
        unsafe { context.state.set_rx_slice(words) };
        context.state.clear_tx_slice();
        context
            .state
            .tx_progress
            .store(write_index, Ordering::Relaxed);
        context.state.rx_progress.store(0, Ordering::Relaxed);
        context.rx_overrun.store(false, Ordering::Relaxed);
        context.set_active(TransferType::Read);

        self.release_tx_pause(len > FIFO_DEPTH)
    }

    fn start_write<'spi, 'buf>(&'spi mut self, words: &'buf [u8]) -> TransferGuard<'spi, 'buf> {
        let write_index = self.init_transfer_and_prefill(words);
        let context = &TRANSFER_CONTEXTS[self.bank_id() as usize];
        // Publish the guarded fields before marking the transfer active, see [TransferContext].
        // Safety: The guard borrows `words` and clears the active transfer on drop.
        unsafe { context.state.set_tx_slice(words) };
        context.state.clear_rx_slice();
        context
            .state
            .tx_progress
            .store(write_index, Ordering::Relaxed);
        context.state.rx_progress.store(0, Ordering::Relaxed);
        context.rx_overrun.store(false, Ordering::Relaxed);
        context.set_active(TransferType::Write);

        self.release_tx_pause(words.len() > FIFO_DEPTH)
    }

    fn start_transfer<'spi, 'buf>(
        &'spi mut self,
        read: &'buf mut [u8],
        write: &'buf [u8],
    ) -> TransferGuard<'spi, 'buf> {
        let full_write_len = core::cmp::max(read.len(), write.len());
        let fifo_prefill = core::cmp::min(super::FIFO_DEPTH, full_write_len);

        self.init_transfer();

        for write_index in 0..fifo_prefill {
            let value = write.get(write_index).copied().unwrap_or(0);
            self.0
                .regs
                .write_data(data_word(value as u32, write_index == full_write_len - 1));
        }

        self.set_triggers(fifo_prefill, full_write_len);

        let context = &TRANSFER_CONTEXTS[self.bank_id() as usize];
        // Publish the guarded fields before marking the transfer active, see [TransferContext].
        // Safety: The guard borrows both buffers and clears the active transfer on drop.
        unsafe {
            context.state.set_tx_slice(write);
            context.state.set_rx_slice(read);
        }
        context
            .state
            .tx_progress
            .store(fifo_prefill, Ordering::Relaxed);
        context.state.rx_progress.store(0, Ordering::Relaxed);
        context.rx_overrun.store(false, Ordering::Relaxed);
        context.set_active(TransferType::Transfer);

        self.release_tx_pause(fifo_prefill > FIFO_DEPTH)
    }

    fn start_transfer_in_place<'spi, 'buf>(
        &'spi mut self,
        words: &'buf mut [u8],
    ) -> TransferGuard<'spi, 'buf> {
        let write_idx = self.init_transfer_and_prefill(words);
        let len = words.len();
        let context = &TRANSFER_CONTEXTS[self.bank_id() as usize];
        // Publish the guarded fields before marking the transfer active, see [TransferContext].
        // Safety: The guard borrows `words` and clears the active transfer on drop.
        unsafe { context.state.set_rx_slice(words) };
        context.state.clear_tx_slice();
        context
            .state
            .tx_progress
            .store(write_idx, Ordering::Relaxed);
        context.state.rx_progress.store(0, Ordering::Relaxed);
        context.rx_overrun.store(false, Ordering::Relaxed);
        context.set_active(TransferType::TransferInPlace);

        self.release_tx_pause(len > FIFO_DEPTH)
    }

    fn init_transfer(&mut self) {
        TRANSFER_CONTEXTS[self.bank_id() as usize]
            .state
            .clear_done();
        self.0
            .regs
            .write_interrupt_control(InterruptControl::DISABLE_ALL);
        self.0.regs.write_fifo_clear(FifoClear::ALL);
        self.0.regs.modify_ctrl1(|v| v.with_mtxpause(true));
    }

    /// Returns amount of bytes written to FIFO.
    fn init_transfer_and_prefill(&mut self, write: &[u8]) -> usize {
        self.init_transfer();

        let write_idx = core::cmp::min(super::FIFO_DEPTH, write.len());
        (0..write_idx).for_each(|idx| {
            self.0
                .regs
                .write_data(data_word(write[idx] as u32, idx == write.len() - 1));
        });

        self.set_triggers(write_idx, write.len());
        write_idx
    }

    fn set_triggers(&mut self, fifo_prefill: usize, write_len: usize) {
        self.0
            .regs
            .write_rx_fifo_trigger(TriggerLevel::new(u5::new(core::cmp::min(
                fifo_prefill,
                FIFO_DEPTH / 2,
            ) as u8)));
        // We want to re-fill the TX FIFO before it is completely empty if the full transfer size
        // is larger than the FIFO depth. Otherwise, set it to 0. Not exactly sure what that does,
        // but we do not enable interrupts anyway.
        if write_len > super::FIFO_DEPTH {
            self.0
                .regs
                .write_tx_fifo_trigger(TriggerLevel::new(u5::new(8)));
        } else {
            self.0
                .regs
                .write_tx_fifo_trigger(TriggerLevel::new(u5::new(0)));
        }
    }

    /// Enables the interrupts and starts the frame. Call after marking the transfer active.
    fn release_tx_pause<'spi, 'buf>(
        &'spi mut self,
        enable_tx_irq: bool,
    ) -> TransferGuard<'spi, 'buf> {
        self.0.regs.write_interrupt_clear(InterruptClear::ALL);
        self.0
            .regs
            .write_interrupt_control(InterruptControl::ENABLE_ALL.with_tx(enable_tx_irq));
        self.0.regs.modify_ctrl1(|v| v.with_mtxpause(false));
        TransferGuard(self, PhantomData)
    }
}

impl embedded_hal_async::spi::ErrorType for Spi {
    type Error = RxOverrunError;
}

impl embedded_hal_async::spi::SpiBus for Spi {
    async fn read(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
        self.read(words).await
    }

    async fn write(&mut self, words: &[u8]) -> Result<(), Self::Error> {
        self.write(words).await
    }

    async fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), Self::Error> {
        self.transfer(read, write).await
    }

    async fn transfer_in_place(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
        self.transfer_in_place(words).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
