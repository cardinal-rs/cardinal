use crate::host::HostImport;
use crate::utils::{read_bytes, with_mem_view};
use crate::SharedExecutionContext;
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Store};

pub(crate) struct SetReqVarImport;

impl HostImport for SetReqVarImport {
    fn namespace(&self) -> &str {
        "env"
    }

    fn name(&self) -> &str {
        "set_req_var"
    }

    fn build(&self, store: &mut Store, env: &FunctionEnv<SharedExecutionContext>) -> Function {
        Function::new_typed_with_env(store, env, set_req_var_raw)
    }
}

pub(crate) static SET_REQ_VAR_IMPORT: SetReqVarImport = SetReqVarImport;

fn set_req_var_raw(
    ctx: FunctionEnvMut<SharedExecutionContext>,
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

    let inner = ctx.data().write();
    inner
        .persistent_vars()
        .write()
        .insert(name.to_ascii_lowercase(), value);
}
