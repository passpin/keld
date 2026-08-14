// The test runtime deliberately compiles the reviewed FFI adapter with the
// test-controls feature enabled. Keeping one source of truth prevents the
// failure-injection DLL from drifting from the production ABI adapter.
mod adapter {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../keld-native-ffi/src/lib.rs"
    ));
}

pub use adapter::*;
