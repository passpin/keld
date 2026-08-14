//! Narrow adapter around the LLVM C API.

#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

use std::fmt;
#[cfg(not(feature = "llvm"))]
use std::path::Path;

pub const LLVM_SYS_VERSION: &str = "221.0.1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OptimizationLevel {
    O0,
    O2,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LlvmError {
    InvalidString(String),
    Message(String),
    FeatureDisabled,
}

impl fmt::Display for LlvmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidString(value) => write!(formatter, "LLVM string contains NUL: {value}"),
            Self::Message(message) => formatter.write_str(message),
            Self::FeatureDisabled => formatter.write_str("LLVM adapter feature is disabled"),
        }
    }
}

impl std::error::Error for LlvmError {}

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

#[cfg(feature = "llvm")]
mod enabled {
    use super::{LlvmError, OptimizationLevel, initialize_x86_target};
    use llvm_sys::analysis::{LLVMVerifierFailureAction, LLVMVerifyModule};
    use llvm_sys::core::{
        LLVMAddFunction, LLVMAppendBasicBlockInContext, LLVMBuildCall2, LLVMBuildRet, LLVMConstInt,
        LLVMContextCreate, LLVMContextDispose, LLVMCreateBuilderInContext, LLVMDisposeBuilder,
        LLVMDisposeMessage, LLVMDisposeModule, LLVMFunctionType, LLVMInt32TypeInContext,
        LLVMInt64TypeInContext, LLVMModuleCreateWithNameInContext, LLVMPositionBuilderAtEnd,
        LLVMPrintModuleToString, LLVMSetDataLayout, LLVMSetSourceFileName, LLVMSetTarget,
    };
    use llvm_sys::prelude::{LLVMContextRef, LLVMModuleRef};
    use llvm_sys::target::{LLVMCopyStringRepOfTargetData, LLVMDisposeTargetData};
    use llvm_sys::target_machine::{
        LLVMCodeGenFileType, LLVMCodeGenOptLevel, LLVMCodeModel, LLVMCreateTargetDataLayout,
        LLVMCreateTargetMachine, LLVMDisposeTargetMachine, LLVMGetTargetFromTriple, LLVMRelocMode,
        LLVMTargetMachineEmitToFile, LLVMTargetMachineRef,
    };
    use llvm_sys::transforms::pass_builder::{
        LLVMCreatePassBuilderOptions, LLVMDisposePassBuilderOptions,
        LLVMPassBuilderOptionsSetVerifyEach, LLVMRunPasses,
    };
    use std::ffi::{CStr, CString, NulError};
    use std::os::raw::c_char;
    use std::path::Path;
    use std::ptr::null_mut;

    fn cstring(value: impl AsRef<str>) -> Result<CString, LlvmError> {
        CString::new(value.as_ref())
            .map_err(|error: NulError| LlvmError::InvalidString(error.nul_position().to_string()))
    }

    fn message(pointer: *mut c_char) -> String {
        if pointer.is_null() {
            return "LLVM returned an unknown error".to_owned();
        }
        // SAFETY: LLVM returns a NUL-terminated diagnostic string and transfers
        // ownership to this adapter until LLVMDisposeMessage.
        let message = unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: pointer came from an LLVM API that documents LLVMDisposeMessage.
        unsafe { LLVMDisposeMessage(pointer) };
        message
    }

    fn verify(module: LLVMModuleRef, phase: &str) -> Result<(), LlvmError> {
        let mut diagnostic = null_mut();
        // SAFETY: module is owned by Resources and remains live for this call;
        // diagnostic is an out pointer initialized to null.
        let failed = unsafe {
            LLVMVerifyModule(
                module,
                LLVMVerifierFailureAction::LLVMReturnStatusAction,
                &raw mut diagnostic,
            )
        };
        if failed == 0 {
            Ok(())
        } else {
            Err(LlvmError::Message(format!(
                "LLVM verification failed {phase}: {}",
                message(diagnostic)
            )))
        }
    }

