//! Generates `memory.x` from `memory.x.template` for the selected image slot.
use std::{env, fs, path::PathBuf};

use flashloader_types::va416xx::{
    APP_A_MAX_SIZE, APP_A_START_ADDR, APP_B_MAX_SIZE, APP_B_START_ADDR,
};

const TEMPLATE: &str = "memory.x.template";

fn main() {
    let slot_a = env::var_os("CARGO_FEATURE_SLOT_A").is_some();
    let slot_b = env::var_os("CARGO_FEATURE_SLOT_B").is_some();
    let (origin, length) = match (slot_a, slot_b) {
        (true, false) => (APP_A_START_ADDR, APP_A_MAX_SIZE),
        (false, true) => (APP_B_START_ADDR, APP_B_MAX_SIZE),
        _ => panic!("enable exactly one of the `slot-a` or `slot-b` features"),
    };

    let memory_x = fs::read_to_string(TEMPLATE)
        .unwrap()
        .replace("{{FLASH_ORIGIN}}", &format!("{origin:#010x}"))
        .replace("{{FLASH_LENGTH}}", &format!("{length:#x}"));
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(out.join("memory.x"), memory_x).unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed={TEMPLATE}");
    println!("cargo:rerun-if-changed=build.rs");
}
