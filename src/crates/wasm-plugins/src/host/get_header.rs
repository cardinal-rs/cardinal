use crate::host::{read_key_lookup_and_write, HostImport};
use crate::SharedExecutionContext;
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Store};

pub(crate) struct GetHeaderImport;

impl HostImport for GetHeaderImport {
    fn namespace(&self) -> &str {
        "env"
    }

    fn name(&self) -> &str {
        "get_header"
    }

    fn build(&self, store: &mut Store, env: &FunctionEnv<SharedExecutionContext>) -> Function {
        Function::new_typed_with_env(store, env, get_header_raw)
    }
}

pub(crate) static GET_HEADER_IMPORT: GetHeaderImport = GetHeaderImport;

fn get_header_raw(
    ctx: FunctionEnvMut<SharedExecutionContext>,
    name_ptr: i32,
    name_len: i32,
    out_ptr: i32,
    out_cap: i32,
) -> i32 {
    read_key_lookup_and_write(
        &ctx,
        name_ptr,
        name_len,
        out_ptr,
        out_cap,
        false,
        |exec, key| exec.request().header_bytes(key),
    )
}
