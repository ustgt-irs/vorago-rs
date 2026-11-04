use core::sync::atomic::Ordering;

use embassy_sync::waitqueue::AtomicWaker;
use portable_atomic::{AtomicBool, AtomicPtr, AtomicUsize};

/// Buffer-tracking and completion-signaling state for an async transfer, used by the SPI driver.
///
/// Plain atomics rather than a `critical_section::Mutex<RefCell<_>>` so it can live in a
/// `static` array directly and the interrupt handler does not need a critical section.
///
/// This does not include the transfer-type gate itself: the driver keeps that as its own
/// `AtomicU8` field alongside a `TransferState`.
pub(crate) struct TransferState {
    /// Progress counter for the TX side of the transfer, in words.
    pub(crate) tx_progress: AtomicUsize,
    /// Progress counter for the RX side of the transfer, in words.
    pub(crate) rx_progress: AtomicUsize,
    tx_ptr: AtomicPtr<u8>,
    tx_len: AtomicUsize,
    rx_ptr: AtomicPtr<u8>,
    rx_len: AtomicUsize,
    /// Completion flag, set by the interrupt handler and consumed by `poll_done`.
    done: AtomicBool,
    waker: AtomicWaker,
}

impl TransferState {
    pub(crate) const fn new() -> Self {
        Self {
            tx_progress: AtomicUsize::new(0),
            rx_progress: AtomicUsize::new(0),
            tx_ptr: AtomicPtr::new(core::ptr::null_mut()),
            tx_len: AtomicUsize::new(0),
            rx_ptr: AtomicPtr::new(core::ptr::null_mut()),
            rx_len: AtomicUsize::new(0),
            done: AtomicBool::new(false),
            waker: AtomicWaker::new(),
        }
    }

    /// Clears the completion flag before a new transfer starts.
    #[inline]
    pub(crate) fn clear_done(&self) {
        self.done.store(false, Ordering::Relaxed);
    }

    /// Marks the transfer as finished and wakes the registered waker.
    ///
    /// `Release` publishes the completed transfer state to whichever context observes `done`
    /// via the `Acquire` swap in `poll_done`.
    #[inline]
    pub(crate) fn signal_done(&self) {
        self.done.store(true, Ordering::Release);
        self.waker.wake();
    }

    /// Reads the completion flag without consuming it.
    #[inline]
    pub(crate) fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    /// Registers the waker and consumes the completion flag.
    ///
    /// `Acquire` pairs with the `Release` store in `signal_done`.
    #[inline]
    pub(crate) fn poll_done(&self, waker: &core::task::Waker) -> bool {
        self.waker.register(waker);
        self.done.swap(false, Ordering::Acquire)
    }

    /// # Safety
    ///
    /// The caller must ensure the slice outlives the transfer.
    #[inline]
    pub(crate) unsafe fn set_tx_slice(&self, data: &[u8]) {
        self.tx_ptr
            .store(data.as_ptr().cast_mut(), Ordering::Relaxed);
        self.tx_len.store(data.len(), Ordering::Relaxed);
    }

    /// # Safety
    ///
    /// The caller must ensure the slice outlives the transfer.
    #[inline]
    pub(crate) unsafe fn set_rx_slice(&self, data: &mut [u8]) {
        self.rx_ptr.store(data.as_mut_ptr(), Ordering::Relaxed);
        self.rx_len.store(data.len(), Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn clear_tx_slice(&self) {
        self.tx_ptr.store(core::ptr::null_mut(), Ordering::Relaxed);
        self.tx_len.store(0, Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn clear_rx_slice(&self) {
        self.rx_ptr.store(core::ptr::null_mut(), Ordering::Relaxed);
        self.rx_len.store(0, Ordering::Relaxed);
    }

    /// # Safety
    ///
    /// Only valid while the transfer which published the slice is still active.
    #[inline]
    pub(crate) unsafe fn tx_slice(&self) -> &'static [u8] {
        let ptr = self.tx_ptr.load(Ordering::Relaxed);
        if ptr.is_null() {
            return &[];
        }
        unsafe {
            core::slice::from_raw_parts(ptr as *const u8, self.tx_len.load(Ordering::Relaxed))
        }
    }

    /// # Safety
    ///
    /// Only valid while the transfer which published the slice is still active. The caller must
    /// not create a second alias to the same buffer.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub(crate) unsafe fn rx_slice(&self) -> &'static mut [u8] {
        let ptr = self.rx_ptr.load(Ordering::Relaxed);
        if ptr.is_null() {
            return &mut [];
        }
        unsafe { core::slice::from_raw_parts_mut(ptr, self.rx_len.load(Ordering::Relaxed)) }
    }

    /// Clears progress counters and buffer slices, so the slot can be reused.
    ///
    /// Does not touch the transfer-type gate: callers must disarm that themselves, before
    /// calling this, so a live interrupt can never see these fields cleared while still
    /// observing an armed transfer.
    #[inline]
    pub(crate) fn reset(&self) {
        self.tx_progress.store(0, Ordering::Relaxed);
        self.rx_progress.store(0, Ordering::Relaxed);
        self.clear_tx_slice();
        self.clear_rx_slice();
    }
}
