//! Narrow adapter around the LLVM C API.

#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

pub const LLVM_SYS_VERSION: &str = "221.0.1";

/// Initializes only the x86 target when the LLVM feature is enabled.
#[cfg(feature = "llvm")]
pub fn initialize_x86_target() {
    unsafe {
        llvm_sys::target::LLVMInitializeX86TargetInfo();
        llvm_sys::target::LLVMInitializeX86Target();
        llvm_sys::target::LLVMInitializeX86TargetMC();
        llvm_sys::target::LLVMInitializeX86AsmPrinter();
    }
}

/// No-op counterpart used by workspace tests without a bootstrapped LLVM.
#[cfg(not(feature = "llvm"))]
pub const fn initialize_x86_target() {}
