use crate::utils::{read_bytes, with_mem_view};
use crate::ExecutionContext;
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Store};

fn set_header_raw(
    mut ctx: FunctionEnvMut<ExecutionContext>,
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

    ctx.data_mut()
        .response_mut()
        .headers_mut()
        .insert(name, value);
}

pub fn set_header(store: &mut Store, env: &FunctionEnv<ExecutionContext>) -> Function {
    Function::new_typed_with_env(store, env, set_header_raw)
}
