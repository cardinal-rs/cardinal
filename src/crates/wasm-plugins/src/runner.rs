use crate::host::{HostFunctionBuilder, HostImportHandle};
use crate::instance::InstancePool;
use crate::plugin::WasmPlugin;
use crate::SharedExecutionContext;
use cardinal_errors::CardinalError;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionPhase {
    Inbound,
    Outbound,
}

#[derive(Debug)]
pub struct ExecutionResult {
    pub should_continue: bool,
    pub execution_context: SharedExecutionContext,
}

pub struct WasmRunner {
    pool: Arc<InstancePool>,
}

impl WasmRunner {
    pub fn new(
        plugin: &Arc<WasmPlugin>,
        phase: ExecutionPhase,
        host_imports: Option<&[HostImportHandle]>,
    ) -> Self {
        let dynamic = host_imports
            .map(|imports| imports.iter().cloned().collect())
            .unwrap_or_else(Vec::new);

        let pool = InstancePool::new(plugin.clone(), phase, dynamic);
        Self {
            pool: Arc::new(pool),
        }
    }

    pub fn run(
        &self,
        shared_ctx: SharedExecutionContext,
    ) -> Result<ExecutionResult, CardinalError> {
        let mut guard = self.pool.acquire(shared_ctx.clone())?;
        let instance = guard.instance();

        let body = shared_ctx.read().request().body().cloned();
        let body_slice = body.as_ref().map(|bytes| bytes.as_ref());

        let (ptr, len) = instance.write_body(body_slice)?;
        let decision = instance.call_handle(ptr, len)?;

        Ok(ExecutionResult {
            should_continue: decision == 1,
            execution_context: shared_ctx,
        })
    }
}

pub fn host_import_from_builder<N, S>(
    namespace: N,
    name: S,
    builder: HostFunctionBuilder,
) -> HostImportHandle
where
    N: Into<String>,
    S: Into<String>,
{
    Arc::new(crate::host::DynamicHostImport::new(
        namespace, name, builder,
    ))
}