    fn run_passes(
        module: LLVMModuleRef,
        target_machine: LLVMTargetMachineRef,
        level: OptimizationLevel,
    ) -> Result<(), LlvmError> {
        let pipeline = match level {
            OptimizationLevel::O0 => "default<O0>",
            OptimizationLevel::O2 => "default<O2>",
        };
        let pipeline = cstring(pipeline)?;
        // SAFETY: options is created and disposed in this function; module and
        // target machine are owned by Resources and remain live.
        let options = unsafe { LLVMCreatePassBuilderOptions() };
        if options.is_null() {
            return Err(LlvmError::Message(
                "LLVM failed to create pass-builder options".to_owned(),
            ));
        }
        // SAFETY: options and module/target machine are valid LLVM handles.
        let error = unsafe {
            LLVMPassBuilderOptionsSetVerifyEach(options, 1);
            LLVMRunPasses(module, pipeline.as_ptr(), target_machine, options)
        };
        // SAFETY: options was created above and has not been disposed yet.
        unsafe { LLVMDisposePassBuilderOptions(options) };
        if error.is_null() {
            Ok(())
        } else {
            // LLVMErrorRef is intentionally opaque; the C API owns its message
            // and requires explicit disposal of both handles.
            // SAFETY: LLVMGetErrorMessage/LLVMDisposeErrorMessage accept the
            // error returned by LLVMRunPasses. Keep the calls local to adapter.
            let text = unsafe {
                let message_pointer = llvm_sys::error::LLVMGetErrorMessage(error);
                if message_pointer.is_null() {
                    "LLVM pass pipeline failed".to_owned()
                } else {
                    let text = CStr::from_ptr(message_pointer)
                        .to_string_lossy()
                        .into_owned();
                    llvm_sys::error::LLVMDisposeErrorMessage(message_pointer);
                    text
                }
            };
            unsafe { llvm_sys::error::LLVMConsumeError(error) };
            Err(LlvmError::Message(text))
        }
    }

    struct Resources {
        context: LLVMContextRef,
        module: LLVMModuleRef,
        builder: llvm_sys::prelude::LLVMBuilderRef,
        target_machine: LLVMTargetMachineRef,
    }

    impl Drop for Resources {
        fn drop(&mut self) {
            // SAFETY: each handle is created at most once and disposed exactly
            // once here; null handles are skipped on partial construction.
            unsafe {
                if !self.builder.is_null() {
                    LLVMDisposeBuilder(self.builder);
                }
                if !self.module.is_null() {
                    LLVMDisposeModule(self.module);
                }
                if !self.target_machine.is_null() {
                    LLVMDisposeTargetMachine(self.target_machine);
                }
                if !self.context.is_null() {
                    LLVMContextDispose(self.context);
                }
            }
        }
    }

