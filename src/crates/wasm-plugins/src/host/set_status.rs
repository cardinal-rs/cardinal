use crate::host::HostImport;
use crate::SharedExecutionContext;
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Store};

pub(crate) struct SetStatusImport;

impl HostImport for SetStatusImport {
    fn namespace(&self) -> &str {
        "env"
    }

    fn name(&self) -> &str {
        "set_status"
    }

    fn build(&self, store: &mut Store, env: &FunctionEnv<SharedExecutionContext>) -> Function {
        Function::new_typed_with_env(store, env, set_status_raw)
    }
}

pub(crate) static SET_STATUS_IMPORT: SetStatusImport = SetStatusImport;

fn set_status_raw(ctx: FunctionEnvMut<SharedExecutionContext>, code: i32) {
    if let Ok(status) = u16::try_from(code) {
        if (100..=599).contains(&status) {
            let mut inner = ctx.data().write();
            inner.response_mut().set_status(status);
        }
    }
}
