use core::{cell::Cell, marker::PhantomData};

use crate::i2c::{
    Address, FIFO_DEPTH, check_group_lengths, next_group, next_op, op_direction, op_len, regs,
};
use arbitrary_int::{u5, u11};
use embassy_sync::waitqueue::AtomicWaker;
use embedded_hal::i2c::Operation;
use portable_atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicUsize, Ordering};

#[cfg(feature = "vor1x")]
use crate::InterruptConfig;

/// Number of I2C peripherals.
#[cfg(feature = "vor1x")]
pub const NUM_I2C: usize = 2;
/// Number of I2C peripherals.
#[cfg(feature = "vor4x")]
pub const NUM_I2C: usize = 3;

static TRANSFER_CONTEXTS: [TransferContext; NUM_I2C] = [const { TransferContext::new() }; NUM_I2C];

/// Error condition observed by the interrupt handler, stored in [TransferContext::error].
///
/// Mirrors [super::Error], minus the payload on [super::Error::ClockTimeout]: `num_enum`'s
/// conversion derives only support field-less, C-like enums, and the timeout limit is cheap to
/// re-read from the register at the point [TransferContext::take_error] reconstructs the full
/// error, so there is no need to carry it through the atomic.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, num_enum::IntoPrimitive, num_enum::TryFromPrimitive,
)]
#[repr(u8)]
enum TransferErrorKind {
    /// No error occurred. The value [TransferContext::error] is reset to.
    None = 0,
    /// Arbitration was lost.
    ArbitrationLost = 1,
    /// Address not acknowledged.
    NackAddr = 2,
    /// Data not acknowledged in write operation.
    NackData = 3,
    /// The I2C clock was seen low for longer than the configured timeout.
    ClockTimeout = 4,
    /// The RX or TX FIFO overflowed.
    Overflow = 5,
    /// The controller went idle before the read buffer was filled.
    InsufficientDataReceived = 6,
}

/// Transfer context structure.
///
/// `ops_slice_ptr` doubles as the "transfer active" flag. It is published last (`Release`)
/// after the other fields and read first (`Acquire`) before them, so a reader which observes a
/// non-null pointer also sees the matching length and progress counters.
struct TransferContext {
    /// Type and lifetime erased pointer to the operations slice.
    ops_slice_ptr: AtomicPtr<()>,
    /// Total number of operations.
    ops_len: AtomicUsize,
    /// Index of the operation currently being transferred.
    ops_index: AtomicUsize,
    /// Bytes of the current operation already moved to or from the FIFO.
    op_progress: AtomicUsize,
    done: AtomicBool,
    waker: AtomicWaker,
    /// Set by the interrupt handler on an error condition, consumed by `take_error`.
    error: AtomicU8,
}

impl TransferContext {
    const fn new() -> Self {
        Self {
            ops_slice_ptr: AtomicPtr::new(core::ptr::null_mut()),
            ops_len: AtomicUsize::new(0),
            ops_index: AtomicUsize::new(0),
            op_progress: AtomicUsize::new(0),
            done: AtomicBool::new(false),
            waker: AtomicWaker::new(),
            error: AtomicU8::new(TransferErrorKind::None as u8),
        }
    }

