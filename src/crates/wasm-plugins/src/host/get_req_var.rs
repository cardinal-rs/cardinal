use crate::host::{read_key_lookup_and_write, read_key_lookup_and_write_ref};
use crate::{ExecutionContext, ExecutionContextCell};
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Store};

fn get_req_var_raw(
    ctx: FunctionEnvMut<ExecutionContextCell>,
    name_ptr: i32,
    name_len: i32,
    out_ptr: i32,
    out_cap: i32,
) -> i32 {
    let map = &ctx.data().inner.read();
    let inner_map = map.persistent_vars.read();
    read_key_lookup_and_write_ref(
        &ctx,
        name_ptr,
        name_len,
        out_ptr,
        out_cap,
        true,
        &*inner_map,
    )
}

pub fn get_req_var(store: &mut Store, env: &FunctionEnv<ExecutionContextCell>) -> Function {
    Function::new_typed_with_env(store, env, get_req_var_raw)
}
