use wasmedge_types::error::WasmEdgeError;

use super::{config::Config, utils::check};
use wasmedge_sys::ffi;

pub struct Loader {
    pub(crate) loader_inner: InnerLoader,
    pub(crate) validator_inner: InnerValidator,
}

pub(crate) struct InnerLoader(pub(crate) *mut ffi::WasmEdge_LoaderContext);
impl Drop for InnerLoader {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { ffi::WasmEdge_LoaderDelete(self.0) }
        }
    }
}
unsafe impl Send for InnerLoader {}
unsafe impl Sync for InnerLoader {}

impl Loader {
    pub fn create(config: &Option<Config>) -> Result<Self, WasmEdgeError> {
        unsafe {
            let config_ctx = if let Some(c) = config {
                c.inner.0
            } else {
                std::ptr::null()
            };

            let loader_inner = ffi::WasmEdge_LoaderCreate(config_ctx);
            if loader_inner.is_null() {
                return Err(WasmEdgeError::LoaderCreate);
            }

            let validator_inner = ffi::WasmEdge_ValidatorCreate(config_ctx);
            if validator_inner.is_null() {
                return Err(WasmEdgeError::ValidatorCreate);
            }
            Ok(Self {
                loader_inner: InnerLoader(loader_inner),
                validator_inner: InnerValidator(validator_inner),
            })
        }
    }
    pub fn load_module_from_bytes(&self, wasm: &[u8]) -> Result<AstModule, WasmEdgeError> {
        unsafe {
            let mut mod_ctx: *mut ffi::WasmEdge_ASTModuleContext = std::ptr::null_mut();

            check(ffi::WasmEdge_LoaderParseFromBuffer(
                self.loader_inner.0,
                &mut mod_ctx,
                wasm.as_ptr(),
                wasm.len() as u32,
            ))?;

            if mod_ctx.is_null() {
                return Err(WasmEdgeError::ModuleCreate);
            }

            check(ffi::WasmEdge_ValidatorValidate(
                self.validator_inner.0,
                mod_ctx,
            ))?;

            Ok(AstModule {
                inner: InnerModule(mod_ctx),
            })
        }
    }
}

pub(crate) struct InnerValidator(pub(crate) *mut ffi::WasmEdge_ValidatorContext);
impl Drop for InnerValidator {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { ffi::WasmEdge_ValidatorDelete(self.0) }
        }
    }
}
unsafe impl Send for InnerValidator {}
unsafe impl Sync for InnerValidator {}

#[derive(Debug)]
pub struct AstModule {
    pub(crate) inner: InnerModule,
}

impl AstModule {
    pub fn create_from_wasm(loader: &Loader, wasm: &[u8]) -> Result<Self, WasmEdgeError> {
        loader.load_module_from_bytes(wasm)
    }
}

#[derive(Debug)]
pub(crate) struct InnerModule(pub(crate) *mut ffi::WasmEdge_ASTModuleContext);
impl Drop for InnerModule {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { ffi::WasmEdge_ASTModuleDelete(self.0) };
        }
    }
}
unsafe impl Send for InnerModule {}
unsafe impl Sync for InnerModule {}
