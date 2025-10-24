use crate::context::ExecutionContext;
use crate::host::{make_imports, HostImportHandle};
use crate::plugin::WasmPlugin;
use crate::runner::ExecutionPhase;
use crate::SharedExecutionContext;
use cardinal_errors::internal::CardinalInternalError;
use cardinal_errors::CardinalError;
use parking_lot::Mutex;
use std::sync::Arc;
use wasmer::{FunctionEnv, Instance, Memory, Store, TypedFunction};

const ALLOC_FUNC: &str = "__new";

pub struct InstancePool {
    plugin: Arc<WasmPlugin>,
    phase: ExecutionPhase,
    dynamic_imports: Arc<Vec<HostImportHandle>>,
    instances: Mutex<Vec<PreparedInstance>>,
}

impl InstancePool {
    pub fn new(
        plugin: Arc<WasmPlugin>,
        phase: ExecutionPhase,
        dynamic_imports: Vec<HostImportHandle>,
    ) -> Self {
        Self {
            plugin,
            phase,
            dynamic_imports: Arc::new(dynamic_imports),
            instances: Mutex::new(Vec::new()),
        }
    }

    pub fn acquire(&self, ctx: SharedExecutionContext) -> Result<InstanceGuard<'_>, CardinalError> {
        let mut pooled = self.instances.lock();
        let mut instance = pooled.pop();
        drop(pooled);

        if instance.is_none() {
            instance = Some(self.instantiate()?);
        }

        let mut instance = instance.expect("instance must be present");
        instance.activate(ctx);

        Ok(InstanceGuard {
            pool: self,
            instance: Some(instance),
        })
    }

    fn instantiate(&self) -> Result<PreparedInstance, CardinalError> {
        let mut store = Store::new(self.plugin.engine.clone());
        let placeholder_ctx = Arc::new(parking_lot::RwLock::new(ExecutionContext::default()));
        let env = FunctionEnv::new(&mut store, placeholder_ctx.clone());

        let imports = make_imports(&mut store, &env, self.phase, self.dynamic_imports.as_ref());

        let instance = Instance::new(&mut store, &self.plugin.module, &imports).map_err(|e| {
            CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                "Error creating WASM Instance {e}"
            )))
        })?;

        let memory_name = self.plugin.memory_name.as_str();
        let memory = instance
            .exports
            .get_memory(memory_name)
            .map_err(|e| {
                CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                    "missing memory export `{memory_name}`: {e}"
                )))
            })?
            .clone();

        initialize_placeholder_memory(&env, &mut store, memory.clone());

        let handle = instance
            .exports
            .get_typed_function::<(i32, i32), i32>(&store, self.plugin.handle_name.as_str())
            .map_err(|e| {
                CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                    "missing `{}` export {e}",
                    self.plugin.handle_name
                )))
            })?;

        let allocator = instance
            .exports
            .get_typed_function::<(i32, i32), i32>(&store, ALLOC_FUNC)
            .map_err(|e| {
                CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                    "missing `{ALLOC_FUNC}` export {e}"
                )))
            })?;

        Ok(PreparedInstance {
            store,
            _instance: instance,
            memory,
            env,
            handle,
            allocator,
        })
    }
}

pub struct InstanceGuard<'a> {
    pool: &'a InstancePool,
    instance: Option<PreparedInstance>,
}

impl<'a> InstanceGuard<'a> {
    pub fn instance(&mut self) -> &mut PreparedInstance {
        self.instance.as_mut().expect("instance should be present")
    }
}

impl Drop for InstanceGuard<'_> {
    fn drop(&mut self) {
        if let Some(instance) = self.instance.take() {
            let mut pooled = self.pool.instances.lock();
            pooled.push(instance);
        }
    }
}

pub struct PreparedInstance {
    store: Store,
    _instance: Instance,
    memory: Memory,
    env: FunctionEnv<SharedExecutionContext>,
    handle: TypedFunction<(i32, i32), i32>,
    allocator: TypedFunction<(i32, i32), i32>,
}

impl PreparedInstance {
    pub fn activate(&mut self, ctx: SharedExecutionContext) {
        {
            let stored = self.env.as_mut(&mut self.store);
            *stored = ctx.clone();
        }

        {
            let mut guard = ctx.write();
            guard.replace_memory(self.memory.clone());
        }
    }

    pub fn memory(&self) -> &Memory {
        &self.memory
    }

    pub fn write_body(&mut self, body: Option<&[u8]>) -> Result<(i32, i32), CardinalError> {
        let Some(body) = body else {
            return Ok((0, 0));
        };

        if body.is_empty() {
            return Ok((0, 0));
        }

        let len = i32::try_from(body.len()).map_err(|_| {
            CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(
                "body too large".into(),
            ))
        })?;

        let ptr = self.allocator.call(&mut self.store, len, 0).map_err(|e| {
            CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                "Alloc failed {e}"
            )))
        })?;

        let view = self.memory.view(&self.store);
        view.write(ptr as u64, body).map_err(|e| {
            CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                "Writing body failed {e}"
            )))
        })?;

        Ok((ptr, len))
    }

    pub fn call_handle(&mut self, ptr: i32, len: i32) -> Result<i32, CardinalError> {
        self.handle.call(&mut self.store, ptr, len).map_err(|e| {
            CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                "WASM handle call failed {e}"
            )))
        })
    }
}

fn initialize_placeholder_memory(
    env: &FunctionEnv<SharedExecutionContext>,
    store: &mut Store,
    memory: Memory,
) {
    let env_mut = env.as_mut(store);
    let mut ctx = env_mut.write();
    ctx.replace_memory(memory);
}
