//! Asynchronous UART transmission functionality.
use core::{marker::PhantomData, sync::atomic::Ordering};

use embassy_sync::waitqueue::AtomicWaker;
use portable_atomic::{AtomicBool, AtomicPtr, AtomicUsize};

use crate::uart::{Bank, FIFO_DEPTH, disable_tx, disable_tx_interrupts, regs::Data};

static UART_TX_WAKERS: [AtomicWaker; 2] = [const { AtomicWaker::new() }; 2];
static TX_CONTEXTS: [TxContext; 2] = [const { TxContext::new() }; 2];
// Completion flag. Kept outside of the context structure as an atomic to avoid
// critical section.
static TX_DONE: [AtomicBool; 2] = [const { AtomicBool::new(false) }; 2];

#[inline]
fn tx_is_drained(tx: &crate::uart::Tx) -> bool {
    let tx_status = tx.regs.read_tx_status();
    tx.regs.read_state().tx_fifo().value() == 0 && !tx_status.tx_busy()
}

/// TX context structure. Plain atomics rather than a `critical_section::Mutex<RefCell<_>>` so it
/// can live in a `static` array directly and the interrupt handler does not need a critical
/// section.
///
/// `raw_data` doubles as the "transfer active" flag: it is always published last (`Release`)
/// after `transfer_len`/`progress`, and read first (`Acquire`) before them, so a reader that
/// observes it non-null is guaranteed to see the matching, not stale, `transfer_len`/`progress`.
struct TxContext {
    progress: AtomicUsize,
    raw_data: AtomicPtr<u8>,
    transfer_len: AtomicUsize,
}

impl TxContext {
    const fn new() -> Self {
        Self {
            progress: AtomicUsize::new(0),
            raw_data: AtomicPtr::new(core::ptr::null_mut()),
            transfer_len: AtomicUsize::new(0),
        }
    }
}

/// Interrupt handler for asynchronous UART TX operations for a given UART bank.
fn on_interrupt(bank: Bank) {
    let mut uart = unsafe { bank.steal_regs() };
    let idx = bank as usize;
    let irq_enabled = uart.read_interrupt_enable();
    // IRQ is not related to TX.
    if !irq_enabled.tx_below_trigger() && !irq_enabled.tx_empty() {
        return;
    }

    let context = &TX_CONTEXTS[idx];
    // `Acquire` pairs with the `Release` store in `Tx::start_transfer`: seeing a non-null pointer
    // here guarantees `transfer_len`/`progress` below are the values published together with
    // it, not stale ones from a previous transfer.
    let raw_data_ptr = context.raw_data.load(Ordering::Acquire);
    // No transfer active.
    if raw_data_ptr.is_null() {
        return;
    }

    let interrupt_status = uart.read_interrupt_status();

    let slice_len = context.transfer_len.load(Ordering::Relaxed);
    let mut progress = context.progress.load(Ordering::Relaxed);
    // Safety: The transfer is active, so the slice published with it is still valid.
    let slice = unsafe { core::slice::from_raw_parts(raw_data_ptr as *const u8, slice_len) };

    if progress >= slice_len && interrupt_status.tx_empty() {
        uart.modify_interrupt_enable(|value| {
            value
                .with_tx_below_trigger(false)
                .with_tx_empty(false)
                .with_tx_status(false)
        });
        uart.modify_enable(|value| value.with_tx(false));
        // Transfer is done. `Release` publishes the final `progress` to whichever context
        // observes `TX_DONE` via the `Acquire` swap in `poll_transfer_done`.
        TX_DONE[idx].store(true, Ordering::Release);
        UART_TX_WAKERS[idx].wake();
        return;
    }

    while progress < slice_len {
        if !uart.read_tx_status().ready() {
            break;
        }
        // Safety: TX structure is owned by the future which does not write into the data
        // register, so we can assume we are the only one writing to the data register.
        uart.write_data(Data::new_with_raw_value(slice[progress] as u32));
        progress += 1;
    }
    // Now we only require the TX empty interrupt.
    if progress == slice_len {
        uart.modify_interrupt_enable(|value| value.with_tx_below_trigger(false));
    }
    context.progress.store(progress, Ordering::Relaxed);
}

/// Asynchronous UART transmitter (TX) driver.
#[derive(Debug)]
pub struct Tx(crate::uart::Tx);

impl Tx {
    /// # Safety
    ///
    /// The user MUST ensure that the `Drop` method of all futures generated with this driver
    /// is called on transfer cancellation. By default, this does not require any special handling.
    /// Do *NOT* use `core::mem::forget` on a started future. The interrupt handler would keep
    /// reading from the buffer after it is gone.
    /// This case was considered exotic enough to not justify an `unsafe` API.
    pub fn new(tx: crate::uart::Tx) -> Self {
        Self(tx)
    }

    /// Token identifying the UART peripheral driven by this instance.
    ///
    /// Pass this to [Self::on_interrupt] to service the peripheral's TX interrupts. Since it is
    /// `Copy`, it can be stashed in a `Mutex<Cell<_>>` or similar and handed to the interrupt
    /// handler without needing access to the [Tx] instance itself.
    #[inline]
    pub fn bank_id(&self) -> Bank {
        self.0.id
    }

