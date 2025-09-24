use crate::host::read_key_lookup_and_write;
use crate::ExecutionContext;
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Store};

fn get_header_raw(
    ctx: FunctionEnvMut<ExecutionContext>,
    name_ptr: i32,
    name_len: i32,
    out_ptr: i32,
    out_cap: i32,
) -> i32 {
    read_key_lookup_and_write(&ctx, name_ptr, name_len, out_ptr, out_cap, true, |ecx| {
        &ecx.req_headers
    })
}

pub fn get_header(store: &mut Store, env: &FunctionEnv<ExecutionContext>) -> Function {
    Function::new_typed_with_env(store, env, get_header_raw)
}
