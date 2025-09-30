use cardinal_errors::internal::CardinalInternalError;
use cardinal_errors::CardinalError;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use wasmer::{Engine, Module};

pub struct WasmPlugin {
    pub engine: Engine,
    pub module: Module,
    pub path: PathBuf,
    pub memory_name: String,
    pub handle_name: String,
}

impl WasmPlugin {
    pub fn new(
        engine: Engine,
        module: Module,
        memory_name: Option<String>,
        handle_name: Option<String>,
    ) -> Result<Self, CardinalError> {
        let memory_name = memory_name.unwrap_or_else(|| "memory".to_string());
        let handle_name = handle_name.unwrap_or_else(|| "handle".to_string());

        let plugin = Self {
            engine,
            module,
            path: PathBuf::new(),
            memory_name: memory_name.clone(),
            handle_name: handle_name.clone(),
        };

        plugin.validate_exports(&[memory_name, handle_name])?;

        Ok(plugin)
    }

    /// Load & compile a Wasm module from a file path.
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, CardinalError> {
        let path = path.as_ref().to_path_buf();
        let bytes = std::fs::read(&path)?;
        let (engine, module) = Self::initiate(&bytes, None)?;
        let mut plugin = Self::new(engine, module, None, None)?;
        plugin.path = path;

        Ok(plugin)
    }

    pub fn initiate(
        bytes: &[u8],
        engine: Option<Engine>,
    ) -> Result<(Engine, Module), CardinalError> {
        let engine = engine.unwrap_or_default();
        let module = Module::new(&engine, bytes).map_err(|e| {
            CardinalError::InternalError(CardinalInternalError::InvalidWasmModule(format!(
                "Error initiating plugin {e}"
            )))
        })?;

        Ok((engine, module))
    }

    pub fn with_memory_name(mut self, name: String) -> Self {
        self.memory_name = name;
        self
    }

    pub fn with_handle_name(mut self, name: String) -> Self {
        self.handle_name = name;
        self
    }

    pub fn validate_exports<I, S>(&self, required: I) -> Result<(), CardinalError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let required: HashSet<String> = required.into_iter().map(Into::into).collect();
        let mut found: HashSet<String> = HashSet::new();

        for export in self.module.exports() {
            let name = export.name().to_string();
            if required.contains(&name) {
                found.insert(name);
            }
        }

        let missing: Vec<_> = required.difference(&found).cloned().collect();
        if !missing.is_empty() {
            return Err(CardinalError::Other(format!(
                "wasm plugin missing required exports: {missing:?}"
            )));
        }

        Ok(())
    }
}
