VA416xx Flashloader Application
========

This flashloader shows a minimal example for a self-updatable Rust software which exposes
a simple CCSDS packet interface to update the software. The
[flashloader client](../../host/flashloader-client) can be used to upload compiled
images to the flashloader application to write them to the NVM.

Please note that the both the application and the client are tailored towards usage
with the [bootloader provided by this repository](https://github.com/ustgt-irs/vorago-rs/tree/main/va416xx/bootloader).

The software can quickly be adapted to interface with a real primary on-board software instead of
the client provided here because it uses a low-level CCSDS based packet interface.
Requests and replies are [postcard](https://github.com/jamesmunns/postcard) encoded types from the
[types](../../host/flashloader-types) crate. Each packet is COBS framed on the wire.

## Using the flashloader client

The client communicates with the flashloader using a dedicated serial port with a baudrate of
115200. The flashloader uses the UART0 interface of the VA416xx board.

The client reads the chip and the serial port from a `loader.toml` file in the current directory:

```toml
chip = "va416xx"
serial_port = "/dev/ttyUSB0"
```

You can also pass them with `--chip` and `--port`. Run the following commands from the
`host/flashloader-client` directory and use `cargo run -- -h` to get an overview of all options.

### Examples

You can use

```sh
cargo run -- ping
```

to send a ping and verify the connection.

You can use

```sh
cd ../../va416xx/flashloader/slot-blinky
cargo build --release --features slot-a
cd ../../../host/flashloader-client
cargo run -- flash a ../../va416xx/flashloader/slot-blinky/target/thumbv7em-none-eabihf/release/slot-blinky
```

to build the slot A sample application and upload it to a running flash loader application
to write it to slot A. Use `--features slot-b` to build the image for slot B.

You can use

```sh
cargo run -- corrupt a
```

to corrupt the image A and test that it switches to image B after a failed CRC check instead.
