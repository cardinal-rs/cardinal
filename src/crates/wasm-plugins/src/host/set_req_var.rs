use crate::utils::{read_bytes, with_mem_view};
use crate::{ExecutionContext, ExecutionContextCell};
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Store};

fn set_req_var_raw(
    mut ctx: FunctionEnvMut<ExecutionContextCell>,
    name_ptr: i32,
    name_len: i32,
    val_ptr: i32,
    val_len: i32,
) {
    let view = match with_mem_view(&ctx) {
        Ok(v) => v,
        Err(_) => return,
    };

    let name = match String::from_utf8(read_bytes(&view, name_ptr, name_len).unwrap_or_default()) {
        Ok(s) => s,
        Err(_) => return,
    };
    let value = match String::from_utf8(read_bytes(&view, val_ptr, val_len).unwrap_or_default()) {
        Ok(s) => s,
        Err(_) => return,
    };

    let mut inner = ctx.data_mut().inner.write();
    inner.persistent_vars.write().insert(name, value);
}

pub fn set_req_var(store: &mut Store, env: &FunctionEnv<ExecutionContextCell>) -> Function {
    Function::new_typed_with_env(store, env, set_req_var_raw)
}
