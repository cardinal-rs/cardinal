use crate::utils::{read_bytes, with_mem_view, write_bytes};
use crate::{ExecutionContext, ExecutionContextCell};
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Store};

fn get_query_param_raw(
    ctx: FunctionEnvMut<ExecutionContextCell>,
    key_ptr: i32,
    key_len: i32,
    out_ptr: i32,
    out_cap: i32,
) -> i32 {
    let view = match with_mem_view(&ctx) {
        Ok(v) => v,
        Err(_) => return -1,
    };

    let key = match String::from_utf8(read_bytes(&view, key_ptr, key_len).unwrap_or_default()) {
        Ok(s) => s.to_ascii_lowercase(),
        Err(_) => return -1,
    };

    let inner = ctx.data().inner.read();

    let values = match inner.query().get(&key) {
        Some(v) if !v.is_empty() => v,
        _ => return -1,
    };

    let value = values.first().expect("checked non-empty");
    let bytes = value.as_bytes();
    let n = bytes.len().min(out_cap as usize);

    if n > 0 && write_bytes(&view, out_ptr, &bytes[..n]).is_err() {
        return -1;
    }

    n as i32
}

pub fn get_query_param(store: &mut Store, env: &FunctionEnv<ExecutionContextCell>) -> Function {
    Function::new_typed_with_env(store, env, get_query_param_raw)
}
