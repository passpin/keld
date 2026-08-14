//! Narrow adapter for the versioned runtime C ABI.

#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

use keld_native_abi::ABI_VERSION;

/// Returns the runtime ABI version without exposing Rust layout or panics.
#[unsafe(no_mangle)]
pub extern "C" fn keld_rt_v1_abi_version() -> u32 {
    ABI_VERSION
}

/// Prints one successful Int result and returns an ABI status code.
#[unsafe(no_mangle)]
pub extern "C" fn keld_rt_v1_print_int(value: i64) -> u32 {
    use std::io::Write;

    let mut stdout = std::io::stdout().lock();
    match writeln!(stdout, "{value}") {
        Ok(()) => 0,
        Err(_) => 2,
    }
}
