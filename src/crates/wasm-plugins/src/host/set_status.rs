use crate::ExecutionContext;
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Store};

fn set_status_raw(mut ctx: FunctionEnvMut<ExecutionContext>, code: i32) {
    if let Ok(status) = u16::try_from(code) {
        if (100..=599).contains(&status) {
            ctx.data_mut().response_mut().set_status(status);
        }
    }
}

pub fn set_status(store: &mut Store, env: &FunctionEnv<ExecutionContext>) -> Function {
    Function::new_typed_with_env(store, env, set_status_raw)
}
