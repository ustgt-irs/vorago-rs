Change Log
=======

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/)
and this project adheres to [Semantic Versioning](http://semver.org/).

## [unreleased]

### Added

- Re-export `MODE_0` to `MODE_3` from `spi`.
- I2C write,read transcation and write_read are now inherent method
  usable without needing to import the trait
- `From<u8>` and `From<u16>` for `i2c::Address`.
- `i2c::MAX_WORD_COUNT`, the maximum length of one I2C transfer.
- `i2c::regs::DataCount::value` to read the TXCOUNT and RXCOUNT registers.

### Changed

- I2C contract fixes and blocking module update
- Consecutive operations of the same direction are now merged and done in one hardware transaction
- Renamed `i2c::I2cAddress` to `Address` and `i2c::I2cCommand` to `Command`.
- Renamed `I2cMaster::write_address` to `write_address_and_direction`.
- Replaced `I2cMaster::write_blocking`, `read_blocking` and `write_read_blocking` with `write`,
  `read` and `write_read`. They take the address type of the master (`u8` or `u16`) instead of
  `I2cAddress`.
- The async SPI driver's interrupt handler no longer takes a critical section on every
  interrupt. The shared transfer state moved from a `Mutex<RefCell<TransferContext>>` to plain
  atomics, gated by an `Acquire`/`Release` flag that publishes the transfer buffers. The waker
  and completion flag, previously separate per-bank statics, are now plain atomic fields on
  `TransferContext` as well.
- `SpiAsync` was renamed to `asynch::Spi`. Its `on_interrupt` free function became
  `Spi::on_interrupt`. It now takes a `Bank` token obtained from the new `Spi::bank_id` instead
  of requiring access to the driver instance, so it can be called from an interrupt handler that
  only has a token, not the driver.
- Added `Spi::into_async` on the blocking driver as a shortcut for `asynch::Spi::new`.
- `SpiFuture` was renamed to `asynch::Transfer`. The old name is kept as a deprecated type alias.
- `SpiConfig` was renamed to `Config` and reworked: it is now `#[non_exhaustive]` with public
  fields (`clock`, `mode`, `blockmode`, ...) instead of a builder-method API, plus a
  `Config::new(mode, clock)` constructor for the two fields without a sensible default.
  `SpiClockConfig` was renamed to `ClockConfig`.
- The async SPI driver now always enables blockmode and blockmode stalling. It marks the last
  word of a transfer with the BMSTART_BMSTOP bit, which ends the frame and deasserts a hardware
  chip select.
- The blocking SPI driver now always enables blockmode and blockmode stalling as well. Removed
  `blockmode` and `bmstall` from `Config` and `TransferConfig`, and the matching parameters of
  `TransferConfig::new_with_hw_cs`.
- Blocking SPI transfers now pre-fill the whole FIFO and keep at most `FIFO_DEPTH` words unread,
  so the RX FIFO can not overrun.
- Blocking SPI `transfer` now sends the fill word instead of 0 when `write` is shorter than `read`.
- `SpiWord` no longer requires `TryFrom<u32>`. It has a new `from_raw` method instead.
- The async UART TX driver's interrupt handler no longer takes a critical section on every
  interrupt either. The shared transfer state moved from a `Mutex<RefCell<TxContext>>` to plain
  atomics, gated the same way as the SPI driver. This also drops the `raw-buffer` dependency.
- `TxAsync` was moved into a new `uart::asynch` module and renamed to `asynch::Tx`. Its free
  `on_interrupt_tx` function became `Tx::on_interrupt`, taking a `Bank` token from the new
  `Tx::bank_id` instead of requiring access to the driver instance.
- `asynch::Tx::new` and `Tx::into_async` are safe again. The requirement that the `Drop` handler
  of generated futures runs is documented instead. The corner case is exotic enough to not
  justify an `unsafe` API.
- Added `core::fmt::Write` for the blocking `uart::Tx`, so `write!`/`writeln!` work directly on
  it.
- `RxAsync` and `RxAsyncOverwriting` were removed, along with the `heapless` dependency they
  pulled in. Async UART RX never fit the same one-shot-future shape as TX: waiting for arbitrary
  incoming data is a queue's job, not the UART peripheral's. Use `RxWithInterrupt::on_interrupt`/
  `RxWithInterrupt::on_interrupt_owned` to drain the RX FIFO into a buffer from your own
  interrupt handler, and forward the bytes into a queue of your choice (e.g. an
  `embassy_sync::pipe::Pipe`, which already gives you an async `read`). See the `uart::asynch`
  module docs and the `async-uart-rx` examples for the pattern.
- `RxWithInterrupt::on_interrupt` now takes a `Bank` token (from the new
  `RxWithInterrupt::bank_id`) instead of requiring an owned instance, so it can be called from a
  bare interrupt handler without stashing the driver in a `Mutex<RefCell<Option<_>>>`. The
  previous `&mut self` form is still available as `RxWithInterrupt::on_interrupt_owned`, for
  owned-instance use (e.g. an RTIC `local` resource).
- Added an async I2C driver in a new `i2c::asynch` module: `I2cMaster::into_async`/
  `asynch::I2c::new` construct it, `I2c::transaction` returns an `asynch::Transfer` future, and
  the unsafe `I2c::on_interrupt` services it from the peripheral's interrupt vector. `I2c::read`/`write`/
  `write_read` are async methods built on top of `transaction`. Like the blocking driver, it
  merges consecutive operations of the same direction into one hardware transfer.
- Added `Error::Overflow` for I2C, reported when the RX or TX FIFO overflows during a transfer.
- Several I2C register accessors were renamed for consistency: `cmd` became `command`, and
  `irq_enb`/`irq_raw`/`irq_status`/`irq_clear` became `interrupt_enable`/`interrupt_raw`/
  `interrupt_status`/`interrupt_clear`.
- Removed `tx_empty_mode` and `rx_full_mode` from the I2C `MasterConfig`. The master now always
  stalls on an empty TX or full RX FIFO, which is required to meet the embedded-hal transaction
  contract. `TxFifoEmptyMode` and `RxFifoFullMode` are no longer re-exported from `i2c`.

### Fixed

- The embassy time driver no longer divides a 64-bit value in `now()`. That division is slow
  in software on Cortex-M0, and its fast path ended with the first timekeeper overflow, after
  roughly 86 s at 50 MHz. A timekeeper period is now a whole number of embassy ticks.
- Alarms are no longer triggered early at every timekeeper overflow. `next_period` compared
  timer ticks with embassy ticks.
- The embassy time driver now rejects an invalid `TICK_HZ` instead of dividing by zero. A
  `TICK_HZ` that does not fit in a `u32` is a compile error. A `TICK_HZ` higher than the timer
  clock panics at init.
- Hardware chip select is now deasserted at the end of an async SPI transfer. Previously the
  async driver disabled blockmode and never set the BMSTART_BMSTOP bit, so CS stayed asserted.
- Cancelling an async SPI transfer now ends the blockmode frame. Dropping a transfer future only
  cleared the FIFOs, which leaves the frame open and the chip select asserted.
- Fixed the I2C blocking module to properly implement the embedded HAL I2C trait contract.
- I2C empty reads are skipped
- An I2C clock timeout now cancels the transfer and clears the FIFO
- I2C `DataTooLarge` is now checked before any bus activity.
- The I2C bus is now released after an error in the middle of a transaction.
- Blocking SPI transfers longer than 12 words are no longer split into two blockmode frames.
- Blocking SPI `transfer` with `read` longer than `write` now ends the frame after the last word.
  An empty `write` no longer panics.

## [v0.5.0] 2026-07-14

### Changed

- Async TX UART `write` now returns a `TxFuture`
- Empty async TX write resolves to `Poll::Ready(0)` immediately.
- Async SPI API now always returns futures instead of optional futures.

### Fixed

- Asynch drivers now borrow the buffers properly for the lifetime of the future.
- Asynch UART TX driver now borrows the TX peripheral for the duration of the future.

## [v0.4.0] 2026-05-19

### Changed

- Naming improvements for UART register module
- Improved UART Async TX module. Only enable TX below threshold interrupts if the FIFO
  actually needs to be refilled.

## [v0.3.0] 2026-05-18

### Added

- Add `is_high` and `is_low` for `InputPinAsync`.
- Add `InputPin` impl for `InputPinAsync`.
- `HwCsPin` in SPI module for easer usage of HW CS pins as `Output` CS pins

### Changed

- Bumped `fugit` from v0.3 to v0.4
- Added `RxWithInterrupt::steal`.
- Renamed UART `Data` register `value` field to `data`
- Improved type level support for resource management for SPI, PWM, UART.
- Renamed `tx_asynch` and `rx_asynch` module name to `*_async`
- Naming improvements in SPI module: replaced `cfg` by `config*`
- UART configuration now expects an explicit clock configuration structure and does not
  calculate it itself anymore.

### Fixed

- `Pull::Up` and `Pull::High` were inverted.
- Removed HW CS pin provider implementation for PA23, PA22 and PA21, which are multi HW CS pins.
- Added missing `AnyPin` trait impl for Multi HW CS pins.
- Expose inner `Input` pin for `InputPinAsync`.
- Bugfix for UART clock calculation with 8x baud mode.
- Possible bugfix for Asynch GPIO where the interrupt handler could become stuck in a loop.
- Robustness improvements for the Asynch GPIO driver code.

## [v0.2.0] 2025-09-03

Renamed to `vorago-shared-hal`

### Changed

- Various renaming to be more in-line with common Embedded Rust naming conventions.
  - `PinId` -> `DynPinId`
  - `PinIdProvider` -> `PinId`
  - `FunSel` -> `FunctionSelect`
  - `PinMarker` -> `AnyPin`
  - Peripheral traits renamed from `*Marker` to `*Instance`
  - `Clk` abbreviation in names changed to `Clock`
  - `Cmd` abbreviation in names changed to `Command`
  - `Irq` abbreviation in names changed to `Interrupt`

## [v0.1.0] 2025-09-02

Init commit.

[unreleased]: https://github.com/ustgt-irs/vorago-rs/compare/vorago-shared-hal-v0.5.0...HEAD
[v0.5.0]: https://github.com/ustgt-irs/vorago-rs/releases/tag/vorago-shared-hal-v0.5.0
[v0.4.0]: https://github.com/ustgt-irs/vorago-rs/compare/vorago-shared-hal-v0.3.0...vorago-shared-hal-v0.4.0
[v0.3.0]: https://github.com/ustgt-irs/vorago-rs/releases/tag/vorago-shared-hal-v0.3.0
[v0.2.0]: https://egit.irs.uni-stuttgart.de/rust/vorago-shared-hal/compare/v0.1.0...v0.2.0
[v0.1.0]: https://egit.irs.uni-stuttgart.de/rust/vorago-shared-hal/src/tag/v0.1.0
