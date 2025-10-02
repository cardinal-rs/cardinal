use crate::ExecutionContextCell;
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Store};

fn set_status_raw(mut ctx: FunctionEnvMut<ExecutionContextCell>, code: i32) {
    if let Ok(status) = u16::try_from(code) {
        if (100..=599).contains(&status) {
            let mut inner = ctx.data_mut().inner.write();
            inner.response_mut().set_status(status);
        }
    }
}

pub fn set_status(store: &mut Store, env: &FunctionEnv<ExecutionContextCell>) -> Function {
    Function::new_typed_with_env(store, env, set_status_raw)
}
