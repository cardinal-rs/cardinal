use crate::host::HostImport;
use crate::utils::{read_bytes, with_mem_view};
use crate::SharedExecutionContext;
use http::{HeaderName, HeaderValue};
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Store};

pub(crate) struct SetHeaderImport;

impl HostImport for SetHeaderImport {
    fn namespace(&self) -> &str {
        "env"
    }

    fn name(&self) -> &str {
        "set_header"
    }

    fn build(&self, store: &mut Store, env: &FunctionEnv<SharedExecutionContext>) -> Function {
        Function::new_typed_with_env(store, env, set_header_raw)
    }
}

pub(crate) static SET_HEADER_IMPORT: SetHeaderImport = SetHeaderImport;

fn set_header_raw(
    ctx: FunctionEnvMut<SharedExecutionContext>,
    set_type: i32,
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

    let header_name = match HeaderName::from_bytes(name.as_bytes()) {
        Ok(n) => n,
        Err(_) => return,
    };
    let header_value = match HeaderValue::from_str(&value) {
        Ok(v) => v,
        Err(_) => return,
    };

    if set_type == 1 {
        let mut inner = ctx.data().write();
        inner
            .response_mut()
            .insert_header(header_name, header_value);
    } else if set_type == 0 {
        let mut inner = ctx.data().write();
        let _ = inner
            .request_mut()
            .headers_mut()
            .insert(header_name, header_value);
    }
}
