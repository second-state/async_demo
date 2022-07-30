use std::future::Future;

use self::{config::Config, executor::Executor, types::WasmVal};

pub mod ast_module;
pub mod config;
pub mod executor;
pub mod instance;
pub mod module;
pub mod types;
pub(crate) mod utils;

pub use ast_module::*;
pub use module::*;
use wasmedge_types::error;
use wasmedge_types::{error::WasmEdgeError, ValType, WasmEdgeResult};

pub struct Linker {
    pub(crate) inst: Option<module::Instance>,
    pub(crate) executor: Executor,
}

impl Linker {
    pub fn new(config: &Option<Config>) -> WasmEdgeResult<Box<Self>> {
        Ok(Box::new(Linker {
            executor: Executor::create(config)?,
            inst: None,
        }))
    }

    pub fn get_memory_slice<'a>(
        &'a self,
        name: &str,
        offset: usize,
        len: usize,
    ) -> WasmEdgeResult<&'a [u8]> {
        let mem = if let Some(inst) = &self.inst {
            inst.get_memory(name)
        } else {
            Err(WasmEdgeError::Instance(error::InstanceError::NotFoundMem(
                name.to_string(),
            )))
        }?;
        unsafe {
            let p = mem.data_pointer_raw(offset, len)?;
            Ok(std::slice::from_raw_parts(p, len))
        }
    }

    pub fn mut_memory_slice<'a>(
        &'a mut self,
        name: &str,
        offset: usize,
        len: usize,
    ) -> WasmEdgeResult<&'a mut [u8]> {
        let mut mem = if let Some(inst) = &self.inst {
            inst.get_memory(name)
        } else {
            Err(WasmEdgeError::Instance(error::InstanceError::NotFoundMem(
                name.to_string(),
            )))
        }?;
        unsafe {
            let p = mem.data_pointer_mut_raw(offset, len)?;
            Ok(std::slice::from_raw_parts_mut(p, len))
        }
    }

    pub fn run(&mut self, name: &str, args: &[WasmVal]) -> WasmEdgeResult<Vec<WasmVal>> {
        let f = if let Some(inst) = &self.inst {
            inst.get_func(name)
        } else {
            Err(WasmEdgeError::Instance(error::InstanceError::NotFoundFunc(
                name.to_string(),
            )))
        }?;
        f.call(&mut self.executor, args)
    }
}

pub struct ImportModuleBuilder {
    import_obj: ImportModule,
    linker_ctx: *mut Linker,
}

impl ImportModuleBuilder {
    pub fn add_func(
        &mut self,
        name: &str,
        ty: (Vec<ValType>, Vec<ValType>),
        real_fn: fn(Option<&mut Linker>, &[WasmVal]) -> Result<Vec<WasmVal>, u32>,
        cost: u64,
    ) -> WasmEdgeResult<()> {
        self.import_obj
            .add_func(name, self.linker_ctx, ty, real_fn, cost)
    }
}

pub trait AsLinker {
    fn new_import_object<F: FnMut(&mut ImportModuleBuilder) -> Result<(), WasmEdgeError>>(
        &mut self,
        name: &str,
        f: &mut F,
    ) -> Result<(), WasmEdgeError>;

    fn active_module(&mut self, ast_module: &AstModule) -> Result<(), WasmEdgeError>;
}

impl AsLinker for Box<Linker> {
    fn new_import_object<F: FnMut(&mut ImportModuleBuilder) -> Result<(), WasmEdgeError>>(
        &mut self,
        name: &str,
        f: &mut F,
    ) -> Result<(), WasmEdgeError> {
        let mut builder = ImportModuleBuilder {
            import_obj: ImportModule::create(name)?,
            linker_ctx: self.as_mut() as *mut Linker,
        };
        f(&mut builder)?;
        self.executor.register_import_object(builder.import_obj)?;
        Ok(())
    }

    fn active_module(&mut self, module: &AstModule) -> Result<(), WasmEdgeError> {
        let inst = self.executor.register_active_module(module)?;
        self.inst = Some(inst);
        Ok(())
    }
}

pub mod async_mod {
    use std::{
        ffi::c_void,
        future::Future,
        pin::Pin,
        task::{Context, Poll, Waker},
        time::Duration,
    };
    use wasmedge_sys::ffi;
    use wasmedge_types::{
        error::{FuncError, WasmEdgeError},
        ValType, WasmEdgeResult,
    };

    use super::{
        config::Config,
        instance::function::{FuncType, Function, InnerFunc},
        types::{WasmEdgeString, WasmVal},
        AsLinker, AstModule, ImportModule, Linker,
    };

    pub type ResultFuture = Pin<Box<dyn Future<Output = WasmEdgeResult<Vec<WasmVal>>>>>;