    /// Generic interrupt handler to handle asynchronous UART TX operations for a given UART
    /// bank.
    ///
    /// The user has to call this once in the interrupt handler responsible for the TX
    /// interrupts on the given UART bank. Takes the token returned by [Self::bank_id] rather
    /// than the [Tx] instance itself, so it can be called from interrupt context without needing
    /// access to the driver.
    pub fn on_interrupt(bank_id: Bank) {
        on_interrupt(bank_id);
    }

    /// Access the wrapped blocking driver.
    #[inline]
    pub fn inner(&mut self) -> &mut crate::uart::Tx {
        &mut self.0
    }

    /// Write a buffer asynchronously.
    ///
    /// This implementation is not side effect free, and a started future might have already
    /// written part of the passed buffer.
    pub async fn write(&mut self, buf: &[u8]) -> usize {
        if buf.is_empty() {
            return 0;
        }
        let id = self.bank_id();
        let _guard = self.start_transfer(buf);
        core::future::poll_fn(|cx| poll_transfer_done(id, cx)).await
    }

    /// The returned guard borrows `buf` and stops the transfer when dropped, because the
    /// interrupt handler reads from `buf` until then.
    fn start_transfer<'tx, 'buf>(&'tx mut self, buf: &'buf [u8]) -> TransferGuard<'tx, 'buf> {
        let idx = self.bank_id() as usize;
        TX_DONE[idx].store(false, Ordering::Relaxed);
        self.0.disable_interrupts();
        self.0.disable();
        self.0.clear_fifo();

        let init_fill_count = core::cmp::min(buf.len(), FIFO_DEPTH);
        // We fill the FIFO.
        for data in buf.iter().take(init_fill_count) {
            self.0
                .regs
                .write_data(Data::new_with_raw_value(*data as u32));
        }

        let context = &TX_CONTEXTS[idx];
        // Publish the guarded fields before marking the transfer active (`raw_data`) with
        // `Release`, so a reader that observes `raw_data` non-null via the `Acquire` load in
        // `on_interrupt` is guaranteed to see these too, rather than stale values from a previous
        // transfer.
        context.transfer_len.store(buf.len(), Ordering::Relaxed);
        context.progress.store(init_fill_count, Ordering::Relaxed);
        context
            .raw_data
            .store(buf.as_ptr().cast_mut(), Ordering::Release);

        // Ensure those are enabled together. Doing this before the transfer is marked active can
        // lead to weird glitches otherwise.
        self.0.enable_interrupts(
            buf.len() > FIFO_DEPTH,
            #[cfg(feature = "vor4x")]
            true,
        );
        self.0.enable();
        TransferGuard(self, PhantomData)
    }

    /// Write an entire buffer into this writer.
    ///
    /// Calls [Self::write] until all bytes were written.
    ///
    /// This function is not cancel-safe. If the future is dropped before it completes, some
    /// bytes might have already been written.
    pub async fn write_all(&mut self, buf: &[u8]) {
        // Operation is infallible, ignore error.
        let Ok(()) = <Self as embedded_io_async::Write>::write_all(self, buf).await;
    }

    /// Wait until all written data has actually left the FIFO.
    pub async fn flush(&mut self) {
        while !tx_is_drained(&self.0) {}
    }

    /// Release the wrapped blocking driver.
    pub fn release(self) -> crate::uart::Tx {
        self.0
    }
}

/// Returns the number of written bytes once the interrupt handler signals completion.
fn poll_transfer_done(bank: Bank, cx: &mut core::task::Context<'_>) -> core::task::Poll<usize> {
    let idx = bank as usize;
    UART_TX_WAKERS[idx].register(cx.waker());
    // `Acquire` pairs with the `Release` store in `on_interrupt`.
    if !TX_DONE[idx].swap(false, Ordering::Acquire) {
        return core::task::Poll::Pending;
    }
    let context = &TX_CONTEXTS[idx];
    // `Release`: pairs with the `Acquire` load in `on_interrupt`, so a spurious
    // interrupt for this slot after completion can never see a dangling pointer.
    context
        .raw_data
        .store(core::ptr::null_mut(), Ordering::Release);
    core::task::Poll::Ready(context.progress.load(Ordering::Relaxed))
}

/// Stops the transfer and marks it inactive on completion and on cancellation.
struct TransferGuard<'tx, 'buf>(&'tx mut Tx, PhantomData<&'buf [u8]>);

impl Drop for TransferGuard<'_, '_> {
    fn drop(&mut self) {
        let id = self.0.bank_id();
        let mut reg_block = unsafe { id.steal_regs() };
        disable_tx_interrupts(&mut reg_block);
        disable_tx(&mut reg_block);
        // Clear the buffer pointer so a spurious or future interrupt for this UART can never
        // dereference it. After completion, this was already done and is harmless.
        let context = &TX_CONTEXTS[id as usize];
        context.progress.store(0, Ordering::Relaxed);
        context
            .raw_data
            .store(core::ptr::null_mut(), Ordering::Release);
    }
}

impl embedded_io::ErrorType for Tx {
    type Error = core::convert::Infallible;
}

impl embedded_io_async::Write for Tx {
    /// Write a buffer asynchronously.
    ///
    /// This implementation is not side effect free, and a started future might have already
    /// written part of the passed buffer.
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        Ok(self.write(buf).await)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.flush().await;
        Ok(())
    }
}
