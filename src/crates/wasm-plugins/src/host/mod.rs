use crate::runner::{ExecutionType, HostFunctionMap};
use crate::utils::{read_bytes, with_mem_view, write_bytes};
use crate::ExecutionContext;
use std::collections::HashMap;
use wasmer::{Exports, FunctionEnv, FunctionEnvMut, Imports, Store};

mod abort;
pub mod get_header;
mod get_query_param;
mod set_header;
mod set_status;

/// Read key from guest, optionally normalize, look it up using `get_map`,
/// write value into guest up to `out_cap`, return written bytes or -1.
pub fn read_key_lookup_and_write(
    ctx: &FunctionEnvMut<ExecutionContext>,
    key_ptr: i32,
    key_len: i32,
    out_ptr: i32,
    out_cap: i32,
    normalize_key: bool,
    get_map: impl Fn(&ExecutionContext) -> &HashMap<String, String>,
) -> i32 {
    let view = match with_mem_view(ctx) {
        Ok(v) => v,
        Err(_) => return -1,
    };
    let key = match String::from_utf8(read_bytes(&view, key_ptr, key_len).unwrap_or_default()) {
        Ok(s) => {
            if normalize_key {
                s.to_ascii_lowercase()
            } else {
                s
            }
        }
        Err(_) => return -1,
    };

    let map_mutex = get_map(ctx.data());
    let Some(val) = map_mutex.get(&key) else {
        return -1;
    };
    let bytes = val.as_bytes();

    let n = bytes.len().min(out_cap as usize);
    if n > 0 && write_bytes(&view, out_ptr, &bytes[..n]).is_err() {
        return -1;
    }
    n as i32
}

pub fn make_imports(
    store: &mut Store,
    env: &FunctionEnv<ExecutionContext>,
    exec_type: ExecutionType,
    host_imports: Option<&HostFunctionMap>,
) -> Imports {
    let mut imports = Imports::new();
    let mut ns = Exports::new();

    ns.insert("abort", abort::abort(store, env));
    ns.insert("get_header", get_header::get_header(store, env));
    ns.insert(
        "get_query_param",
        get_query_param::get_query_param(store, env),
    );

    if let ExecutionType::Outbound = exec_type {
        ns.insert("set_header", set_header::set_header(store, env));
        ns.insert("set_status", set_status::set_status(store, env));
    }

    if let Some(host_map) = host_imports {
        if let Some(extra_env) = host_map.get("env") {
            for (name, builder) in extra_env {
                let function = builder(store);
                ns.insert(name.as_str(), function);
            }
        }
    }

    imports.register_namespace("env", ns);

    if let Some(host_map) = host_imports {
        for (namespace, functions) in host_map {
            if namespace == "env" {
                continue;
            }

            let mut exports = Exports::new();
            for (name, builder) in functions {
                let function = builder(store);
                exports.insert(name.as_str(), function);
            }

            imports.register_namespace(namespace, exports);
        }
    }

    imports
}
