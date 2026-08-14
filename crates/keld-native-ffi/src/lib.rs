//! Narrow adapter for the versioned runtime C ABI.

#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

use keld_native_abi::{
    ABI_VERSION, FaultKind, KeldEntity, KeldFault, KeldLifecycle, KeldLink, KeldValue,
    RuntimeStatus,
};
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
        NativeValueError::Capacity => {
            context.record_language_fault(FaultKind::Capacity, location);
        }
        NativeValueError::InvalidHandle | NativeValueError::TypeMismatch => {
            context.record_internal_failure(location);
        }
        NativeValueError::Bounds => {
            context.record_language_fault(FaultKind::Bounds, location);
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

/// Copies the first fault's numeric parts into separate output slots.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_context_fault_parts(
    context: *const RuntimeContext,
    kind: *mut u32,
    location: *mut u32,
) -> u32 {
    if context.is_null() || kind.is_null() || location.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    // SAFETY: pointers are checked non-null and borrowed synchronously.
    let (context, kind_slot, location_slot) = unsafe { (&*context, &mut *kind, &mut *location) };
    if let Some(fault) = context.first_failure() {
        *kind_slot = fault.kind;
        *location_slot = fault.location;
    } else {
        *kind_slot = 0;
        *location_slot = 0;
    }
    status_code(RuntimeStatus::Ok)
}

