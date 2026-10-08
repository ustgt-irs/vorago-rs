[![build](https://github.com/ustgt-irs/vorago-rs/actions/workflows/va416xx.yml/badge.svg)](https://github.com/ustgt-irs/vorago-rs/actions/workflows/va416xx.yml)

Vorago VA416xx Rust Support
=========

This crate collection provides support to write Rust applications for the VA416XX family
of devices.

## List of crates

This workspace contains the following crates:

- The [`va416xx`](https://github.com/ustgt-irs/vorago-rs/tree/main/va416xx/va416xx)
  PAC crate containing basic low-level register definition
- The [`va416xx-hal`](https://github.com/ustgt-irs/vorago-rs/tree/main/va416xx/va416xx-hal)
  HAL crate containing higher-level abstractions on top of the PAC register crate.
- The [`va416xx-embassy`](https://github.com/ustgt-irs/vorago-rs/tree/main/va416xx/va416xx-embassy)
  crate containing support for running the embassy-rs asynchronous runtime.
- The [`vorago-peb1`](https://github.com/ustgt-irs/vorago-rs/tree/main/va416xx/vorago-peb1)
  BSP crate containing support for the PEB1 development board.

It also contains the following helper crates:

- The [`bootloader`](https://github.com/ustgt-irs/vorago-rs/tree/main/va416xx/bootloader)
  crate contains a sample bootloader strongly based on the one provided by Vorago.
- The [`flashloader`](https://github.com/ustgt-irs/vorago-rs/tree/main/va416xx/flashloader)
  crate contains a sample flashloader which is able to update the redundant images in the NVM which
  is compatible to the provided bootloader as well.
- The [`examples`](https://github.com/ustgt-irs/vorago-rs/tree/main/va416xx/examples)
  folder contains various example applications crates using the HAL and the PAC.
  This folder also contains dedicated example applications using the
  [`RTIC`](https://rtic.rs/2/book/en/) and [`embassy`](https://github.com/embassy-rs/embassy)
  native Rust RTOSes.

Some parts of the HAL implementation and the Embassy-rs support are contained in the external
[`vorago-shared-periphs`](https://egit.irs.uni-stuttgart.de/rust/vorago-shared-periphs) crate.

## Using the `.cargo/config.toml` file

The `.cargo/config.toml` file sets the build target, the linker scripts and a `probe-rs` runner.
You can adapt it to your needs. For example, you can configure runners to conveniently flash with
`cargo run`.

## Using the sample VS Code files

Use the following command to have a starting configuration for VS Code:

```sh
cp -rT vscode .vscode
```

You can then adapt the files in `.vscode` to your needs.

## Building projects

The `rust-toolchain.toml` file makes `rustup` install the `thumbv7em-none-eabihf` target
automatically, so you can directly use `cargo build` to build the development version of the crate.
For example, you can use

```sh
cargo build --example blinky
```

to build a simple blinky app.

## Flashing, running and debugging the software

You can use CLI or VS Code for flashing, running and debugging.

### Using CLI with probe-rs

Install [probe-rs](https://probe.rs/docs/getting-started/installation/) first. On Linux, you also
need to install the [udev rules](https://probe.rs/docs/getting-started/probe-setup/#linux-udev-rules)
so that your user can access the debug probe.

You can use `probe-rs` to run the software and display RTT log output. However, debugging does not
work yet.

After installation, you can run the following command

```sh
probe-rs run --chip VA416xx_RAM --protocol jtag target/thumbv7em-none-eabihf/debug/blinky
```

to flash and run the blinky program on the RAM. There is also a `VA416xx` chip target
available for persistent flashing.

Runner configuration is available in the `.cargo/config.toml` file to use `probe-rs` for
convenience. `probe-rs` is also able to process and display `defmt` strings directly.

### Using VS Code

Following tools are required:

1. [SEGGER J-Link tools](https://www.segger.com/downloads/jlink/) installed
2. [gdb-multiarch](https://packages.debian.org/sid/gdb-multiarch) or similar
   cross-architecture debugger installed. All commands here assume `gdb-multiarch`.

Assuming a working debug connection to your VA416xx board, you can debug using VS Code with
the [`Cortex-Debug` plugin](https://marketplace.visualstudio.com/items?itemName=marus25.cortex-debug).
Please make sure that [`objdump-multiarch` and `nm-multiarch`](https://forums.raspberrypi.com/viewtopic.php?t=333146)
are installed as well.

Some sample configuration files for VS code were provided and can be used by running
`cp -rT vscode .vscode` like specified above. After that, you can use `Run and Debug`
to automatically rebuild and flash your application.

If you would like to use a custom GDB application, you can specify the gdb binary in the following
configuration variables in your `settings.json`:

- `"cortex-debug.gdbPath"`
- `"cortex-debug.gdbPath.linux"`
- `"cortex-debug.gdbPath.windows"`
- `"cortex-debug.gdbPath.osx"`

The provided VS Code configurations also provide an integrated RTT logger, which you can access
via the terminal at `RTT Ch:0 console`. In order for the RTT block address detection to
work properly, `objdump-multiarch` and `nm-multiarch` need to be installed.

### Using CLI with GDB and Segger J-Link Tools

Following tools are required:

1. [SEGGER J-Link tools](https://www.segger.com/downloads/jlink/) installed
2. [gdb-multiarch](https://packages.debian.org/sid/gdb-multiarch) or similar
   cross-architecture debugger installed. All commands here assume `gdb-multiarch`.

You can build the blinky example application with the following command

```sh
cargo build --example blinky
```

Start the GDB server first. Depending on whether the application is flashed to RAM or the NVM
flash memory, the server needs to be started with a different configuration
For example, on Debian based system the following command can be used to do this (this command
is also run when running the `jlink-gdb.sh` script)

**RAM Flash**

```sh
JLinkGDBServer -select USB -device Cortex-M4 -endian little -if SWD -speed 2000 \
  -LocalhostOnly -vd -jlinkscriptfile ./jlink/JLinkSettings.JLinkScript
```

**NVM Flash**

```sh
JLinkGDBServer -select USB -device VA416xx -endian little -if SWD -speed 2000 \
  -LocalhostOnly -vd
```

After this, you can flash and debug the application with the following command

```sh
gdb-mutliarch -q -x jlink/jlink.gdb target/thumbv7em-none-eabihf/debug/examples/blinky
```

Please note that you can automate all steps except starting the GDB server by using a cargo
runner configuration, for example with the following lines in your `.cargo/config.toml` file:

```toml
[target.'cfg(all(target_arch = "arm", target_os = "none"))']
runner = "gdb-multiarch -q -x jlink/jlink.gdb -tui"
```

After that, you can simply use `cargo run --example blinky` to flash the blinky
example.

### Using the RTT Viewer

The Segger RTT viewer can be used to display log messages received from the target. The base
address for the RTT block placement is 0x1fff8000. It is recommended to use a search range of
0x1000 around that base address when using the RTT viewer.

The RTT viewer will not be able to process `defmt` printouts. However, you can view the defmt
logs by [installing defmt-print](https://crates.io/crates/defmt-print) first and then running

```sh
defmt-print -e <pathToElfFile> tcp
```

The path of the ELF file which is being debugged needs to be specified for this to work.

## Learning (Embedded) Rust

If you are unfamiliar with Rust on Embedded Systems or Rust in general, the following resources
are recommended:

- [Rust Book](https://doc.rust-lang.org/book/)
- [Embedded Rust Book](https://docs.rust-embedded.org/book/)
- [Embedded Rust Discovery](https://docs.rust-embedded.org/discovery/microbit/)
- [Awesome Embedded Rust](https://github.com/rust-embedded/awesome-embedded-rust)
