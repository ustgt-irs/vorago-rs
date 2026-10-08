Vorago Microcontroller Rust Repository
=========

This monorepo provides a full embedded Rust ecosystem for the Vorago VA108xx (Cortex-M0) and
VA416xx (Cortex-M4) microcontrollers. See the
[Embedded Rust Book](https://docs.rust-embedded.org/book/start/registers.html) for an explanation
of the PAC, HAL and BSP layers.

## Feature List

- Peripheral access crates (PACs): [`va108xx`](va108xx/va108xx) and [`va416xx`](va416xx/va416xx),
  generated from the SVD files. Basic building block for all higher-level libraries.
- Hardware abstraction layers (HALs): [`va108xx-hal`](va108xx/va108xx-hal) and
  [`va416xx-hal`](va416xx/va416xx-hal). Also supports
  [`embedded-hal`](https://github.com/rust-embedded/embedded-hal) traits.
- Async drivers for GPIO, UART, SPI and I2C, with
  [`embedded-hal-async`](https://docs.rs/embedded-hal-async) support.
- [Embassy](https://embassy.dev/) time drivers based on the timer peripherals.
- VA416xx only: CAN, DMA, ADC, DAC, EDAC, watchdog, NVM access and the IRQ router.
- Board support packages (BSPs) for the REB1 and PEB1 boards.
- Examples for both families: bare-metal, [RTIC](https://rtic.rs/) and Embassy.
- Bootloader and flashloader for a boot scheme with two redundant, CRC-checked image slots.
  - Bootloaders for both families, which fall back to the second slot if an image is corrupt.
  - Flashloaders, which update the image slots over a UART with a CCSDS packet interface.
- Host tools:
  - A flashloader client for both families to flash and corrupt image slots, and to select the
    boot slot on the VA108xx.
  - Calculators for the VA416xx PLL and the UART clock configuration.
- Examples in this repository use the [`probe-rs`](https://probe.rs/) flashing tool which has
  support for both chips.
- No vendor SDK, Keil IDE or any C toolchain required.

## Workspaces

This monorepo is structured to contain the following top-level components:

- [`va108xx`](va108xx): Support for the VA108xx family of Cortex-M0 based MCUs
- [`va416xx`](va416xx): Support for the VA416xx family of Cortex-M4 based MCUs
- [`vorago-shared-hal`](vorago-shared-hal): Shared peripheral drivers used by both the VA108xx and
  VA416xx family
- [`host`](host): Code that also runs on your development computer

## Embedded Rust Learning Resources

If you are new to embedded Rust, these resources are a good starting point:

- [The Embedded Rust Book](https://docs.rust-embedded.org/book/): The official introduction to
  embedded Rust, including the
  [PAC, HAL and BSP layers](https://docs.rust-embedded.org/book/start/registers.html).
- [Rust Training](https://rust-training.ferrous-systems.com/) by Ferrous Systems: Training
  slides for Rust in general and for embedded Rust.
- [Rust Exercises](https://rust-exercises.ferrous-systems.com/) by Ferrous Systems: Hands-on
  exercises accompanying the training, including embedded exercises on real hardware.
- [The Rusty Bits](https://www.youtube.com/@therustybits): YouTube channel with videos about
  embedded Rust.
