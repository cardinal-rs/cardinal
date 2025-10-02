use crate::utils::{read_bytes, with_mem_view};
use crate::{ExecutionContext, ExecutionContextCell};
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Store};

pub fn abort(store: &mut Store, env: &FunctionEnv<ExecutionContextCell>) -> Function {
    let env = env.clone();
    Function::new_typed_with_env(
        store,
        &env,
        move |ctx: FunctionEnvMut<ExecutionContextCell>,
              msg_ptr: i32,
              file_ptr: i32,
              line: i32,
              col: i32| {
            // Try to decode message and file strings
            if let Ok(view) = with_mem_view(&ctx) {
                let msg = String::from_utf8(read_bytes(&view, msg_ptr, 256).unwrap_or_default())
                    .unwrap_or_default();
                let file = String::from_utf8(read_bytes(&view, file_ptr, 256).unwrap_or_default())
                    .unwrap_or_default();
                eprintln!("[wasm abort] {file}:{line}:{col}  {msg}");
            } else {
                eprintln!("[wasm abort] (unable to read strings) line={line} col={col}");
            }
        },
    )
}
