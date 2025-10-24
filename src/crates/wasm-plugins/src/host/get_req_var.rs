use crate::host::{read_key_lookup_and_write, HostImport};
use crate::SharedExecutionContext;
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Store};

pub(crate) struct GetReqVarImport;

impl HostImport for GetReqVarImport {
    fn namespace(&self) -> &str {
        "env"
    }

    fn name(&self) -> &str {
        "get_req_var"
    }

    fn build(&self, store: &mut Store, env: &FunctionEnv<SharedExecutionContext>) -> Function {
        Function::new_typed_with_env(store, env, get_req_var_raw)
    }
}

pub(crate) static GET_REQ_VAR_IMPORT: GetReqVarImport = GetReqVarImport;

fn get_req_var_raw(
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
        true,
        |exec, key| {
            exec.persistent_vars()
                .read()
                .get(key)
                .map(|value| value.as_bytes().to_vec())
        },
    )
}
