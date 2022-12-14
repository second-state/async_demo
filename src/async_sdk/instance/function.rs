use crate::async_sdk::executor::Executor;
use crate::async_sdk::types::WasmVal;
use core::ffi::c_void;
use wasmedge_sys::ffi;
use wasmedge_types::error::{FuncError, WasmEdgeError};
use wasmedge_types::ValType;
use wasmedge_types::WasmEdgeResult;

extern "C" fn wraper_fn<T: Sized>(
    key_ptr: *mut c_void,
    data: *mut c_void,
    _mem_ctx: *mut ffi::WasmEdge_MemoryInstanceContext,
    params: *const ffi::WasmEdge_Value,
    param_len: u32,
    returns: *mut ffi::WasmEdge_Value,
    return_len: u32,
) -> ffi::WasmEdge_Result {
    let real_fn: fn(Option<&mut T>, &[WasmVal]) -> Result<Vec<WasmVal>, u32> =
        unsafe { std::mem::transmute(key_ptr) };

    let input = {
        let raw_input = unsafe { std::slice::from_raw_parts(params, param_len as usize) };
        raw_input
            .iter()
            .map(|r| (*r).into())
            .collect::<Vec<WasmVal>>()
    };

    let return_len = return_len as usize;
    let raw_returns = unsafe { std::slice::from_raw_parts_mut(returns, return_len) };

    let data = unsafe { (data as *mut T).as_mut() };

    let result = real_fn(data, &input);

    match result {
        Ok(v) => {
            assert!(v.len() == return_len);
            for (idx, item) in v.into_iter().enumerate() {
                raw_returns[idx] = item.into();
            }
            ffi::WasmEdge_Result { Code: 0 }
        }
        Err(c) => ffi::WasmEdge_Result { Code: c as u8 },
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Function {
    pub(crate) inner: InnerFunc,
}
impl Function {
    pub(crate) fn create<T: Sized>(
        ty: (Vec<ValType>, Vec<ValType>),
        real_fn: fn(Option<&mut T>, &[WasmVal]) -> Result<Vec<WasmVal>, u32>,
        data: *mut T,
        cost: u64,
    ) -> WasmEdgeResult<Self> {
        unsafe {
            let ty = FuncType::create(ty.0, ty.1)?;
            let ctx = ffi::WasmEdge_FunctionInstanceCreateBinding(
                ty.inner.0,
                Some(wraper_fn::<T>),
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

    pub fn func_type(&self) -> WasmEdgeResult<(Vec<ValType>, Vec<ValType>)> {
        let ty = unsafe { ffi::WasmEdge_FunctionInstanceGetFunctionType(self.inner.0 as *mut _) };
        if ty.is_null() {
            Err(WasmEdgeError::Func(FuncError::Type))
        } else {
            let ty = FuncType {
                inner: InnerFuncType(ty),
            };
            Ok((
                ty.params_type_iter().collect(),
                ty.returns_type_iter().collect(),
            ))
        }
    }

    pub fn func_param_size(&self) -> WasmEdgeResult<usize> {
        let ty = unsafe { ffi::WasmEdge_FunctionInstanceGetFunctionType(self.inner.0 as *mut _) };
        if ty.is_null() {
            Err(WasmEdgeError::Func(FuncError::Type))
        } else {
            let ty = FuncType {
                inner: InnerFuncType(ty),
            };
            Ok(ty.params_len() as usize)
        }
    }

    pub fn func_return_size(&self) -> WasmEdgeResult<usize> {
        let ty = unsafe { ffi::WasmEdge_FunctionInstanceGetFunctionType(self.inner.0 as *mut _) };
        if ty.is_null() {
            Err(WasmEdgeError::Func(FuncError::Type))
        } else {
            let ty = FuncType {
                inner: InnerFuncType(ty),
            };
            Ok(ty.returns_len() as usize)
        }
    }

    pub(crate) fn delete(self) {
        unsafe { ffi::WasmEdge_FunctionInstanceDelete(self.inner.0 as *mut _) }
    }
}

#[derive(Debug)]
pub(crate) struct InnerFunc(pub(crate) *const ffi::WasmEdge_FunctionInstanceContext);
impl Clone for InnerFunc {
    fn clone(&self) -> Self {
        InnerFunc(self.0)
    }
}
unsafe impl Send for InnerFunc {}
unsafe impl Sync for InnerFunc {}

#[derive(Debug)]
pub(crate) struct FuncType {
    pub(crate) inner: InnerFuncType,
}
impl FuncType {
    pub fn create<I: IntoIterator<Item = ValType>, R: IntoIterator<Item = ValType>>(
        args: I,
        returns: R,
    ) -> WasmEdgeResult<Self> {
        let param_tys = args
            .into_iter()
            .map(|x| x.into())
            .collect::<Vec<ffi::WasmEdge_ValType>>();
        let ret_tys = returns
            .into_iter()
            .map(|x| x.into())
            .collect::<Vec<ffi::WasmEdge_ValType>>();

        let ctx = unsafe {
            ffi::WasmEdge_FunctionTypeCreate(
                param_tys.as_ptr() as *const _,
                param_tys.len() as u32,
                ret_tys.as_ptr() as *const _,
                ret_tys.len() as u32,
            )
        };
        match ctx.is_null() {
            true => Err(WasmEdgeError::FuncTypeCreate),
            false => Ok(Self {
                inner: InnerFuncType(ctx),
            }),
        }
    }

    pub fn params_len(&self) -> u32 {
        unsafe { ffi::WasmEdge_FunctionTypeGetParametersLength(self.inner.0) }
    }

    pub fn params_type_iter(&self) -> impl Iterator<Item = ValType> {
        let len = self.params_len();
        let mut types = Vec::with_capacity(len as usize);
        unsafe {
            ffi::WasmEdge_FunctionTypeGetParameters(self.inner.0, types.as_mut_ptr(), len);
            types.set_len(len as usize);
        }

        types.into_iter().map(Into::into)
    }

    pub fn returns_len(&self) -> u32 {
        unsafe { ffi::WasmEdge_FunctionTypeGetReturnsLength(self.inner.0) }
    }

    pub fn returns_type_iter(&self) -> impl Iterator<Item = ValType> {
        let len = self.returns_len();
        let mut types = Vec::with_capacity(len as usize);
        unsafe {
            ffi::WasmEdge_FunctionTypeGetReturns(self.inner.0, types.as_mut_ptr(), len);
            types.set_len(len as usize);
        }

        types.into_iter().map(Into::into)
    }

    pub(crate) fn delete(self) {
        unsafe { ffi::WasmEdge_FunctionTypeDelete(self.inner.0 as *mut _) };
    }
}

impl From<wasmedge_types::FuncType> for FuncType {
    fn from(ty: wasmedge_types::FuncType) -> Self {
        let param_tys: Vec<_> = match ty.args() {
            Some(args) => args.to_vec(),
            None => Vec::new(),
        };
        let ret_tys: Vec<_> = match ty.returns() {
            Some(returns) => returns.to_vec(),
            None => Vec::new(),
        };

        FuncType::create(param_tys, ret_tys).expect("[wasmedge-sys] Failed to convert wasmedge_types::FuncType into wasmedge_sys::FuncType.")
    }
}
impl From<FuncType> for wasmedge_types::FuncType {
    fn from(ty: FuncType) -> Self {
        let args = if ty.params_len() > 0 {
            let mut args = Vec::with_capacity(ty.params_len() as usize);
            for ty in ty.params_type_iter() {
                args.push(ty);
            }
            Some(args)
        } else {
            None
        };

        let returns = if ty.returns_len() > 0 {
            let mut returns = Vec::with_capacity(ty.returns_len() as usize);
            for ty in ty.returns_type_iter() {
                returns.push(ty);
            }
            Some(returns)
        } else {
            None
        };

        wasmedge_types::FuncType::new(args, returns)
    }
}

#[derive(Debug)]
pub(crate) struct InnerFuncType(pub(crate) *const ffi::WasmEdge_FunctionTypeContext);
unsafe impl Send for InnerFuncType {}
unsafe impl Sync for InnerFuncType {}

#[derive(Debug, Clone)]
pub struct FuncRef {
    pub(crate) inner: InnerFunc,
}

impl FuncRef {
    pub fn func_type(&self) -> WasmEdgeResult<(Vec<ValType>, Vec<ValType>)> {
        let ty = unsafe { ffi::WasmEdge_FunctionInstanceGetFunctionType(self.inner.0 as *mut _) };
        if ty.is_null() {
            Err(WasmEdgeError::Func(FuncError::Type))
        } else {
            let ty = FuncType {
                inner: InnerFuncType(ty),
            };
            Ok((
                ty.params_type_iter().collect(),
                ty.returns_type_iter().collect(),
            ))
        }
    }

    pub fn func_param_size(&self) -> WasmEdgeResult<usize> {
        let ty = unsafe { ffi::WasmEdge_FunctionInstanceGetFunctionType(self.inner.0 as *mut _) };
        if ty.is_null() {
            Err(WasmEdgeError::Func(FuncError::Type))
        } else {
            let ty = FuncType {
                inner: InnerFuncType(ty),
            };
            Ok(ty.params_len() as usize)
        }
    }

    pub fn func_return_size(&self) -> WasmEdgeResult<usize> {
        let ty = unsafe { ffi::WasmEdge_FunctionInstanceGetFunctionType(self.inner.0 as *mut _) };
        if ty.is_null() {
            Err(WasmEdgeError::Func(FuncError::Type))
        } else {
            let ty = FuncType {
                inner: InnerFuncType(ty),
            };
            Ok(ty.returns_len() as usize)
        }
    }

    pub fn call(&self, engine: &mut Executor, args: &[WasmVal]) -> WasmEdgeResult<Vec<WasmVal>> {
        engine.run_func_ref(self, args)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct InnerFuncRef(pub(crate) *const ffi::WasmEdge_FunctionInstanceContext);
unsafe impl Send for InnerFuncRef {}
unsafe impl Sync for InnerFuncRef {}
