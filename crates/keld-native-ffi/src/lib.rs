//! Narrow adapter for the versioned runtime C ABI.

#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

use keld_native_abi::{ABI_VERSION, FaultKind, KeldFault, KeldValue, RuntimeStatus};
use keld_native_runtime::{NativeValueError, RuntimeContext};
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};

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

fn status_code(status: RuntimeStatus) -> u32 {
    status as u32
}

fn record_value_failure(
    context: &mut RuntimeContext,
    error: NativeValueError,
    location: u32,
) -> u32 {
    match error {
        NativeValueError::Allocation => {
            context.record_language_fault(FaultKind::Allocation, location);
        }
        NativeValueError::InvalidHandle
        | NativeValueError::TypeMismatch
        | NativeValueError::Bounds => {
            context.record_internal_failure(location);
        }
    }
    status_code(context.status())
}

/// Creates one native runtime context. Null is an internal failure sentinel.
#[unsafe(no_mangle)]
pub extern "C" fn keld_rt_v1_context_new() -> *mut RuntimeContext {
    catch_unwind(|| RuntimeContext::new().map(|context| Box::into_raw(Box::new(context))))
        .ok()
        .and_then(Result::ok)
        .unwrap_or(std::ptr::null_mut())
}

/// Destroys a context after generated `main` has finished execution.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_context_destroy(context: *mut RuntimeContext) -> u32 {
    if context.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: ownership is transferred exactly once by context_new.
        unsafe { drop(Box::from_raw(context)) };
        status_code(RuntimeStatus::Ok)
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Returns the context's first-failure status.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_context_status(context: *const RuntimeContext) -> u32 {
    if context.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    // SAFETY: the caller borrows a live context for this synchronous query.
    unsafe { status_code((*context).status()) }
}

/// Copies the first recorded fault into a caller-owned ABI slot.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_context_fault(
    context: *const RuntimeContext,
    fault: *mut KeldFault,
) -> u32 {
    if context.is_null() || fault.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    // SAFETY: both pointers are checked non-null and borrowed synchronously.
    let (context, fault_slot) = unsafe { (&*context, &mut *fault) };
    *fault_slot = context.first_failure().unwrap_or_default();
    status_code(RuntimeStatus::Ok)
}

/// Copies a managed value into `dst`, committing the output only on success.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_value_copy(
    context: *mut RuntimeContext,
    src: *const KeldValue,
    dst: *mut KeldValue,
    location: u32,
) -> u32 {
    if context.is_null() || src.is_null() || dst.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed only for this call.
        let (context, source) = unsafe { (&mut *context, *src) };
        match context.copy_managed(source) {
            Ok(value) => {
                // SAFETY: dst is a valid caller-owned output slot.
                unsafe { *dst = value };
                status_code(RuntimeStatus::Ok)
            }
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Drops a managed value and clears its source slot only on success.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_value_drop(
    context: *mut RuntimeContext,
    value: *mut KeldValue,
    location: u32,
) -> u32 {
    if context.is_null() || value.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed only for this call.
        let (context, source) = unsafe { (&mut *context, *value) };
        match context.drop_managed(source) {
            Ok(()) => {
                // SAFETY: value is a valid caller-owned slot.
                unsafe { *value = KeldValue::default() };
                status_code(RuntimeStatus::Ok)
            }
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}
