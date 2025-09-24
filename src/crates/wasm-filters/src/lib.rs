use bytes::Bytes;
use derive_builder::Builder;
use std::collections::HashMap;
use wasmer::Memory;

mod host;
pub mod instance;
pub mod plugin;
pub mod runner;
mod utils;

/// Per-instance host context; we’ll extend this in the next step
/// (headers map, status, etc.). For now it just carries `memory`.
#[derive(Clone, Builder)]
pub struct ExecutionContext {
    pub memory: Option<Memory>,
    pub req_headers: HashMap<String, String>,
    pub query: HashMap<String, Vec<String>>,
    pub resp_headers: HashMap<String, String>,
    pub status: i32,
    pub body: Option<Bytes>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::WasmPlugin;
    use crate::runner::WasmRunner;
    use bytes::Bytes;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    const CASE_ROOT: &str = "../../../tests/wasm-plugins";

    #[test]
    fn wasm_plugin_allow_sets_headers() {
        run_case("allow");
    }

    #[test]
    fn wasm_plugin_blocks_flagged_requests() {
        run_case("block");
    }

    #[test]
    fn wasm_plugin_requires_tenant() {
        run_case("require-tenant");
    }

    fn run_case(name: &str) {
        let case_dir = case_path(name);
        let wasm_path = case_dir.join("plugin.wasm");
        let incoming_path = case_dir.join("incoming_request.json");
        let expected_path = case_dir.join("expected_response.json");

        let wasm_plugin = WasmPlugin::from_path(&wasm_path)
            .unwrap_or_else(|e| panic!("failed to load plugin {:?}: {}", wasm_path, e));

        let incoming = load_json(&incoming_path);
        let expected = load_json(&expected_path);

        let exec_ctx = execution_context_from_value(&incoming);
        let expected = expected_response_from_value(&expected);

        let runner = WasmRunner::new(&wasm_plugin);
        let result = runner
            .run(exec_ctx)
            .unwrap_or_else(|e| panic!("plugin execution failed for {:?}: {}", wasm_path, e));

        assert_eq!(
            result.should_continue, expected.should_continue,
            "decision mismatch for {}",
            name
        );
        assert_eq!(
            result.execution_context.status, expected.status,
            "status mismatch for {}",
            name
        );

        let actual_headers = lowercase_string_map(result.execution_context.resp_headers.clone());
        for (key, value) in expected.resp_headers.iter() {
            let actual = actual_headers
                .get(key)
                .unwrap_or_else(|| panic!("missing header `{}` for {}", key, name));
            assert_eq!(actual, value, "header `{}` mismatch for {}", key, name);
        }
    }

    fn case_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(CASE_ROOT)
            .join(name)
    }

    fn load_json(path: &Path) -> Value {
        let data =
            fs::read_to_string(path).unwrap_or_else(|e| panic!("failed to read {:?}: {}", path, e));
        serde_json::from_str(&data).unwrap_or_else(|e| panic!("failed to parse {:?}: {}", path, e))
    }

    fn execution_context_from_value(value: &Value) -> ExecutionContext {
        let req_headers = lowercase_string_map(json_string_map(value.get("req_headers")));
        let query = lowercase_string_vec_map(json_string_vec_map(value.get("query")));
        let resp_headers = lowercase_string_map(json_string_map(value.get("resp_headers")));
        let status = value.get("status").and_then(Value::as_i64).unwrap_or(0) as i32;
        let body = value.get("body").and_then(body_from_value);

        ExecutionContext {
            memory: None,
            req_headers,
            query,
            resp_headers,
            status,
            body,
        }
    }

    fn expected_response_from_value(value: &Value) -> ExpectedResponse {
        let should_continue = value
            .get("should_continue")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let status = value.get("status").and_then(Value::as_i64).unwrap_or(0) as i32;
        let resp_headers = lowercase_string_map(json_string_map(value.get("resp_headers")));

        ExpectedResponse {
            should_continue,
            status,
            resp_headers,
        }
    }

    fn json_string_map(value: Option<&Value>) -> HashMap<String, String> {
        match value {
            Some(Value::Object(map)) => map
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect(),
            _ => HashMap::new(),
        }
    }

    fn json_string_vec_map(value: Option<&Value>) -> HashMap<String, Vec<String>> {
        match value {
            Some(Value::Object(map)) => map
                .iter()
                .map(|(k, v)| (k.clone(), value_to_string_vec(v)))
                .collect(),
            _ => HashMap::new(),
        }
    }

    fn value_to_string_vec(value: &Value) -> Vec<String> {
        match value {
            Value::String(s) => vec![s.to_string()],
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn lowercase_string_map(map: HashMap<String, String>) -> HashMap<String, String> {
        map.into_iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v))
            .collect()
    }

    fn lowercase_string_vec_map(map: HashMap<String, Vec<String>>) -> HashMap<String, Vec<String>> {
        map.into_iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v))
            .collect()
    }

    fn body_from_value(value: &Value) -> Option<Bytes> {
        match value {
            Value::Null => None,
            Value::String(s) => Some(Bytes::from(s.clone())),
            _ => None,
        }
    }

    struct ExpectedResponse {
        should_continue: bool,
        status: i32,
        resp_headers: HashMap<String, String>,
    }
}
