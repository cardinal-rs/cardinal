use crate::host::{read_key_lookup_and_write, HostImport};
use crate::SharedExecutionContext;
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Store};

pub(crate) struct GetQueryParamImport;

impl HostImport for GetQueryParamImport {
    fn namespace(&self) -> &str {
        "env"
    }

    fn name(&self) -> &str {
        "get_query_param"
    }

    fn build(&self, store: &mut Store, env: &FunctionEnv<SharedExecutionContext>) -> Function {
        Function::new_typed_with_env(store, env, get_query_param_raw)
    }
}

pub(crate) static GET_QUERY_PARAM_IMPORT: GetQueryParamImport = GetQueryParamImport;

fn get_query_param_raw(
    ctx: FunctionEnvMut<SharedExecutionContext>,
    key_ptr: i32,
    key_len: i32,
    out_ptr: i32,
    out_cap: i32,
) -> i32 {
    read_key_lookup_and_write(
        &ctx,
        key_ptr,
        key_len,
        out_ptr,
        out_cap,
        true,
        |exec, key| {
            exec.request()
                .query_first(key)
                .map(|value| value.into_bytes())
        },
    )
}