    /// Stores the operations and arms the gate, starting at operation `first`.
    ///
    /// # Safety
    ///
    /// `operations` must stay alive and must not be accessed otherwise until the context is
    /// disarmed.
    unsafe fn arm(&self, operations: &mut [Operation<'_>], first: usize, progress: usize) {
        self.ops_len.store(operations.len(), Ordering::Relaxed);
        self.ops_index.store(first, Ordering::Relaxed);
        self.op_progress.store(progress, Ordering::Relaxed);
        self.ops_slice_ptr
            .store(operations.as_mut_ptr().cast(), Ordering::Release);
    }

    /// Returns the operations of the active transfer, or [None] if the gate is disarmed.
    ///
    /// # Safety
    ///
    /// Call at most once per interrupt handler call, and do not let the slice or anything
    /// borrowed from it outlive that call.
    unsafe fn operations<'a>(&self) -> Option<&'a mut [Operation<'a>]> {
        let ptr = self
            .ops_slice_ptr
            .load(Ordering::Acquire)
            .cast::<Operation<'a>>();
        if ptr.is_null() {
            return None;
        }
        Some(unsafe { core::slice::from_raw_parts_mut(ptr, self.ops_len.load(Ordering::Relaxed)) })
    }

    /// Disarms the gate, so a spurious interrupt can never observe the stale operations.
    #[inline]
    fn disarm(&self) {
        self.ops_slice_ptr
            .store(core::ptr::null_mut(), Ordering::Release);
    }

    /// Records an error condition, to be observed by `take_error`.
    #[inline]
    fn set_error(&self, kind: TransferErrorKind) {
        self.error.store(kind.into(), Ordering::Relaxed);
    }

    /// Takes (and clears) the recorded error condition, if any.
    ///
    /// For [TransferErrorKind::ClockTimeout], the caller must still read the timeout limit
    /// itself to build a full [super::Error::ClockTimeout] value.
    #[inline]
    fn take_error(&self) -> Option<TransferErrorKind> {
        let raw = self
            .error
            .swap(TransferErrorKind::None as u8, Ordering::Relaxed);
        match TransferErrorKind::try_from(raw) {
            Ok(TransferErrorKind::None) | Err(_) => None,
            Ok(kind) => Some(kind),
        }
    }

    /// Marks the transfer as finished and wakes the registered waker.
    #[inline]
    fn signal_done(&self) {
        self.done.store(true, Ordering::Release);
        self.waker.wake();
    }

    /// Registers the waker and consumes the completion flag.
    #[inline]
    fn poll_done(&self, waker: &core::task::Waker) -> bool {
        self.waker.register(waker);
        self.done.swap(false, Ordering::Acquire)
    }

    /// Closes the gate and restores the initial state, so the slot can be reused.
    #[inline]
    fn reset(&self) {
        // Must come first in program order: a live interrupt (e.g. during Drop's
        // cancellation path) must never see the fields below cleared while still
        // observing an armed transfer.
        self.disarm();
        self.done.store(false, Ordering::Relaxed);
        self.error
            .store(TransferErrorKind::None as u8, Ordering::Relaxed);
    }
}

/// Moves bytes between the FIFO and `op`, starting at `progress`. Returns the new progress.
fn pump_or_drain_fifo(
    regs: &mut regs::MmioRegisters<'static>,
    op: &mut Operation<'_>,
    mut progress: usize,
) -> usize {
    match op {
        Operation::Read(buf) => {
            while progress < buf.len() && regs.read_status().rx_not_empty() {
                buf[progress] = regs.read_data().data();
                progress += 1;
            }
        }
        Operation::Write(buf) => {
            while progress < buf.len() && regs.read_status().tx_not_full() {
                regs.write_data(regs::Data::new(buf[progress]));
                progress += 1;
            }
        }
    }
    progress
}

/// Bytes of the group containing `op_idx` that are not yet moved to or from the FIFO.
fn group_remaining(ops: &[Operation<'_>], op_idx: usize, progress: usize) -> usize {
    let dir = op_direction(&ops[op_idx]);
    let mut remaining = op_len(&ops[op_idx]) - progress;
    let mut i = next_op(ops, op_idx + 1);
    while let Some(idx) = i {
        if op_direction(&ops[idx]) != dir {
            break;
        }
        remaining += op_len(&ops[idx]);
        i = next_op(ops, idx + 1);
    }
    remaining
}

/// Sets the RX trigger level for the bytes still to come. A level above them would never be
/// reached, so the last chunk lowers it to exactly what remains.
fn set_rx_trigger(regs: &mut regs::MmioRegisters<'static>, remaining: usize) {
    let level = remaining.clamp(1, FIFO_DEPTH / 2);
    regs.write_rx_fifo_trigger(regs::TriggerLevel::new(u5::new(level as u8)));
}

/// Programs the word count and direction for the group starting at `first`, fills the TX FIFO
/// for a write group and issues the command. Returns the interrupts the group needs, and the
/// operation index and progress where the pre-fill stopped.
///
/// The caller enables the interrupts after the command was issued. Enabling `idle` before
/// that would let the still idle bus trigger the completion path of a group which has not
/// started yet.
///
/// `idle` and `waiting` are only enabled once all bytes of the group are moved. On the VA108xx,
/// an enabled `idle` also fires during a receive group, about once per byte.
fn start_group(
    regs: &mut regs::MmioRegisters<'static>,
    ops: &mut [Operation<'_>],
    first: usize,
) -> (regs::InterruptControl, usize, usize) {
    let dir = op_direction(&ops[first]);
    let mut len = 0;
    // Assume this is the last group until a direction change is found.
    let mut command = super::Command::StartWithStop;
    let mut i = Some(first);
    while let Some(idx) = i {
        if op_direction(&ops[idx]) != dir {
            command = super::Command::Start;
            break;
        }
        len += op_len(&ops[idx]);
        i = next_op(ops, idx + 1);
    }
    // A zero-length write group is still sent. It addresses the target without data, which
    // is a valid way to probe for a device.
    regs.write_words(regs::Words::new(u11::new(len as u16)));
    regs.modify_address(|val| val.with_direction(dir));
    let receive = dir == regs::Direction::Receive;
    let mut op_idx = first;
    let mut progress = 0;
    // The hardware sees the group as one byte stream, so the pre-fill can cross operation
    // boundaries. It stops when the FIFO is full or the group ends.
    if !receive {
        loop {
            progress = pump_or_drain_fifo(regs, &mut ops[op_idx], 0);
            if progress < op_len(&ops[op_idx]) {
                break;
            }
            match next_op(ops, op_idx + 1) {
                Some(next) if op_direction(&ops[next]) == dir => op_idx = next,
                _ => break,
            }
        }
    }
    let remaining = group_remaining(ops, op_idx, progress);
    if receive {
        set_rx_trigger(regs, remaining);
    }
    regs.write_command(command.reg_value());

    let interrupts = regs::InterruptControl::builder()
        // Error conditions.
        .with_clock_timeout(true)
        .with_rx_overflow(receive)
        .with_tx_overflow(!receive)
        .with_arb_lost(true)
        .with_nack_addr(true)
        .with_nack_data(!receive)
        // FIFO drain and re-fill conditions.
        .with_rx_ready(receive)
        .with_tx_ready(!receive && remaining > 0)
        // Done status. Groups followed by another one end in `waiting`, the last one in `idle`.
        .with_idle(remaining == 0 && command == super::Command::StartWithStop)
        .with_waiting(remaining == 0 && command == super::Command::Start)
        // Not needed by the driver. On the VA108xx it can fire about once per byte during a
        // read, although neither FIFO condition from the reference manual applies.
        .with_stalled(false)
        // Unused.
        .with_i2c_idle(false)
        .with_tx_empty(false)
        .with_rx_full(false)
        .build();
    (interrupts, op_idx, progress)
}

/// Cancels the transfer after an error and wakes the future.
fn abort(
    regs: &mut regs::MmioRegisters<'static>,
    context: &TransferContext,
    error: TransferErrorKind,
) {
    context.set_error(error);
    regs.write_interrupt_enable(regs::InterruptControl::ZERO);
    // Ends the transaction on the bus. Necessary because groups started with a plain `Start`
    // do not self-terminate the way `StartWithStop` does.
    regs.write_command(super::Command::Cancel.reg_value());
    context.signal_done();
}

/// Async I2C driver, built on top of the blocking [I2cMaster](super::I2cMaster).
pub struct I2c(super::I2cMaster);

impl I2c {
    /// Construct an asynchronous I2C driver for the given I2C peripheral.
    ///
    /// # Safety
    ///
    /// The user MUST ensure that the `Drop` method of all futures generated with this driver
    /// is called on transfer cancellation. By default, this does not require any special
    /// handling. This case was considered exotic enough to justify not making the function
    /// `unsafe`.
    pub fn new(
        mut i2c: super::I2cMaster,
        #[cfg(feature = "vor1x")] opt_irq_cfg: Option<InterruptConfig>,
    ) -> Self {
        i2c.regs
            .write_interrupt_enable(regs::InterruptControl::ZERO);
        i2c.regs.write_interrupt_clear(regs::InterruptClear::ALL);
        // Half the FIFO depth for both directions leaves room for interrupt latency. A zero RX
        // trigger level would keep `rx_ready` firing even with an empty FIFO.
        i2c.regs
            .write_rx_fifo_trigger(regs::TriggerLevel::new(u5::new(FIFO_DEPTH as u8 / 2)));
        i2c.regs
            .write_tx_fifo_trigger(regs::TriggerLevel::new(u5::new(FIFO_DEPTH as u8 / 2)));
        #[cfg(feature = "vor1x")]
        if let Some(irq_cfg) = opt_irq_cfg {
            if irq_cfg.route {
                crate::enable_peripheral_clock(crate::PeripheralSelect::Irqsel);
                unsafe { va108xx::Irqsel::steal() }
                    .i2c_ms(i2c.id as usize)
                    .write(|w| unsafe { w.bits(irq_cfg.id as u32) });
            }
            if irq_cfg.enable_in_nvic {
                // Safety: User has specifically configured this.
                unsafe { crate::enable_nvic_interrupt(irq_cfg.id) };
            }
        }
        // Unlike vor1x, vor4x has a fixed interrupt vector per bank instead of an IRQSEL mux,
        // so there is no routing decision to make: always enable it.
        #[cfg(feature = "vor4x")]
        unsafe {
            crate::enable_nvic_interrupt(i2c.id.interrupt_id_master());
        }
        Self(i2c)
    }

    /// Interrupt handler for the given I2C bank.
    ///
    /// Call this from the bank's interrupt vector. Does nothing if no async transfer is active.
    ///
    /// Returns the live status observed on this call. The driver only treats clock timeouts,
    /// arbitration loss, NACKs and FIFO overflows as transfer errors. Other bits, like
    /// `stalled`, are not surfaced as an [super::Error] variant and do not raise an interrupt of
    /// their own. They are still in the returned value of each call.
    ///
    /// # Safety
    ///
    /// Only call this from the interrupt handler of the given bank, and only once per handler
    /// call. Concurrent calls for the same bank would access the transfer buffers at the same
    /// time.
    pub unsafe fn on_interrupt(bank_id: super::Bank) -> regs::Status {
        let mut regs = unsafe { bank_id.steal_regs() };
        let context = &TRANSFER_CONTEXTS[bank_id as usize];

        let interrupt_status = regs.read_interrupt_status();
        regs.write_interrupt_clear(regs::InterruptClear::ALL);
        let status = regs.read_status();

        // Safety: Only called once, and the slice does not leave this function.
        let Some(operations) = (unsafe { context.operations() }) else {
            // Disable interrupts if there is no active transfer to avoid an interrupt loop.
            regs.write_interrupt_enable(regs::InterruptControl::ZERO);
            return status;
        };
        let mut op_index = context.ops_index.load(Ordering::Relaxed);
        let mut progress = context.op_progress.load(Ordering::Relaxed);

        if interrupt_status.clock_timeout() {
            abort(&mut regs, context, TransferErrorKind::ClockTimeout);
            return status;
        }
        if status.arb_lost() {
            abort(&mut regs, context, TransferErrorKind::ArbitrationLost);
            return status;
        }
        if status.nack_addr() {
            abort(&mut regs, context, TransferErrorKind::NackAddr);
            return status;
        }
        // On a receive, the master NACKs the last byte itself to end the transfer, which sets
        // the same bit, so only writes are checked. On a write, the embedded-hal contract
        // expects every byte to be acknowledged, including the last one.
        if status.nack_data() && op_direction(&operations[op_index]) == regs::Direction::Send {
            abort(&mut regs, context, TransferErrorKind::NackData);
            return status;
        }
        // Normally the bus should stall before RX overflows and the logic should ensure a full
        // FIFO is never written, but these checks are kept as a safety net.
        if interrupt_status.rx_overflow() || interrupt_status.tx_overflow() {
            abort(&mut regs, context, TransferErrorKind::Overflow);
            return status;
        }

        loop {
            // Sampled before moving bytes: once the controller is done, all received bytes are
            // already in the FIFO.
            let live_status = regs.read_status();
            progress = pump_or_drain_fifo(&mut regs, &mut operations[op_index], progress);
            let dir = op_direction(&operations[op_index]);
            let opt_next_index = next_op(operations, op_index + 1);
            let opt_next_group = next_group(operations, op_index);
            // Only the last group ends in `idle`. The others end in `waiting`, which the last
            // group never sets, even when the current operation is not the last one.
            let group_done_condition = if opt_next_group.is_none() {
                live_status.idle()
            } else {
                live_status.waiting()
            };

            // We moved bytes to or from the FIFO, and the hardware still needs to do work.
            // So we break out of the loop.
            if progress < op_len(&operations[op_index]) {
                // Special case: We are done but did not receive all bytes for whatever reason.
                if group_done_condition && dir == regs::Direction::Receive {
                    abort(
                        &mut regs,
                        context,
                        TransferErrorKind::InsufficientDataReceived,
                    );
                    return status;
                }
                if dir == regs::Direction::Receive {
                    set_rx_trigger(&mut regs, group_remaining(operations, op_index, progress));
                }
                break;
            }
            // Everything after here: progress is equal to operations length and we either have
            // to wait for the done flag or start a new operation.

            // Handling for consecutive operations. The continue advances to the next operation.
            if let Some(next_index) = opt_next_index
                && op_direction(&operations[next_index]) == dir
            {
                op_index = next_index;
                progress = 0;
                continue;
            }
            // When we reach this point, all bytes of the group are moved. `tx_ready` and
            // `rx_ready` are level interrupts, so they have to be disabled until the group is
            // done. Only now the done status is enabled, see [start_group].
            //
            // Only enable it while the group is still running. On an already idle controller,
            // even a short enable pends the next interrupt. That interrupt finds the transfer
            // still armed and enables it again, so the task that disarms it never runs.
            if !group_done_condition {
                regs.modify_interrupt_enable(|val| {
                    val.with_tx_ready(false)
                        .with_rx_ready(false)
                        .with_idle(opt_next_group.is_none())
                        .with_waiting(opt_next_group.is_some())
                });
                break;
            }
            // At this point, we have to finish the last operation or start the next one.
            match opt_next_group {
                Some(next) => {
                    let interrupts;
                    (interrupts, op_index, progress) = start_group(&mut regs, operations, next);
                    regs.write_interrupt_enable(interrupts);
                }
                None => {
                    regs.write_interrupt_enable(regs::InterruptControl::ZERO);
                    context.signal_done();
                }
            }
            // The group is finished, so leave. Only the `continue` above repeats the loop. The
            // next interrupt picks up the new group, because the status sampled here can still
            // show the end of the previous one.
            break;
        }
        context.ops_index.store(op_index, Ordering::Relaxed);
        context.op_progress.store(progress, Ordering::Relaxed);
        status
    }

    /// Start an async transaction, returning a future which completes once all operations
    /// were performed.
    ///
    /// Consecutive operations of the same direction are merged into one hardware transfer.
    /// Empty reads are skipped. Empty writes address the target without data, which can be used
    /// to probe for a device.
    pub fn transaction<'ops>(
        &mut self,
        address: u8,
        operations: &'ops mut [Operation<'_>],
    ) -> Result<Transfer<'_, 'ops>, super::Error> {
        check_group_lengths(operations)?;
        let context = &TRANSFER_CONTEXTS[self.0.id as usize];
        let Some(first) = next_op(operations, 0) else {
            context.signal_done();
            return Ok(Transfer::new(self));
        };
        self.0.disable_interrupts();
        self.0.regs.write_interrupt_clear(regs::InterruptClear::ALL);
        self.0.clear_tx_fifo();
        self.0.clear_rx_fifo();

        self.0.write_address_and_direction(
            Address::Regular(address),
            op_direction(&operations[first]),
        );
        let (interrupts, op_index, progress) = start_group(&mut self.0.regs, operations, first);
        // Safety: The returned transfer borrows `operations` until it completes or is dropped.
        // Users are not allowed to forget transfers, see the safety note on [Self::new].
        unsafe { context.arm(operations, op_index, progress) };
        self.0.regs.write_interrupt_enable(interrupts);

        Ok(Transfer::new(self))
    }
}

/// Inherent versions of the [embedded_hal_async::i2c::I2c] methods, so they can be used without
/// importing the trait.
impl I2c {
    /// Async read transaction.
    pub async fn read(&mut self, address: u8, buf: &mut [u8]) -> Result<(), super::Error> {
        embedded_hal_async::i2c::I2c::read(self, address, buf).await
    }

