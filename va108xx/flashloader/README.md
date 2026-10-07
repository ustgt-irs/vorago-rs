VA108xx Flashloader Application
========

This flashloader shows a minimal example for a self-updatable Rust software which exposes
a simple CCSDS packet interface to update the software. It also provides a Rust application
which can be used to upload compiled images to the flashloader application to write them to the NVM.
You can find it inside the `tools/va108xx-flashloader-client` directory of the monorepo.

Please note that the both the application and the image loader are tailored towards usage
with the [bootloader provided by this repository](https://github.com/ustgt-irs/vorago-rs/tree/main/va108xx/bootloader).

The flashloader software could be be adapted to interface with a real primary on-board software
instead of the loader application provided here to upload images because it already uses a
low-level CCSDS based packet interface.

## Using the image loader

Inside `tools/va108xx-image-loader` you can find a Rust application which can be used to
update the image slots via a serial port.

You can install this tool using the following command inside the project folder:

```
cargo install --path .
```

After that, you can run `va108xx-iamge-loader --help` to get some to get usage informations.

The flash loader uses the UART0 with the Pins PA8 (RX) and PA9 (TX) interface of the VA108xx to
perform CCSDS based communication. The serial port can be set inside the `config.toml` file
or with the `--port` argument.

### Examples

You can use

```sh
va108xx-image-loader ping
```

to send a ping an verify the connection.

You can use

```sh
cd flashloader/slot-a-blinky
cargo build --release
va108xx-image-loader flash a ./target/thumbv6m-none-eabi/release/slot-a-blinky
```

to build the slot A sample application and upload it to a running flash loader application
to write it to slot A.

You can use

```sh
va108xx-image-loader set-boot-slot a 
```

to select the Slot A as a boot slot. The boot slot is stored in a reserved section in EEPROM
and will be read and used by the bootloader to determine which slot to boot.

You can use

```sh
va108xx-image-loader corrupt a 
```

to corrupt the image A and test that it switches to image B after a failed CRC check instead.
