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

    pub fn load_async_module_from_bytes(
        &self,
        wasm: &[u8],
        async_fn_names: &[&str],
    ) -> Result<AstModule, WasmEdgeError> {
        let mut codegen_config = binaryen::CodegenConfig::default();
        codegen_config.optimization_level = 2;
        // codegen_config.debug_info = true;

        let async_fn_name = async_fn_names.join(",");
        codegen_config
            .pass_argument
            .push(("asyncify-imports".to_string(), async_fn_name));

        let mut module = binaryen::Module::read(wasm).map_err(|_| WasmEdgeError::ModuleCreate)?;

        // pass start
        {
            if let Some(start) = module.get_start() {
                let global_ref = module.add_global("run_start", true, 0_i32).unwrap();
                let new_body = module.binaryen_if(
                    module.binaryen_get_global(global_ref),
                    start.body(),
                    module.binaryen_set_global(global_ref, module.binaryen_const_value(1_i32)),
                );
                start.set_body(new_body);
                module.add_function_export(&start, "async_start").unwrap();
            }
        }

        module
            .run_optimization_passes(["asyncify", "strip"], &codegen_config)
            .map_err(|_| WasmEdgeError::ModuleCreate)?;

        let new_wasm = module.write();
        self.load_module_from_bytes(&new_wasm)
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
