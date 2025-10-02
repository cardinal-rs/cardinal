use crate::host::make_imports;
use crate::plugin::WasmPlugin;
use crate::runner::HostFunctionMap;
use crate::SharedExecutionContext;
use cardinal_errors::internal::CardinalInternalError;
use cardinal_errors::CardinalError;
use wasmer::{FunctionEnv, Instance, Memory, Store};

pub struct WasmInstance {
    pub store: Store,
    pub instance: Instance,
    pub memory: Memory,
    pub env: FunctionEnv<SharedExecutionContext>, // <— store the env here
}

impl WasmInstance {
    pub fn from_plugin(
        plugin: &WasmPlugin,
        host_imports: Option<&HostFunctionMap>,
        ctx: SharedExecutionContext,
    ) -> Result<Self, CardinalError> {
        let mut store = Store::new(plugin.engine.clone());

        let env = FunctionEnv::new(&mut store, ctx.clone());

        let imports = make_imports(&mut store, &env, host_imports);

        // Create the instance.
        let instance = Instance::new(&mut store, &plugin.module, &imports).map_err(|e| {
            CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                "Error creating WASM Instance {e}"
            )))
        })?;

        // Stash it in the env so host imports can access it.
        // Get the guest linear memory (usually named "memory")
        let memory_name = plugin.memory_name.as_str(); // or default to "memory"
        let memory = instance
            .exports
            .get_memory(memory_name)
            .map_err(|e| {
                CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                    "missing memory export `{memory_name}`: {e}"
                )))
            })?
            .clone();

        {
            let env_mut = env.as_mut(&mut store);
            let mut inner_exec_ctx = env_mut.write();
            inner_exec_ctx.replace_memory(memory.clone());
        }

        Ok(WasmInstance {
            store,
            instance,
            memory,
            env,
        })
    }
}