    /// Async write transaction.
    pub async fn write(&mut self, address: u8, data: &[u8]) -> Result<(), super::Error> {
        embedded_hal_async::i2c::I2c::write(self, address, data).await
    }

    /// Async write-read transaction.
    pub async fn write_read(
        &mut self,
        address: u8,
        write: &[u8],
        read: &mut [u8],
    ) -> Result<(), super::Error> {
        embedded_hal_async::i2c::I2c::write_read(self, address, write, read).await
    }
}

impl embedded_hal_async::i2c::I2c for I2c {
    #[inline]
    async fn transaction(
        &mut self,
        address: u8,
        operations: &mut [Operation<'_>],
    ) -> Result<(), Self::Error> {
        self.transaction(address, operations)?.await
    }
}

impl embedded_hal_async::i2c::ErrorType for I2c {
    type Error = super::Error;
}

/// Live I2C transfer returned by [I2c::transaction].
///
/// Implements [Future] and can be polled/awaited to completion.
pub struct Transfer<'d, 'ops> {
    driver: &'d mut I2c,
    finished_regularly: Cell<bool>,
    /// The interrupt handler accesses the operations until the transfer completes or is dropped.
    _operations: PhantomData<&'ops mut ()>,
}

impl<'d> Transfer<'d, '_> {
    fn new(driver: &'d mut I2c) -> Self {
        Self {
            driver,
            finished_regularly: Cell::new(false),
            _operations: PhantomData,
        }
    }
}

impl core::future::Future for Transfer<'_, '_> {
    type Output = Result<(), super::Error>;

