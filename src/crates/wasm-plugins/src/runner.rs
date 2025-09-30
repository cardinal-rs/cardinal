use crate::instance::WasmInstance;
use crate::plugin::WasmPlugin;
use crate::ExecutionContext;
use cardinal_errors::internal::CardinalInternalError;
use cardinal_errors::CardinalError;
use std::collections::HashMap;
use std::sync::Arc;
use wasmer::TypedFunction;
use wasmer::{Function, FunctionEnv, Store};

#[derive(Debug, Copy, Clone)]
pub enum ExecutionType {
    Inbound,
    Outbound,
}

pub struct ExecutionResult {
    pub should_continue: bool,
    pub execution_context: ExecutionContext,
}

pub type HostFunctionBuilder =
    Arc<dyn Fn(&mut Store, &FunctionEnv<ExecutionContext>) -> Function + Send + Sync>;
pub type HostFunctionMap = HashMap<String, Vec<(String, HostFunctionBuilder)>>;

pub struct WasmRunner<'a> {
    pub plugin: &'a WasmPlugin,
    pub execution_type: ExecutionType,
    host_imports: Option<&'a HostFunctionMap>,
}

impl<'a> WasmRunner<'a> {
    pub fn new(
        plugin: &'a WasmPlugin,
        execution_type: ExecutionType,
        host_imports: Option<&'a HostFunctionMap>,
    ) -> Self {
        Self {
            plugin,
            execution_type,
            host_imports,
        }
    }

    pub fn run(&self, exec_ctx: ExecutionContext) -> Result<ExecutionResult, CardinalError> {
        // 1) Instantiate a fresh instance per request
        let mut instance =
            WasmInstance::from_plugin(self.plugin, self.execution_type, self.host_imports)?;

        {
            let ctx = instance.env.as_mut(&mut instance.store);
            match ctx {
                ExecutionContext::Inbound(inbound) => {
                    let inbound_ctx = exec_ctx.as_inbound().unwrap().to_owned();
                    inbound.req_headers = inbound_ctx.req_headers;
                    inbound.query = inbound_ctx.query;
                    inbound.body = inbound_ctx.body;
                }
                ExecutionContext::Outbound(outbound) => {
                    let inbound_ctx = exec_ctx.as_outbound().unwrap().to_owned();
                    outbound.req_headers = inbound_ctx.req_headers;
                    outbound.query = inbound_ctx.query;
                    outbound.body = inbound_ctx.body;
                    outbound.resp_headers = inbound_ctx.resp_headers;
                    outbound.status = inbound_ctx.status;
                }
            }
        }

        // 3) Get exports
        let handle: TypedFunction<(i32, i32), i32> = instance
            .instance
            .exports
            .get_typed_function(&instance.store, "handle")
            .map_err(|e| {
                CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                    "missing `handle` export {e}"
                )))
            })?;

        let alloc: TypedFunction<(i32, i32), i32> = instance
            .instance
            .exports
            .get_typed_function(&instance.store, "__new")
            .map_err(|e| {
                CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                    "missing `alloc` export {e}"
                )))
            })?;

        let body_opt = {
            let ctx_ref = instance.env.as_ref(&instance.store);
            ctx_ref.body().clone()
        };

        let (ptr, len) = if let Some(body) = body_opt.filter(|b| !b.is_empty()) {
            let len = body.len() as i32;

            let p = alloc.call(&mut instance.store, len, 0).map_err(|e| {
                CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                    "Alloc failed {e}"
                )))
            })?;

            {
                let view = instance.memory.view(&instance.store);
                view.write(p as u64, &body).map_err(|e| {
                    CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                        "Writing Body failed {e}"
                    )))
                })?;
            }

            (p, len)
        } else {
            (0, 0)
        };

        let decision = handle.call(&mut instance.store, ptr, len).map_err(|e| {
            CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                "WASM Handle call failed {e}"
            )))
        })?;

        Ok(ExecutionResult {
            should_continue: decision == 1,
            execution_context: instance.env.as_ref(&instance.store).clone(),
        })
    }
}
