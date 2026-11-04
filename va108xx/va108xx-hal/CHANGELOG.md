Change Log
=======

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/)
and this project adheres to [Semantic Versioning](http://semver.org/).

## [unreleased]

### Changed

- The async SPI driver's interrupt handler no longer takes a critical section on every
  interrupt. The shared transfer state moved from a `Mutex<RefCell<TransferContext>>` to plain
  atomics, gated by an `Acquire`/`Release` flag that publishes the transfer buffers.
- `SpiAsync` was renamed to `asynch::Spi`. Its `on_interrupt` free function became
  `Spi::on_interrupt`. It now takes a `Bank` token obtained from the new `Spi::bank_id` instead
  of requiring access to the driver instance, so it can be called from an interrupt handler that
  only has a token, not the driver.
- Added `Spi::into_async` on the blocking driver as a shortcut for `asynch::Spi::new`.
- `SpiConfig` was renamed to `Config` and reworked: it is now `#[non_exhaustive]` with public
  fields (`clock`, `mode`, `blockmode`, ...) instead of a builder-method API, plus a
  `Config::new(mode, clock)` constructor for the two fields without a sensible default.
  `SpiClockConfig` was renamed to `ClockConfig`.
- The async SPI driver now always enables blockmode and blockmode stalling. It marks the last
  word of a transfer with the BMSTART_BMSTOP bit, which ends the frame and deasserts a hardware
  chip select. This overrides the `blockmode` and `bmstall` settings of the passed `Config`.
- The async UART TX driver's interrupt handler no longer takes a critical section on every
  interrupt either. The shared transfer state moved from a `Mutex<RefCell<TxContext>>` to plain
  atomics, gated the same way as the SPI driver. This also drops the `raw-buffer` dependency.
- `TxAsync` was moved into a new `uart::asynch` module and renamed to `asynch::Tx`. Its free
  `on_interrupt_tx` function became `Tx::on_interrupt`, taking a `Bank` token from the new
  `Tx::bank_id` instead of requiring access to the driver instance.
- `asynch::Tx::new` and `Tx::into_async` are safe again. The requirement that the `Drop` handler
  of generated futures runs is documented instead. The corner case is exotic enough to not
  justify an `unsafe` API.
- `asynch::Tx::write` is now an `async fn` returning the number of written bytes. The `TxFuture`
  type was removed. The transfer now starts on the first poll of the future, not when `write` is
  called.
- Removed `TxOverrunError`. It was never returned. `asynch::Tx::write_all` and `flush` now return
  `()`, and the `embedded_io` error type of `asynch::Tx` is `core::convert::Infallible`.
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

### Fixed

- Hardware chip select is now deasserted at the end of an async SPI transfer. Previously the
  async driver disabled blockmode and never set the BMSTART_BMSTOP bit, so CS stayed asserted.
- Cancelling an async SPI transfer now ends the blockmode frame. Dropping a transfer future only
  cleared the FIFOs, which leaves the frame open and the chip select asserted.

## [v0.14.0] 2026-07-14

### Changed

- Async TX UART `write` now returns a `TxFuture`
- Empty async TX write resolves to `Poll::Ready(0)` immediately

## [v0.13.1] 2026-05-19

- Docs.rs feature fix

## [v0.13.0] 2026-05-19

- Bump `vorago-shared-hal` dependency to v0.4
- Integrate `va108xx-embassy` as a `embassy-time` module.

## [v0.12.0] 2025-09-03

## Changed

- Move most library components to new [`vorago-shared-hal`](https://egit.irs.uni-stuttgart.de/rust/vorago-shared-hal)
  which is mostly re-exported in this crate.
- Overhaul and simplification of several HAL APIs. The system configuration and IRQ router
  peripheral instance generally does not need to be passed to HAL API anymore.
- All HAL drivers are now type erased. The constructors will still expect and consume the PAC
  singleton component for resource management purposes, but are not cached anymore.
- Refactoring of GPIO library to be more inline with embassy GPIO API.

## Added

- I2C clock timeout feature support.

## [v0.11.1] 2025-03-10

## Fixed

- Fix `embedded_io` UART implementation to implement the documented contract properly.
  The implementation will now block until at least one byte is available or can be written, unless
  the send or receive buffer is empty.

## [v0.11.0] 2025-03-07

## Changed

- Bugfix for I2C `TimingCfg::reg`
- Simplified UART error handling. All APIs are now infallible because writing to a FIFO or
  reading from a FIFO never fails. Users can either poll errors using `Rx::poll_errors` or
  `Uart::poll_rx_errors` / `UartBase::poll_rx_errors`, or detect errors using the provided
  interrupt handlers.

## [v0.10.0] 2025-02-17

## Added

- A lot of missing `defmt::Format` implementations.

## Changed

- Larger refactoring of GPIO library. The edge and level interrupt configurator functions do not
  enable interrupts anymore. Instead, there are explicit `enbable_interrupt` and
  `disable_interrupt` methods
- Renamed GPIO `DynGroup` to `Port`
- Rename generic GPIO interrupt handler into `on_interrupt_for_asynch_gpio`
  into `on_interrupt_for_async_gpio_for_port` which expects a Port argument

## Fixed

- Bug in async GPIO interrupt handler where all enabled interrupts, even the ones which might
  be unrelated to the pin, were disabled.

## [v0.9.0] 2025-02-13

## Fixed

- Important bugfix for UART driver which causes UART B drivers not to work.

## Removed

- Deleted some HAL re-exports in the PWM module

## Changed

- GPIO API: Interrupt, pulse and filter and `set_datamask` and `clear_datamask` APIs are now
  methods which mutable modify the pin instead of consuming and returning it.
- Simplified PWM module implementation.
- All error types now implement `core::error::Error` by using the `thiserror::Error` derive.
- `InvalidPinTypeError` now wraps the pin mode.
- I2C `TimingCfg` constructor now returns explicit error instead of generic Error.
  Removed the timing configuration error type from the generic I2C error enumeration.
- `PinsA` and `PinsB` constructor do not expect an optional `pac::Ioconfig` argument anymore.
- `IrqCfg` renamed to `InterruptConfig`, kept alias for old name.
- All library provided interrupt handlers now start with common prefix `on_interrupt_*`
- `RxWithIrq` renamed to `RxWithInterrupt`
- `Rx::into_rx_with_irq` does not expect any arguments any more.
- `filter_type` renamed to `configure_filter_type`.
- `level_irq` renamed to `configure_level_interrupt`.
- `edge_irq` renamed to `configure_edge_interrupt`.
- `PinsA` and `PinsB` constructor do not expect an optional IOCONFIG argument anymore.
- UART interrupt management is now handled by the main constructor instead of later stages to
  statically ensure one interrupt vector for the UART peripheral. `Uart::new` expects an
  optional `InterruptConfig` argument.
- `enable_interrupt` and `disable_interrupt` renamed to `enable_nvic_interrupt` and
  `disable_nvic_interrupt` to distinguish them from peripheral interrupts more clearly.
- `port_mux` renamed to `port_function_select`
- Renamed `IrqUartErrors` to `UartErrors`.

## Added

- Add `downgrade` method for `Pin` and `upgrade` method for `DynPin` as explicit conversion
  methods.
- Asynchronous GPIO support.
- Asynchronous UART TX support.
- Asynchronous UART RX support.
- Add new `get_tim_raw` unsafe method to retrieve TIM peripheral blocks.
- `Uart::with_with_interrupt` and `Uart::new_without_interrupt`

## [v0.8.0] 2024-09-30

## Changed

- Improves `CascardSource` handling and general API when chosing cascade sources.
- Replaced `utility::unmask_irq` by `enable_interrupt` and `disable_interrupt` API.
- Improve and fix SPI abstractions. Add new low level interface. The primary SPI constructor now
  only expects a configuration structure and the transfer configuration needs to be applied in a
  separate step.
- Removed complete `timer` module re-export in `pwm` module
- `CountDownTimer` renamed to `CountdownTimer`

## Fixes

- Fixes for SPI peripheral: Flush implementation was incorrect and should now flush properly.

## [v0.7.0] 2024-07-04

- Replace `uarta` and `uartb` `Uart` constructors by `new` constructor
- Replace SPI `spia`, `spib` and `spic` constructors by `new` constructor
- Replace I2C `i2ca`, `i2cb` constructors by `new` constructor. Update constructor
  to fail on invalid fast I2C speed system clock values
- Renamed `gpio::pins` to `gpio::pin` and `gpio::dynpins` to `gpio::dynpin`
- Simplify UART clock divider calculations and remove `libm` dependency consequently

## [v0.6.0] 2024-06-16

- Updated `embedded-hal` to v1
- Added optional `defmt` v0.3 feature and support.

## v0.5.2 2024-06-16

## Fixed

- Replaced usage to `ptr::write_volatile` in UART module which is denied on more recent Rust
  compilers.

## v0.5.1

### Changes

- Updated dependencies:
  - `cortex-m-rtic` (dev-depencency) to 1.1.2
  - `once_cell` to 1.12.0
  - Other dependencies: Only revision has changed

## v0.5.0

### Added

- Reactored IRQ handling, so that `unmask` operations can be moved to HAL
- Added UART IRQ handler. Right now, can only perform reception, TX still needs to be done in
  a blocking manner
- Added RTIC template and RTIC UART IRQ application

### Fixed

- Bugfix in UART code where RX and TX could not be enabled or disabled independently

## v0.4.3

- Various smaller fixes for READMEs, update of links in documentation
- Simplified CI for github, do not use `cross`
- New `blinky-pac` example
- Use HAL delay in `blinky` example

## v0.4.2

### Added

- `port_mux` function to set pin function select manually

### Changed

- Clear TX and RX FIFO in SPI transfer function

## v0.4.1

### Fixed

- Initial blockmode setting was not set in SPI constructor

## v0.4.0

### Changed

- Replaced `Hertz` by `impl Into<Hertz>` completely and removed
  `+ Copy` where not necessary

## v0.3.1

- Updated all links to point to new repository

## v0.3.0

### Added

- TIM Cascade example

### Changed

- `CountDownTimer` new function now expects an `impl Into<Hertz>` instead of `Hertz`
- Primary repository now hosted on IRS external git: https://egit.irs.uni-stuttgart.de/rust/va108xx-hal
- Relicensed as Apache-2.0

## v0.2.3

### Added

- Basic API for EDAC functionality
- PWM implementation and example
- API to perform peripheral resets

### Changed

- Improved Timer API. It is now possible to simply use `new` on `CountDownTimer`

## v0.2.2

### Added

- DelayUs and DelayMs trait implementations for timer
- SPI implementation for blocking API, supports blockmode as well
- Basic I2C implementation for blocking API

### Changed

- API which expects values in Hertz now uses `impl Into<Hertz>` as input parameter

## v0.2.1

### Added

- Adds the IRQ interface to configure interrupts on output and input pins
- Utility function to set up millisecond timer with `TIM0`
- Function to set clock divisor registers in `clock` module

### Changed

- Minor optimizations and tweaks for GPIO module
- Moved the `FilterClkSel` struct to the `clock` module, re-exporting in `gpio`
- Clearing output state at initialization of Output pins

## v0.2.0

### Changed

- New GPIO implementation which uses type-level programming. Implementation heavily based on the
  ATSAMD GPIO HAL: https://docs.rs/atsamd-hal/0.13.0/atsamd_hal/gpio/v2/index.html
- Changes to API, therefore minor version bump

### Added

- UART implementation
- UART example
- Some bugfixes for GPIO implementation
- Rust edition updated to 2021

## v0.1.0

### Added

- First version of the HAL which adds the GPIO implementation and timer implementation.
- Also adds some examples and helper files to set up new binary crates
- RTT example application
- Added basic test binary in form of an example
- README with basic instructions how to set up own binary crate


[unreleased]: https://github.com/ustgt-irs/vorago-rs/compare/va108xx-hal-v0.14.0...HEAD
[v0.14.0]: https://github.com/ustgt-irs/vorago-rs/releases/tag/va108xx-hal-v0.14.0
[v0.13.1]: https://github.com/ustgt-irs/vorago-rs/compare/va108xx-hal-v0.13.0...va108xx-hal-v0.13.1
[v0.13.0]: https://github.com/ustgt-irs/vorago-rs/releases/tag/va108xx-hal-v0.13.0
[v0.12.0]: https://egit.irs.uni-stuttgart.de/rust/va108xx-rs/compare/va108xx-hal-v0.11.1...va108xx-hal-v0.12.0
[v0.11.1]: https://egit.irs.uni-stuttgart.de/rust/va108xx-rs/compare/va108xx-hal-v0.11.0...va108xx-hal-v0.11.1
[v0.11.0]: https://egit.irs.uni-stuttgart.de/rust/va108xx-rs/compare/va108xx-hal-v0.10.0...va108xx-hal-v0.11.0
[v0.10.0]: https://egit.irs.uni-stuttgart.de/rust/va108xx-rs/compare/va108xx-hal-v0.9.0...va108xx-hal-v0.10.0
[v0.9.0]: https://egit.irs.uni-stuttgart.de/rust/va108xx-rs/compare/va108xx-hal-v0.8.0...va108xx-hal-v0.9.0
[v0.8.0]: https://egit.irs.uni-stuttgart.de/rust/va108xx-rs/compare/va108xx-hal-v0.7.0...va108xx-hal-v0.8.0
[v0.7.0]: https://egit.irs.uni-stuttgart.de/rust/va108xx-rs/compare/va108xx-hal-v0.6.0...va108xx-hal-v0.7.0
[v0.6.0]: https://egit.irs.uni-stuttgart.de/rust/va108xx-rs/src/tag/va108xx-hal-v0.6.0
