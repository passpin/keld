//! Narrow adapter for the versioned runtime C ABI.

#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

use keld_native_abi::ABI_VERSION;
use std::io::Write;

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

/// Prints one language fault using the interpreter's stable diagnostic format.
///
/// The path pointer is borrowed only for the duration of this call. A null
/// pointer or an invalid fault kind becomes an internal ABI failure.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_print_fault(
    kind: u32,
    path: *const u8,
    path_len: usize,
    line: u32,
    column: u32,
) -> u32 {
    let Some(name) = fault_name(kind) else {
        return 2;
    };
    let Some(message) = fault_message(kind) else {
        return 2;
    };
    if path.is_null() {
        return 2;
    }
    // SAFETY: the generated executable supplies a valid immutable path buffer
    // for exactly this call; the adapter never retains the borrowed slice.
    let path = unsafe { std::slice::from_raw_parts(path, path_len) };
    let path = String::from_utf8_lossy(path);
    let mut stderr = std::io::stderr().lock();
    match writeln!(stderr, "{path}:{line}:{column}: runtime[{name}]: {message}") {
        Ok(()) => 0,
        Err(_) => 2,
    }
}

const fn fault_name(kind: u32) -> Option<&'static str> {
    match kind {
        1 => Some("ArithmeticFault"),
        2 => Some("DivisionByZeroFault"),
        3 => Some("ShiftFault"),
        4 => Some("AllocationFault"),
        5 => Some("CapacityFault"),
        6 => Some("BoundsFault"),
        _ => None,
    }
}

const fn fault_message(kind: u32) -> Option<&'static str> {
    match kind {
        1 => Some("checked integer arithmetic overflow"),
        2 => Some("integer division or remainder by zero"),
        3 => Some("invalid integer shift amount"),
        4 => Some("runtime allocation failed"),
        5 => Some("requested list capacity is impossible"),
        6 => Some("list index is out of bounds"),
        _ => None,
    }
}
