use crate::ExecutionContext;
use wasmer::{Function, FunctionEnv, FunctionEnvMut, Store};

fn set_status_raw(mut ctx: FunctionEnvMut<ExecutionContext>, code: i32) {
    ctx.data_mut().status = code;
}

pub fn set_status(store: &mut Store, env: &FunctionEnv<ExecutionContext>) -> Function {
    Function::new_typed_with_env(store, env, set_status_raw)
}