    /// Lowers the task-3 scalar seed program and emits a COFF object.
    ///
    /// # Errors
    ///
    /// Returns [`LlvmError`] if LLVM cannot create, verify, optimize, or emit
    /// the module, or if one of the supplied names contains a NUL byte.
    #[allow(clippy::too_many_lines)]
    pub fn emit_const_return_program(
        output: &Path,
        module_name: &str,
        value: i64,
        triple: &str,
        level: OptimizationLevel,
    ) -> Result<(), LlvmError> {
        initialize_x86_target();
        let module_name = cstring(module_name)?;
        let triple = cstring(triple)?;
        let source_file = cstring(module_name.to_string_lossy())?;
        let output = cstring(output.to_string_lossy())?;
        let i32_name = cstring("main")?;
        let program_name = cstring("keld_program_main")?;
        let print_name = cstring("keld_rt_v1_print_int")?;
        let generic_cpu = cstring("generic")?;
        let empty_features = cstring("")?;
        let context = unsafe { LLVMContextCreate() };
        if context.is_null() {
            return Err(LlvmError::Message(
                "LLVM context creation failed".to_owned(),
            ));
        }
        let module = unsafe { LLVMModuleCreateWithNameInContext(module_name.as_ptr(), context) };
        if module.is_null() {
            unsafe { LLVMContextDispose(context) };
            return Err(LlvmError::Message("LLVM module creation failed".to_owned()));
        }
        let builder = unsafe { LLVMCreateBuilderInContext(context) };
        if builder.is_null() {
            unsafe {
                LLVMDisposeModule(module);
                LLVMContextDispose(context);
            }
            return Err(LlvmError::Message(
                "LLVM builder creation failed".to_owned(),
            ));
        }
        let mut resources = Resources {
            context,
            module,
            builder,
            target_machine: null_mut(),
        };
        let mut target = null_mut();
        let mut target_error = null_mut();
        // SAFETY: triple and out pointers are valid for this call.
        let target_failed = unsafe {
            LLVMGetTargetFromTriple(triple.as_ptr(), &raw mut target, &raw mut target_error)
        };
        if target_failed != 0 || target.is_null() {
            return Err(LlvmError::Message(format!(
                "LLVM target lookup failed: {}",
                message(target_error)
            )));
        }
        let opt_level = match level {
            OptimizationLevel::O0 => LLVMCodeGenOptLevel::LLVMCodeGenLevelNone,
            OptimizationLevel::O2 => LLVMCodeGenOptLevel::LLVMCodeGenLevelDefault,
        };
        // SAFETY: target and all C strings are valid; the returned target
        // machine is owned by Resources.
        resources.target_machine = unsafe {
            LLVMCreateTargetMachine(
                target,
                triple.as_ptr(),
                generic_cpu.as_ptr(),
                empty_features.as_ptr(),
                opt_level,
                LLVMRelocMode::LLVMRelocDefault,
                LLVMCodeModel::LLVMCodeModelDefault,
            )
        };
        if resources.target_machine.is_null() {
            return Err(LlvmError::Message(
                "LLVM target-machine creation failed".to_owned(),
            ));
        }
        // SAFETY: module and triple handles remain live for this call.
        unsafe {
            LLVMSetTarget(resources.module, triple.as_ptr());
            LLVMSetSourceFileName(
                resources.module,
                source_file.as_ptr(),
                source_file.as_bytes().len(),
            );
        }
        let target_data = unsafe { LLVMCreateTargetDataLayout(resources.target_machine) };
        if target_data.is_null() {
            return Err(LlvmError::Message(
                "LLVM target data-layout creation failed".to_owned(),
            ));
        }
        // SAFETY: target_data and module are valid; the copied layout string is
        // disposed after LLVM copies it into the module.
        unsafe {
            let layout = LLVMCopyStringRepOfTargetData(target_data);
            if layout.is_null() {
                LLVMDisposeTargetData(target_data);
                return Err(LlvmError::Message(
                    "LLVM target data-layout string was null".to_owned(),
                ));
            }
            LLVMSetDataLayout(resources.module, layout);
            LLVMDisposeMessage(layout);
            LLVMDisposeTargetData(target_data);
        }
        // SAFETY: all type/value handles belong to this context/module.
        unsafe {
            let i64_type = LLVMInt64TypeInContext(resources.context);
            let i32_type = LLVMInt32TypeInContext(resources.context);
            let mut print_parameters = [i64_type];
            let print_type = LLVMFunctionType(i32_type, print_parameters.as_mut_ptr(), 1, 0);
            let print_function = LLVMAddFunction(resources.module, print_name.as_ptr(), print_type);
            let program_type = LLVMFunctionType(i64_type, null_mut(), 0, 0);
            let program_function =
                LLVMAddFunction(resources.module, program_name.as_ptr(), program_type);
            let program_block = LLVMAppendBasicBlockInContext(
                resources.context,
                program_function,
                cstring("entry")?.as_ptr(),
            );
            LLVMPositionBuilderAtEnd(resources.builder, program_block);
            let constant = LLVMConstInt(i64_type, value.cast_unsigned(), 1);
            LLVMBuildRet(resources.builder, constant);
            let main_type = LLVMFunctionType(i32_type, null_mut(), 0, 0);
            let main_function = LLVMAddFunction(resources.module, i32_name.as_ptr(), main_type);
            let main_block = LLVMAppendBasicBlockInContext(
                resources.context,
                main_function,
                cstring("entry_main")?.as_ptr(),
            );
            LLVMPositionBuilderAtEnd(resources.builder, main_block);
            let value = LLVMBuildCall2(
                resources.builder,
                program_type,
                program_function,
                null_mut(),
                0,
                cstring("value")?.as_ptr(),
            );
            let mut arguments = [value];
            LLVMBuildCall2(
                resources.builder,
                print_type,
                print_function,
                arguments.as_mut_ptr(),
                1,
                cstring("print")?.as_ptr(),
            );
            LLVMBuildRet(resources.builder, LLVMConstInt(i32_type, 0, 0));
        }
        verify(resources.module, "before optimization")?;
        run_passes(resources.module, resources.target_machine, level)?;
        verify(resources.module, "after optimization")?;
        let mut diagnostic = null_mut();
        // SAFETY: all handles and output path remain live for this call.
        let failed = unsafe {
            LLVMTargetMachineEmitToFile(
                resources.target_machine,
                resources.module,
                output.as_ptr(),
                LLVMCodeGenFileType::LLVMObjectFile,
                &raw mut diagnostic,
            )
        };
        if failed == 0 {
            Ok(())
        } else {
            Err(LlvmError::Message(format!(
                "LLVM object emission failed: {}",
                message(diagnostic)
            )))
        }
    }