/// Copies the context's active root lifecycle into an ABI slot.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_context_root_lifecycle(
    context: *const RuntimeContext,
    out: *mut KeldLifecycle,
) -> u32 {
    if context.is_null() || out.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    // SAFETY: pointers are checked non-null and borrowed synchronously.
    let (context, out) = unsafe { (&*context, &mut *out) };
    *out = context.root_lifecycle();
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

/// Materializes one UTF-8 Text value into `dst`.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_text_new(
    context: *mut RuntimeContext,
    bytes: *const u8,
    length: usize,
    dst: *mut KeldValue,
    location: u32,
) -> u32 {
    if context.is_null() || bytes.is_null() || dst.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, bytes) =
            unsafe { (&mut *context, std::slice::from_raw_parts(bytes, length)) };
        match context.text_new(bytes) {
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

/// Computes the byte length of one Text value.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_text_byte_length(
    context: *mut RuntimeContext,
    value: *const KeldValue,
    out: *mut u64,
    location: u32,
) -> u32 {
    if context.is_null() || value.is_null() || out.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, value) = unsafe { (&mut *context, *value) };
        match context.text_byte_length(value) {
            Ok(length) => {
                // SAFETY: out is a valid caller-owned output slot.
                unsafe { *out = length };
                status_code(RuntimeStatus::Ok)
            }
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Computes whether one Text value is empty.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_text_is_empty(
    context: *mut RuntimeContext,
    value: *const KeldValue,
    out: *mut u8,
    location: u32,
) -> u32 {
    if context.is_null() || value.is_null() || out.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, value) = unsafe { (&mut *context, *value) };
        match context.text_is_empty(value) {
            Ok(empty) => {
                // SAFETY: out is a valid caller-owned output slot.
                unsafe { *out = u8::from(empty) };
                status_code(RuntimeStatus::Ok)
            }
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Compares two immutable Text payloads by exact UTF-8 bytes.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_text_equal(
    context: *mut RuntimeContext,
    lhs: *const KeldValue,
    rhs: *const KeldValue,
    out: *mut u8,
    location: u32,
) -> u32 {
    if context.is_null() || lhs.is_null() || rhs.is_null() || out.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, lhs, rhs) = unsafe { (&mut *context, *lhs, *rhs) };
        match context.text_equal(lhs, rhs) {
            Ok(equal) => {
                // SAFETY: out is a valid caller-owned output slot.
                unsafe { *out = u8::from(equal) };
                status_code(RuntimeStatus::Ok)
            }
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Concatenates two Text values into `dst` without committing on failure.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_text_concat(
    context: *mut RuntimeContext,
    lhs: *const KeldValue,
    rhs: *const KeldValue,
    dst: *mut KeldValue,
    location: u32,
) -> u32 {
    if context.is_null() || lhs.is_null() || rhs.is_null() || dst.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, lhs, rhs) = unsafe { (&mut *context, *lhs, *rhs) };
        match context.text_concat(lhs, rhs) {
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

/// Allocates an empty List value into `dst`.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_list_new(
    context: *mut RuntimeContext,
    dst: *mut KeldValue,
    location: u32,
) -> u32 {
    if context.is_null() || dst.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let context = unsafe { &mut *context };
        match context.list_new() {
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

/// Returns a List's element count.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_list_length(
    context: *mut RuntimeContext,
    list: *const KeldValue,
    out: *mut u64,
    location: u32,
) -> u32 {
    if context.is_null() || list.is_null() || out.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, list) = unsafe { (&mut *context, *list) };
        match context.list_length(list) {
            Ok(length) => {
                // SAFETY: out is a valid caller-owned output slot.
                unsafe { *out = length };
                status_code(RuntimeStatus::Ok)
            }
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Appends one value to a List. Managed values transfer their existing handle.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_list_push(
    context: *mut RuntimeContext,
    list: *const KeldValue,
    value: *const KeldValue,
    managed: u8,
    location: u32,
) -> u32 {
    if context.is_null() || list.is_null() || value.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, list, value) = unsafe { (&mut *context, *list, *value) };
        match context.list_push(list, value, managed != 0) {
            Ok(()) => status_code(RuntimeStatus::Ok),
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Gets one List element, deep-copying managed elements.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_list_get(
    context: *mut RuntimeContext,
    list: *const KeldValue,
    index: u64,
    out: *mut KeldValue,
    managed: u8,
    location: u32,
) -> u32 {
    if context.is_null() || list.is_null() || out.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, list) = unsafe { (&mut *context, *list) };
        let value = match context.list_get(list, index) {
            Ok((value, _)) => value,
            Err(error) => return record_value_failure(context, error, location),
        };
        let value = if managed != 0 {
            match context.copy_managed(value) {
                Ok(value) => value,
                Err(error) => return record_value_failure(context, error, location),
            }
        } else {
            value
        };
        let value = KeldValue {
            optional_some_layers: 1,
            ..value
        };
        // SAFETY: out is a valid caller-owned output slot.
        unsafe { *out = value };
        status_code(RuntimeStatus::Ok)
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Removes one List element without copying it.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_list_remove(
    context: *mut RuntimeContext,
    list: *const KeldValue,
    index: u64,
    out: *mut KeldValue,
    location: u32,
) -> u32 {
    if context.is_null() || list.is_null() || out.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, list) = unsafe { (&mut *context, *list) };
        match context.list_remove(list, index) {
            Ok((value, _)) => {
                // SAFETY: out is a valid caller-owned output slot.
                unsafe { *out = value };
                status_code(RuntimeStatus::Ok)
            }
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Replaces one List element and returns the displaced value.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_list_replace(
    context: *mut RuntimeContext,
    list: *const KeldValue,
    index: u64,
    value: *const KeldValue,
    out: *mut KeldValue,
    managed: u8,
    location: u32,
) -> u32 {
    if context.is_null() || list.is_null() || value.is_null() || out.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, list, value) = unsafe { (&mut *context, *list, *value) };
        match context.list_replace(list, index, value, managed != 0) {
            Ok((displaced, _)) => {
                // SAFETY: out is a valid caller-owned output slot.
                unsafe { *out = displaced };
                status_code(RuntimeStatus::Ok)
            }
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Removes one element into an Optional envelope, returning absent when out
/// of bounds without recording a fault.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_list_try_remove(
    context: *mut RuntimeContext,
    list: *const KeldValue,
    index: u64,
    out: *mut KeldValue,
    location: u32,
) -> u32 {
    if context.is_null() || list.is_null() || out.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, list) = unsafe { (&mut *context, *list) };
        match context.list_remove(list, index) {
            Ok((value, _)) => {
                let value = KeldValue {
                    optional_some_layers: 1,
                    ..value
                };
                // SAFETY: out is a valid caller-owned output slot.
                unsafe { *out = value };
                status_code(RuntimeStatus::Ok)
            }
            Err(NativeValueError::Bounds) => {
                // SAFETY: out is a valid caller-owned output slot.
                unsafe { *out = KeldValue::default() };
                status_code(RuntimeStatus::Ok)
            }
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Clears a List and recursively drops managed elements.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_list_clear(
    context: *mut RuntimeContext,
    list: *const KeldValue,
    location: u32,
) -> u32 {
    if context.is_null() || list.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, list) = unsafe { (&mut *context, *list) };
        match context.list_clear(list) {
            Ok(()) => status_code(RuntimeStatus::Ok),
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Reserves List capacity, reporting language faults on failure.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_list_reserve(
    context: *mut RuntimeContext,
    list: *const KeldValue,
    additional: u64,
    location: u32,
) -> u32 {
    if context.is_null() || list.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, list) = unsafe { (&mut *context, *list) };
        match context.list_reserve(list, additional) {
            Ok(()) => status_code(RuntimeStatus::Ok),
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Tries List growth, returning false for allocation or capacity failure.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_list_try_reserve(
    context: *mut RuntimeContext,
    list: *const KeldValue,
    additional: u64,
    out: *mut u8,
    location: u32,
) -> u32 {
    if context.is_null() || list.is_null() || out.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, list) = unsafe { (&mut *context, *list) };
        match context.list_try_reserve(list, additional) {
            Ok(success) => {
                // SAFETY: out is a valid caller-owned output slot.
                unsafe { *out = u8::from(success) };
                status_code(RuntimeStatus::Ok)
            }
            Err(NativeValueError::Capacity | NativeValueError::Allocation) => {
                // SAFETY: out is a valid caller-owned output slot.
                unsafe { *out = 0 };
                status_code(RuntimeStatus::Ok)
            }
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Constructs one struct payload from caller-owned field envelopes.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_struct_new(
    context: *mut RuntimeContext,
    definition: u32,
    fields: *const KeldValue,
    managed: *const u8,
    count: u32,
    dst: *mut KeldValue,
    location: u32,
) -> u32 {
    if context.is_null() || dst.is_null() || (count != 0 && (fields.is_null() || managed.is_null()))
    {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let Ok(count) = usize::try_from(count) else {
            return status_code(RuntimeStatus::InternalFailure);
        };
        // SAFETY: non-null arrays are validated for non-zero lengths and are
        // borrowed only for this synchronous call.
        let (fields, managed) = if count == 0 {
            (&[][..], &[][..])
        } else {
            // SAFETY: non-null arrays are validated for non-zero lengths and
            // are borrowed only for this synchronous call.
            unsafe {
                (
                    std::slice::from_raw_parts(fields, count),
                    std::slice::from_raw_parts(managed, count),
                )
            }
        };
        // SAFETY: context is checked non-null and borrowed synchronously.
        let context = unsafe { &mut *context };
        let flags = managed.iter().map(|flag| *flag != 0).collect::<Vec<_>>();
        match context.struct_new(definition, fields, &flags) {
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

/// Reads one struct field, deep-copying it when the field is managed.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_struct_field(
    context: *mut RuntimeContext,
    value: *const KeldValue,
    field: u32,
    out: *mut KeldValue,
    managed: u8,
    location: u32,
) -> u32 {
    if context.is_null() || value.is_null() || out.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, value) = unsafe { (&mut *context, *value) };
        let (field, field_is_managed) = match context.struct_field(value, field) {
            Ok(field) => field,
            Err(error) => return record_value_failure(context, error, location),
        };
        let field = if managed != 0 || field_is_managed {
            match context.copy_managed(field) {
                Ok(field) => field,
                Err(error) => return record_value_failure(context, error, location),
            }
        } else {
            field
        };
        // SAFETY: out is a valid caller-owned output slot.
        unsafe { *out = field };
        status_code(RuntimeStatus::Ok)
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Starts a child lifecycle.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_begin_lifecycle(
    context: *mut RuntimeContext,
    parent: *const KeldLifecycle,
    dst: *mut KeldLifecycle,
    location: u32,
) -> u32 {
    if context.is_null() || parent.is_null() || dst.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, parent) = unsafe { (&mut *context, *parent) };
        match context.begin_lifecycle(parent) {
            Ok(lifecycle) => {
                // SAFETY: dst is a valid caller-owned output slot.
                unsafe { *dst = lifecycle };
                status_code(RuntimeStatus::Ok)
            }
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Ends a child lifecycle and cleans its entities.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_end_lifecycle(
    context: *mut RuntimeContext,
    lifecycle: *const KeldLifecycle,
    location: u32,
) -> u32 {
    if context.is_null() || lifecycle.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, lifecycle) = unsafe { (&mut *context, *lifecycle) };
        match context.end_lifecycle(lifecycle) {
            Ok(()) => status_code(RuntimeStatus::Ok),
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Allocates an entity and moves its field envelopes into runtime custody.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_allocate_entity(
    context: *mut RuntimeContext,
    definition: u32,
    fields: *const KeldValue,
    managed: *const u8,
    count: u32,
    lifecycle: *const KeldLifecycle,
    dst: *mut KeldValue,
    location: u32,
) -> u32 {
    if context.is_null()
        || lifecycle.is_null()
        || dst.is_null()
        || (count != 0 && (fields.is_null() || managed.is_null()))
    {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let Ok(count) = usize::try_from(count) else {
            return status_code(RuntimeStatus::InternalFailure);
        };
        let (fields, managed) = if count == 0 {
            (&[][..], &[][..])
        } else {
            // SAFETY: non-null arrays are validated for non-zero lengths and
            // borrowed only for this synchronous call.
            unsafe {
                (
                    std::slice::from_raw_parts(fields, count),
                    std::slice::from_raw_parts(managed, count),
                )
            }
        };
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, lifecycle) = unsafe { (&mut *context, *lifecycle) };
        let flags = managed.iter().map(|flag| *flag != 0).collect::<Vec<_>>();
        match context.allocate_entity(definition, fields, &flags, lifecycle) {
            Ok(entity) => {
                // SAFETY: dst is a valid caller-owned output slot.
                unsafe {
                    *dst = KeldValue {
                        words: [
                            entity.brand,
                            (u64::from(entity.generation) << 32) | u64::from(entity.slot),
                            u64::from(entity.definition),
                        ],
                        ..KeldValue::default()
                    }
                };
                status_code(RuntimeStatus::Ok)
            }
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

fn entity_from_value(value: KeldValue) -> KeldEntity {
    KeldEntity {
        brand: value.words[0],
        slot: u32::try_from(value.words[1] & u64::from(u32::MAX)).unwrap_or(0),
        generation: u32::try_from(value.words[1] >> 32).unwrap_or(0),
        definition: u32::try_from(value.words[2]).unwrap_or(0),
        reserved: 0,
    }
}

fn link_from_value(value: KeldValue) -> KeldLink {
    KeldLink {
        brand: value.words[0],
        slot: u32::try_from(value.words[1] & u64::from(u32::MAX)).unwrap_or(0),
        generation: u32::try_from(value.words[1] >> 32).unwrap_or(0),
        expected: u32::try_from(value.words[2]).unwrap_or(0),
        reserved: 0,
    }
}

fn entity_to_value(entity: KeldEntity) -> KeldValue {
    KeldValue {
        words: [
            entity.brand,
            (u64::from(entity.generation) << 32) | u64::from(entity.slot),
            u64::from(entity.definition),
        ],
        ..KeldValue::default()
    }
}

fn link_to_value(link: KeldLink) -> KeldValue {
    KeldValue {
        words: [
            link.brand,
            (u64::from(link.generation) << 32) | u64::from(link.slot),
            u64::from(link.expected),
        ],
        ..KeldValue::default()
    }
}

/// Converts an entity identity to a Link value.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_entity_to_link(
    context: *mut RuntimeContext,
    entity: *const KeldValue,
    dst: *mut KeldValue,
    location: u32,
) -> u32 {
    if context.is_null() || entity.is_null() || dst.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, entity) = unsafe { (&mut *context, *entity) };
        match context.entity_to_link(entity_from_value(entity)) {
            Ok(link) => {
                // SAFETY: dst is a valid caller-owned output slot.
                unsafe { *dst = link_to_value(link) };
                status_code(RuntimeStatus::Ok)
            }
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Resolves a Link, writing a live flag and entity value.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_resolve_link(
    context: *mut RuntimeContext,
    link: *const KeldValue,
    entity: *mut KeldValue,
    live: *mut u8,
    location: u32,
) -> u32 {
    if context.is_null() || link.is_null() || entity.is_null() || live.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, link) = unsafe { (&mut *context, *link) };
        let resolved = context.resolve_link(link_from_value(link));
        // SAFETY: outputs are valid caller-owned slots.
        unsafe {
            if let Some(entity_value) = resolved {
                *entity = entity_to_value(entity_value);
                *live = 1;
            } else {
                *entity = KeldValue::default();
                *live = 0;
            }
        }
        let _ = location;
        status_code(RuntimeStatus::Ok)
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Reads one entity field, copying managed values when requested.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_entity_field(
    context: *mut RuntimeContext,
    entity: *const KeldValue,
    field: u32,
    out: *mut KeldValue,
    managed: u8,
    location: u32,
) -> u32 {
    if context.is_null() || entity.is_null() || out.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, entity) = unsafe { (&mut *context, *entity) };
        let (field, field_is_managed) = match context.entity_field(entity_from_value(entity), field)
        {
            Ok(field) => field,
            Err(error) => return record_value_failure(context, error, location),
        };
        let field = if managed != 0 || field_is_managed {
            match context.copy_managed(field) {
                Ok(field) => field,
                Err(error) => return record_value_failure(context, error, location),
            }
        } else {
            field
        };
        // SAFETY: out is a valid caller-owned output slot.
        unsafe { *out = field };
        status_code(RuntimeStatus::Ok)
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Replaces one entity field and returns the displaced value.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_replace_field(
    context: *mut RuntimeContext,
    entity: *const KeldValue,
    field: u32,
    value: *const KeldValue,
    out: *mut KeldValue,
    managed: u8,
    location: u32,
) -> u32 {
    if context.is_null() || entity.is_null() || value.is_null() || out.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, entity, value) = unsafe { (&mut *context, *entity, *value) };
        match context.replace_entity_field(entity_from_value(entity), field, value, managed != 0) {
            Ok((displaced, _)) => {
                // SAFETY: out is a valid caller-owned output slot.
                unsafe { *out = displaced };
                status_code(RuntimeStatus::Ok)
            }
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Keeps an entity in an ancestor lifecycle.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_keep_entity(
    context: *mut RuntimeContext,
    entity: *const KeldValue,
    lifecycle: *const KeldLifecycle,
    location: u32,
) -> u32 {
    if context.is_null() || entity.is_null() || lifecycle.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, entity, lifecycle) = unsafe { (&mut *context, *entity, *lifecycle) };
        match context.keep_entity(entity_from_value(entity), lifecycle) {
            Ok(()) => status_code(RuntimeStatus::Ok),
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}

/// Retires one entity and cleans its managed fields.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn keld_rt_v1_retire_entity(
    context: *mut RuntimeContext,
    entity: *const KeldValue,
    location: u32,
) -> u32 {
    if context.is_null() || entity.is_null() {
        return status_code(RuntimeStatus::InternalFailure);
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: pointers are checked non-null and borrowed synchronously.
        let (context, entity) = unsafe { (&mut *context, *entity) };
        match context.retire_entity(entity_from_value(entity)) {
            Ok(()) => status_code(RuntimeStatus::Ok),
            Err(error) => record_value_failure(context, error, location),
        }
    }));
    result.unwrap_or(status_code(RuntimeStatus::InternalFailure))
}
