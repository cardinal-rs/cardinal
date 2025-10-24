use crate::context::ExecutionContext;
use crate::runner::ExecutionPhase;
use crate::utils::{read_bytes, with_mem_view, write_bytes};
use crate::SharedExecutionContext;
use std::collections::HashMap;
use std::sync::Arc;
use wasmer::{Exports, Function, FunctionEnv, FunctionEnvMut, Imports, Store};

mod abort;
pub mod get_header;
mod get_query_param;
mod get_req_var;
mod set_header;
mod set_req_var;
mod set_status;

use self::abort::ABORT_IMPORT;
use self::get_header::GET_HEADER_IMPORT;
use self::get_query_param::GET_QUERY_PARAM_IMPORT;
use self::get_req_var::GET_REQ_VAR_IMPORT;
use self::set_header::SET_HEADER_IMPORT;
use self::set_req_var::SET_REQ_VAR_IMPORT;
use self::set_status::SET_STATUS_IMPORT;

pub type HostFunctionBuilder =
    Arc<dyn Fn(&mut Store, &FunctionEnv<SharedExecutionContext>) -> Function + Send + Sync>;

pub trait HostImport: Send + Sync {
    fn namespace(&self) -> &str;
    fn name(&self) -> &str;
    fn build(&self, store: &mut Store, env: &FunctionEnv<SharedExecutionContext>) -> Function;
}

pub type HostImportHandle = Arc<dyn HostImport>;

#[derive(Clone)]
pub struct DynamicHostImport {
    namespace: String,
    name: String,
    builder: HostFunctionBuilder,
}

impl DynamicHostImport {
    pub fn new<N, S>(namespace: N, name: S, builder: HostFunctionBuilder) -> Self
    where
        N: Into<String>,
        S: Into<String>,
    {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            builder,
        }
    }
}

impl HostImport for DynamicHostImport {
    fn namespace(&self) -> &str {
        &self.namespace
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn build(&self, store: &mut Store, env: &FunctionEnv<SharedExecutionContext>) -> Function {
        (self.builder)(store, env)
    }
}

pub fn make_imports(
    store: &mut Store,
    env: &FunctionEnv<SharedExecutionContext>,
    phase: ExecutionPhase,
    dynamic_imports: &[HostImportHandle],
) -> Imports {
    let mut namespaces: HashMap<String, Exports> = HashMap::new();

    for import in builtin_imports(phase) {
        register_import(&mut namespaces, store, env, *import);
    }

    for import in dynamic_imports {
        register_import(&mut namespaces, store, env, import.as_ref());
    }

    let mut imports = Imports::new();
    for (namespace, exports) in namespaces {
        imports.register_namespace(&namespace, exports);
    }
    imports
}

fn register_import(
    namespaces: &mut HashMap<String, Exports>,
    store: &mut Store,
    env: &FunctionEnv<SharedExecutionContext>,
    import: &dyn HostImport,
) {
    let namespace = import.namespace().to_string();
    let exports = namespaces.entry(namespace.clone()).or_default();
    let function = import.build(store, env);
    exports.insert(import.name(), function);
}

fn builtin_imports(phase: ExecutionPhase) -> &'static [&'static dyn HostImport] {
    match phase {
        ExecutionPhase::Inbound => INBOUND_IMPORTS,
        ExecutionPhase::Outbound => OUTBOUND_IMPORTS,
    }
}

static INBOUND_IMPORTS: &[&dyn HostImport] = &[
    &ABORT_IMPORT,
    &GET_HEADER_IMPORT,
    &GET_QUERY_PARAM_IMPORT,
    &SET_HEADER_IMPORT,
    &SET_STATUS_IMPORT,
    &SET_REQ_VAR_IMPORT,
    &GET_REQ_VAR_IMPORT,
];

static OUTBOUND_IMPORTS: &[&dyn HostImport] = &[
    &ABORT_IMPORT,
    &GET_HEADER_IMPORT,
    &GET_QUERY_PARAM_IMPORT,
    &SET_HEADER_IMPORT,
    &SET_STATUS_IMPORT,
    &SET_REQ_VAR_IMPORT,
    &GET_REQ_VAR_IMPORT,
];

/// Read key from guest memory and write lookup result back into guest memory.
/// Returns number of bytes written or -1 on failure.
pub fn read_key_lookup_and_write(
    ctx: &FunctionEnvMut<SharedExecutionContext>,
    key_ptr: i32,
    key_len: i32,
    out_ptr: i32,
    out_cap: i32,
    normalize_key: bool,
    lookup: impl Fn(&ExecutionContext, &str) -> Option<Vec<u8>>,
) -> i32 {
    let view = match with_mem_view(ctx) {
        Ok(v) => v,
        Err(_) => return -1,
    };

    let raw_key = match String::from_utf8(read_bytes(&view, key_ptr, key_len).unwrap_or_default()) {
        Ok(key) => key,
        Err(_) => return -1,
    };

    let key = if normalize_key {
        raw_key.to_ascii_lowercase()
    } else {
        raw_key
    };

    let guard = ctx.data().read();
    let bytes = match lookup(&guard, &key) {
        Some(data) => data,
        None => return -1,
    };

    let write_len = bytes.len().min(out_cap as usize);
    if write_len > 0 && write_bytes(&view, out_ptr, &bytes[..write_len]).is_err() {
        return -1;
    }

    write_len as i32
}
