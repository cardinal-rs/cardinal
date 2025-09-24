use crate::instance::WasmInstance;
use crate::plugin::WasmPlugin;
use crate::ExecutionContext;
use cardinal_errors::internal::CardinalInternalError;
use cardinal_errors::CardinalError;
use wasmer::TypedFunction;

pub struct ExecutionResult {
    pub decision: bool,
    pub execution_context: ExecutionContext,
}

pub struct WasmRunner<'a> {
    pub plugin: &'a WasmPlugin,
}

impl<'a> WasmRunner<'a> {
    pub fn new(plugin: &'a WasmPlugin) -> Self {
        Self { plugin }
    }

    pub fn run(&self, exec_ctx: ExecutionContext) -> Result<ExecutionResult, CardinalError> {
        // 1) Instantiate a fresh instance per request
        let mut instance = WasmInstance::from_plugin(self.plugin)?;

        for name in instance.instance.exports.iter().map(|e| e.0.to_string()) {
            eprintln!("export: {}", name);
        }

        {
            let ctx = instance.env.as_mut(&mut instance.store);
            ctx.req_headers = exec_ctx.req_headers;
            ctx.resp_headers = exec_ctx.resp_headers;
            ctx.query = exec_ctx.query;
            ctx.body = exec_ctx.body;
            ctx.status = 200;
        }

        // 3) Get exports
        let handle: TypedFunction<(i32, i32), i32> = instance
            .instance
            .exports
            .get_typed_function(&instance.store, "handle")
            .map_err(|e| {
                CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                    "missing `handle` export {}",
                    e.to_string()
                )))
            })?;

        let alloc: TypedFunction<(i32, i32), i32> = instance
            .instance
            .exports
            .get_typed_function(&instance.store, "__new")
            .map_err(|e| {
                CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                    "missing `alloc` export {}",
                    e.to_string()
                )))
            })?;

        let body_opt = {
            let ctx_ref = instance.env.as_ref(&instance.store);
            ctx_ref.body.clone()
        };

        let (ptr, len) = if let Some(body) = body_opt.filter(|b| !b.is_empty()) {
            let len = body.len() as i32;

            let p = alloc.call(&mut instance.store, len, 0).map_err(|e| {
                CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                    "Alloc failed {}",
                    e.to_string()
                )))
            })?;

            {
                let view = instance.memory.view(&instance.store);
                view.write(p as u64, &body).map_err(|e| {
                    CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                        "Writing Body failed {}",
                        e.to_string()
                    )))
                })?;
            }

            (p, len)
        } else {
            (0, 0)
        };

        let decision = handle.call(&mut instance.store, ptr, len).map_err(|e| {
            CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                "WASM Handle call failed {}",
                e.to_string()
            )))
        })?;

        Ok(ExecutionResult {
            decision: decision == 1,
            execution_context: instance.env.as_ref(&instance.store).clone(),
        })
    }
}
