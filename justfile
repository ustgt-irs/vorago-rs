all: check \
    build \
    check-fmt \
    clippy \
    test \
    docs

check: check-va108xx check-va416xx
build: build-va108xx build-va416xx
check-fmt: check-fmt-va108xx check-fmt-va416xx check-fmt-host
fmt: fmt-va108xx fmt-va416xx fmt-shared-hal fmt-host
clippy: clippy-va108xx clippy-va416xx clippy-shared-hal clippy-host
docs: docs-va108xx docs-va416xx docs-shared-hal
clean: clean-va108xx clean-va416xx clean-shared-hal clean-host
test: test-host

[working-directory: 'va108xx']
check-va108xx:
  cargo check --target thumbv6m-none-eabi
  cargo check --target thumbv6m-none-eabi --examples
  cargo check -p va108xx --target thumbv6m-none-eabi --all-features
  cargo check -p va108xx-hal --target thumbv6m-none-eabi --features "defmt"

[working-directory: 'va416xx']
check-va416xx:
  cargo check --target thumbv7em-none-eabihf
  cargo check -p va416xx --target thumbv7em-none-eabihf --all-features
  cargo check -p va416xx-hal --target thumbv7em-none-eabihf --features "defmt va41630"

[working-directory: 'va108xx']
build-va108xx:
  cargo build --target thumbv6m-none-eabi --release

[working-directory: 'va416xx']
build-va416xx:
  cargo build --target thumbv7em-none-eabihf

[working-directory: 'va108xx']
check-fmt-va108xx:
  cargo fmt --all -- --check

[working-directory: 'va416xx']
check-fmt-va416xx:
  cargo fmt --all -- --check

[working-directory: 'va108xx']
fmt-va108xx:
  cargo fmt

[working-directory: 'va416xx']
fmt-va416xx:
  cargo fmt

[working-directory: 'vorago-shared-hal']
fmt-shared-hal:
  cargo fmt

[working-directory: 'va108xx']
clippy-va108xx:
  cargo clippy --target thumbv6m-none-eabi -- -D warnings

[working-directory: 'va416xx']
clippy-va416xx:
  cargo clippy --target thumbv7em-none-eabihf -- -D warnings

[working-directory: 'vorago-shared-hal']
clippy-shared-hal:
  cargo clippy --target thumbv7em-none-eabihf --features "vor4x" -- -D warnings
  cargo clippy --target thumbv6m-none-eabi --features "vor1x" -- -D warnings

[working-directory: 'va108xx']
docs-va108xx:
  RUSTDOCFLAGS="--cfg docsrs --generate-link-to-definition -Z unstable-options" cargo +nightly doc -p va108xx --all-features
  RUSTDOCFLAGS="--cfg docsrs --generate-link-to-definition -Z unstable-options" cargo +nightly doc -p va108xx-hal --features "defmt, embassy-oc30-oc31" --no-deps
  RUSTDOCFLAGS="--cfg docsrs --generate-link-to-definition -Z unstable-options" cargo +nightly doc -p vorago-reb1 --no-deps

[working-directory: 'va416xx']
docs-va416xx:
  RUSTDOCFLAGS="--cfg docsrs --generate-link-to-definition -Z unstable-options" cargo +nightly doc -p va416xx
  RUSTDOCFLAGS="--cfg docsrs --generate-link-to-definition -Z unstable-options" cargo +nightly doc -p va416xx-hal --features va41630 --no-deps
  RUSTDOCFLAGS="--cfg docsrs --generate-link-to-definition -Z unstable-options" cargo +nightly doc -p vorago-peb1 --no-deps

[working-directory: 'vorago-shared-hal']
docs-shared-hal:
  RUSTDOCFLAGS="--cfg docsrs --generate-link-to-definition -Z unstable-options" cargo +nightly doc --target thumbv7em-none-eabihf --features "vor4x, defmt" --no-deps
  RUSTDOCFLAGS="--cfg docsrs --generate-link-to-definition -Z unstable-options" cargo +nightly doc --target thumbv6m-none-eabi --features "vor1x, defmt" --no-deps

[working-directory: 'va108xx']
clean-va108xx:
  cargo clean

[working-directory: 'va416xx']
clean-va416xx:
  cargo clean

[working-directory: 'vorago-shared-hal']
clean-shared-hal:
  cargo clean

[working-directory: 'host']
test-host:
  cargo test

[working-directory: 'host']
check-fmt-host:
  cargo fmt --all -- --check

[working-directory: 'host']
fmt-host:
  cargo fmt

[working-directory: 'host']
clippy-host:
  cargo clippy -- -D warnings

[working-directory: 'host']
clean-host:
  cargo clean