    fn poll(
        self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        let context = &TRANSFER_CONTEXTS[self.driver.0.id() as usize];
        if !context.poll_done(cx.waker()) {
            return core::task::Poll::Pending;
        }
        self.finished_regularly.set(true);
        // Read the error before resetting: `reset` clears it too, so it must happen last.
        let error = context.take_error();
        context.reset();
        let Some(error) = error else {
            return core::task::Poll::Ready(Ok(()));
        };
        let transfer_error = match error {
            TransferErrorKind::None => return core::task::Poll::Ready(Ok(())),
            TransferErrorKind::ArbitrationLost => super::Error::ArbitrationLost,
            TransferErrorKind::NackAddr => super::Error::NackAddr,
            TransferErrorKind::NackData => super::Error::NackData,
            TransferErrorKind::ClockTimeout => {
                super::Error::ClockTimeout(self.driver.0.regs.read_clk_timeout_limit().value())
            }
            TransferErrorKind::Overflow => super::Error::Overflow,
            TransferErrorKind::InsufficientDataReceived => super::Error::InsufficientDataReceived,
        };
        core::task::Poll::Ready(Err(transfer_error))
    }
}

impl Drop for Transfer<'_, '_> {
    fn drop(&mut self) {
        if !self.finished_regularly.get() {
            // Disarm before disabling the interrupts. An interrupt already pending in the NVIC
            // would otherwise still see the armed transfer, start the next group and re-enable
            // the interrupts.
            let context = &TRANSFER_CONTEXTS[self.driver.0.id() as usize];
            context.reset();
            self.driver.0.disable_interrupts();
            self.driver
                .0
                .regs
                .write_interrupt_clear(regs::InterruptClear::ALL);
            self.driver.0.cancel_transfer();
            self.driver.0.clear_tx_fifo();
            self.driver.0.clear_rx_fifo();
        }
    }
}