    pub struct WasmEdgeResultFuture<'a> {
        linker: &'a mut AsyncLink,
        name: String,
        args: Vec<WasmVal>,
    }

    impl Future for WasmEdgeResultFuture<'_> {
        type Output = WasmEdgeResult<Vec<WasmVal>>;

        fn poll(
            self: std::pin::Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            let WasmEdgeResultFuture { linker, name, args } = self.get_mut();
            linker.cx = cx.waker().clone();

            linker.asyncify_resume();
            let r = linker.real_call(name, args);
            let r = if r.is_err() {
                Poll::Ready(r)
            } else {
                if linker.asyncify_done() {
                    Poll::Ready(r)
                } else {
                    Poll::Pending
                }
            };
            if r.is_ready() {
                linker.asyncify_normal();
            }
            r
        }
    }

    extern "C" fn wrapper_async_fn(
        key_ptr: *mut c_void,
        data: *mut c_void,
        _mem_ctx: *mut ffi::WasmEdge_MemoryInstanceContext,
        params: *const ffi::WasmEdge_Value,
        param_len: u32,
        returns: *mut ffi::WasmEdge_Value,
        return_len: u32,
    ) -> ffi::WasmEdge_Result {
        let data = unsafe { (data as *mut AsyncLink).as_mut() };
        if let Some(data) = data {
            let cx = data.cx.clone();
            let mut cx = Context::from_waker(&cx);

            let mut fut = if data.asyncify_done() {
                let real_fn: fn(&mut AsyncLink, &[WasmVal]) -> ResultFuture =
                    unsafe { std::mem::transmute(key_ptr) };

                let input = {
                    let raw_input =
                        unsafe { std::slice::from_raw_parts(params, param_len as usize) };
                    raw_input
                        .iter()
                        .map(|r| (*r).into())
                        .collect::<Vec<WasmVal>>()
                };

                real_fn(data, &input)
            } else {
                data.func_futures.pop_back().unwrap()
            };

            let return_len = return_len as usize;
            let raw_returns = unsafe { std::slice::from_raw_parts_mut(returns, return_len) };

            match Future::poll(fut.as_mut(), &mut cx) {
                std::task::Poll::Ready(result) => match result {
                    Ok(v) => {
                        data.asyncify_normal();

                        assert!(v.len() == return_len);
                        for (idx, item) in v.into_iter().enumerate() {
                            raw_returns[idx] = item.into();
                        }
                        ffi::WasmEdge_Result { Code: 0 }
                    }
                    Err(c) => ffi::WasmEdge_Result { Code: 64 },
                },
                std::task::Poll::Pending => {
                    data.asyncify_interrupt();
                    data.func_futures.push_back(fut);
                    ffi::WasmEdge_Result { Code: 0 }
                }
            }
        } else {
            ffi::WasmEdge_Result { Code: 0 }
        }
    }

    pub trait AsAsyncLinker {
        fn new_import_object<F: FnMut(&mut AsyncImportModuleBuilder) -> Result<(), WasmEdgeError>>(
            &mut self,
            name: &str,
            f: &mut F,
        ) -> Result<(), WasmEdgeError>;

        fn active_module(&mut self, ast_module: &AstModule) -> Result<(), WasmEdgeError>;
    }

    impl AsAsyncLinker for Box<AsyncLink> {
        fn new_import_object<
            F: FnMut(&mut AsyncImportModuleBuilder) -> Result<(), WasmEdgeError>,
        >(
            &mut self,
            name: &str,
            f: &mut F,
        ) -> Result<(), WasmEdgeError> {
            let mut builder = AsyncImportModuleBuilder {
                import_obj: ImportModule::create(name)?,
                linker_ctx: self,
            };
            f(&mut builder)?;
            let AsyncImportModuleBuilder {
                import_obj,
                linker_ctx,
            } = builder;
            linker_ctx
                .real_linker
                .executor
                .register_import_object(import_obj)?;
            Ok(())
        }

        fn active_module(&mut self, module: &AstModule) -> Result<(), WasmEdgeError> {
            self.real_linker.active_module(module)
        }
    }

    pub struct AsyncLink {
        cx: Waker,
        real_linker: Box<Linker>,
        func_futures: std::collections::LinkedList<ResultFuture>,
    }

    impl AsyncLink {
        pub fn new(config: &Option<Config>) -> WasmEdgeResult<Box<Self>> {
            Ok(Box::new(AsyncLink {
                cx: waker_fn::waker_fn(|| {}),
                real_linker: Linker::new(config)?,
                func_futures: Default::default(),
            }))
        }

        pub fn call(&mut self, name: &str, args: Vec<WasmVal>) -> WasmEdgeResultFuture {
            WasmEdgeResultFuture {
                linker: self,
                name: name.to_string(),
                args,
            }
        }

        fn real_call(&mut self, name: &str, args: &[WasmVal]) -> WasmEdgeResult<Vec<WasmVal>> {
            self.real_linker.run(name, args)
        }

        fn asyncify_interrupt(&mut self) {
            self.real_call("asyncify_start_unwind", &[]).unwrap();
        }

        fn asyncify_resume(&mut self) {
            if !self.asyncify_done() {
                self.real_call("asyncify_start_rewind", &[]).unwrap();
            }
        }

        fn asyncify_normal(&mut self) {
            self.real_call("asyncify_stop_unwind", &[]).unwrap();
        }

        fn asyncify_done(&mut self) -> bool {
            let r = self.real_call("asyncify_get_state", &[]);
            if let Ok(s) = r {
                if let Some(WasmVal::I32(i)) = s.first() {
                    return *i == 0;
                }
            }
            return true;
        }
    }

    pub struct AsyncImportModuleBuilder<'a> {
        import_obj: ImportModule,
        linker_ctx: &'a mut AsyncLink,
    }

    impl AsyncImportModuleBuilder<'_> {
        pub fn add_func(
            &mut self,
            name: &str,
            ty: (Vec<ValType>, Vec<ValType>),
            real_fn: fn(linker: &'static mut AsyncLink, args: &[WasmVal]) -> ResultFuture,
            cost: u64,
        ) -> WasmEdgeResult<()> {
            self.import_obj
                .add_async_func(name, self.linker_ctx, ty, real_fn, cost)
        }
    }

    impl ImportModule {
        pub fn add_async_func(
            &mut self,
            name: &str,
            data: &mut AsyncLink,
            ty: (Vec<ValType>, Vec<ValType>),
            real_fn: fn(&'static mut AsyncLink, &[WasmVal]) -> ResultFuture,
            cost: u64,
        ) -> WasmEdgeResult<()> {
            let func_name = WasmEdgeString::new(name);
            unsafe {
                let func = Function::create_async(ty, real_fn, data, cost)?;
                ffi::WasmEdge_ModuleInstanceAddFunction(
                    self.inner.0,
                    func_name.as_raw(),
                    func.inner.0 as *mut _,
                );
                Ok(())
            }
        }
    }

    impl Function {
        pub(crate) fn create_async<T: Sized>(
            ty: (Vec<ValType>, Vec<ValType>),
            real_fn: fn(&'static mut AsyncLink, &[WasmVal]) -> ResultFuture,
            data: *mut T,
            cost: u64,
        ) -> WasmEdgeResult<Self> {
            unsafe {
                let ty = FuncType::create(ty.0, ty.1)?;
                let ctx = ffi::WasmEdge_FunctionInstanceCreateBinding(
                    ty.inner.0,
                    Some(wrapper_async_fn),
                    real_fn as *mut c_void,
                    data.cast(),
                    cost,
                );
                ty.delete();

                match ctx.is_null() {
                    true => Err(WasmEdgeError::Func(FuncError::Create)),
                    false => Ok(Self {
                        inner: InnerFunc(ctx),
                    }),
                }
            }
        }
    }

    fn linker_sleep(linker: &'static mut AsyncLink, args: &[WasmVal]) -> ResultFuture {
        Box::pin(async {
            println!(" sleep... {}", chrono::Utc::now());
            linker.call("call_sleep1", vec![]).await?;
            let timeout = Duration::from_secs(2);
            tokio::time::sleep(timeout).await;
            println!(" sleep awake! {}", chrono::Utc::now());
            Ok(vec![])
        })
    }

    fn linker_sleep1(linker: &'static mut AsyncLink, args: &[WasmVal]) -> ResultFuture {
        Box::pin(async {
            println!("  sleep1... {}", chrono::Utc::now());
            let timeout = Duration::from_secs(2);
            tokio::time::sleep(timeout).await;
            println!("  sleep1 awake! {}", chrono::Utc::now());
            Ok(vec![])
        })
    }

    fn linker_prn(linker: &'static mut AsyncLink, args: &[WasmVal]) -> ResultFuture {
        let args = args.to_vec();

        Box::pin(async move {
            println!("=> print {:?}", args);
            Ok(vec![])
        })
    }

    pub fn try_(config: &Option<Config>, ast_module: AstModule) {
        println!("start try");
        let mut vm = AsyncLink::new(&config).unwrap();

        vm.new_import_object("spectest", &mut |builder| {
            builder.add_func("sleep", (vec![], vec![]), linker_sleep, 0)?;
            builder.add_func("sleep1", (vec![], vec![]), linker_sleep1, 0)?;
            builder.add_func("print", (vec![ValType::I32], vec![]), linker_prn, 0)?;
            Ok(())
        })
        .unwrap();

        vm.active_module(&ast_module).unwrap();

        println!("init_module ok");
        println!();

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        runtime.block_on(async move {
            vm.call("_start", vec![]).await.unwrap();
        })
    }
}
