use crate::host::HostImport;
use crate::utils::{read_bytes, with_mem_view};
use crate::SharedExecutionContext;
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Store};

pub(crate) struct AbortImport;

impl HostImport for AbortImport {
    fn namespace(&self) -> &str {
        "env"
    }

    fn name(&self) -> &str {
        "abort"
    }

    fn build(&self, store: &mut Store, env: &FunctionEnv<SharedExecutionContext>) -> Function {
        build_abort(store, env)
    }
}

pub(crate) static ABORT_IMPORT: AbortImport = AbortImport;

fn build_abort(store: &mut Store, env: &FunctionEnv<SharedExecutionContext>) -> Function {
    let env = env.clone();
    Function::new_typed_with_env(
        store,
        &env,
        move |ctx: FunctionEnvMut<SharedExecutionContext>,
              msg_ptr: i32,
              file_ptr: i32,
              line: i32,
              col: i32| {
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