    /// Returns textual IR for focused backend diagnostics.
    ///
    /// # Errors
    ///
    /// Returns [`LlvmError`] if LLVM cannot create or print the module, or if
    /// one of the supplied names contains a NUL byte.
    pub fn print_const_return_ir(
        module_name: &str,
        value: i64,
        triple: &str,
    ) -> Result<String, LlvmError> {
        initialize_x86_target();
        let module_name = cstring(module_name)?;
        let triple = cstring(triple)?;
        let context = unsafe { llvm_sys::core::LLVMContextCreate() };
        if context.is_null() {
            return Err(LlvmError::Message(
                "LLVM context creation failed".to_owned(),
            ));
        }
        let module = unsafe {
            llvm_sys::core::LLVMModuleCreateWithNameInContext(module_name.as_ptr(), context)
        };
        if module.is_null() {
            unsafe { llvm_sys::core::LLVMContextDispose(context) };
            return Err(LlvmError::Message("LLVM module creation failed".to_owned()));
        }
        unsafe {
            llvm_sys::core::LLVMSetTarget(module, triple.as_ptr());
            let i64_type = llvm_sys::core::LLVMInt64TypeInContext(context);
            let function_type = llvm_sys::core::LLVMFunctionType(i64_type, null_mut(), 0, 0);
            let function = llvm_sys::core::LLVMAddFunction(
                module,
                cstring("keld_program_main")?.as_ptr(),
                function_type,
            );
            let block = llvm_sys::core::LLVMAppendBasicBlockInContext(
                context,
                function,
                cstring("entry")?.as_ptr(),
            );
            let builder = llvm_sys::core::LLVMCreateBuilderInContext(context);
            llvm_sys::core::LLVMPositionBuilderAtEnd(builder, block);
            llvm_sys::core::LLVMBuildRet(
                builder,
                llvm_sys::core::LLVMConstInt(i64_type, value.cast_unsigned(), 1),
            );
            let pointer = LLVMPrintModuleToString(module);
            if pointer.is_null() {
                llvm_sys::core::LLVMDisposeBuilder(builder);
                llvm_sys::core::LLVMDisposeModule(module);
                llvm_sys::core::LLVMContextDispose(context);
                return Err(LlvmError::Message("LLVM IR printing failed".to_owned()));
            }
            let text = CStr::from_ptr(pointer).to_string_lossy().into_owned();
            LLVMDisposeMessage(pointer);
            llvm_sys::core::LLVMDisposeBuilder(builder);
            llvm_sys::core::LLVMDisposeModule(module);
            llvm_sys::core::LLVMContextDispose(context);
            Ok(text)
        }
    }
}

#[cfg(feature = "llvm")]
pub use enabled::{emit_const_return_program, print_const_return_ir};

#[cfg(not(feature = "llvm"))]
pub fn emit_const_return_program(
    _output: &Path,
    _module_name: &str,
    _value: i64,
    _triple: &str,
    _level: OptimizationLevel,
) -> Result<(), LlvmError> {
    Err(LlvmError::FeatureDisabled)
}

#[cfg(not(feature = "llvm"))]
pub fn print_const_return_ir(
    _module_name: &str,
    _value: i64,
    _triple: &str,
) -> Result<String, LlvmError> {
    Err(LlvmError::FeatureDisabled)
}
