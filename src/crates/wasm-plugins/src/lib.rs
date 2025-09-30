use ::wasmer::Memory;
use bytes::Bytes;
use derive_builder::Builder;
use enum_as_inner::EnumAsInner;
use std::collections::HashMap;

mod host;
pub mod instance;
pub mod plugin;
pub mod runner;
pub mod utils;

pub mod wasmer {
    pub use wasmer::*;
}

#[derive(Clone, Builder)]
pub struct ExecutionRequest {
    pub memory: Option<Memory>,
    pub req_headers: HashMap<String, String>,
    pub query: HashMap<String, Vec<String>>,
    pub body: Option<Bytes>,
}

#[derive(Clone, Builder)]
pub struct ExecutionResponse {
    pub memory: Option<Memory>,
    pub req_headers: HashMap<String, String>,
    pub query: HashMap<String, Vec<String>>,
    pub body: Option<Bytes>,

    // Response fields
    pub resp_headers: HashMap<String, String>,
    pub status: i32,
}

/// Per-instance host context; we’ll extend this in the next step
/// (headers map, status, etc.). For now it just carries `memory`.
#[derive(Clone, EnumAsInner)]
pub enum ExecutionContext {
    Inbound(ExecutionRequest),
    Outbound(ExecutionResponse),
}

impl ExecutionContext {
    pub fn replace_memory(&mut self, memory: Memory) {
        match self {
            ExecutionContext::Inbound(ctx) => {
                ctx.memory.replace(memory);
            }
            ExecutionContext::Outbound(ctx) => {
                ctx.memory.replace(memory);
            }
        }
    }

    pub fn body(&self) -> &Option<Bytes> {
        match self {
            ExecutionContext::Inbound(inbound) => &inbound.body,
            ExecutionContext::Outbound(outbound) => &outbound.body,
        }
    }

    pub fn req_headers(&self) -> &HashMap<String, String> {
        match self {
            ExecutionContext::Inbound(inbound) => &inbound.req_headers,
            ExecutionContext::Outbound(outbound) => &outbound.req_headers,
        }
    }

    pub fn query(&self) -> &HashMap<String, Vec<String>> {
        match self {
            ExecutionContext::Inbound(inbound) => &inbound.query,
            ExecutionContext::Outbound(outbound) => &outbound.query,
        }
    }

    pub fn memory(&self) -> &Option<Memory> {
        match self {
            ExecutionContext::Inbound(inbound) => &inbound.memory,
            ExecutionContext::Outbound(outbound) => &outbound.memory,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::WasmPlugin;
    use crate::runner::{ExecutionType, WasmRunner};
    use bytes::Bytes;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    const CASE_ROOT: &str = "../../../tests/wasm-plugins";

    #[test]
    fn wasm_plugin_allow_sets_headers() {
        run_wasm_case("allow", ExecutionType::Outbound);
    }

    #[test]
    fn wasm_plugin_blocks_flagged_requests() {
        run_wasm_case("block", ExecutionType::Outbound);
    }

    #[test]
    fn wasm_plugin_requires_tenant() {
        run_wasm_case("require-tenant", ExecutionType::Outbound);
    }

    #[test]
    fn wasm_inbound_plugin_allows_when_header_present() {
        run_wasm_case("inbound-allow", ExecutionType::Inbound);
    }

    #[test]
    fn wasm_inbound_plugin_blocks_without_header() {
        run_wasm_case("inbound-block", ExecutionType::Inbound);
    }

    fn run_wasm_case(name: &str, expected_type: ExecutionType) {
        let case_dir = case_path(name);
        let wasm_path = case_dir.join("plugin.wasm");
        let incoming_path = case_dir.join("incoming_request.json");
        let expected_path = case_dir.join("expected_response.json");

        let wasm_plugin = WasmPlugin::from_path(&wasm_path)
            .unwrap_or_else(|e| panic!("failed to load plugin {:?}: {}", wasm_path, e));

        let incoming = load_json(&incoming_path);
        let expected = load_json(&expected_path);

        let expected = expected_response_from_value(&expected, name);
        if !matches!(
            (expected.execution_type, expected_type),
            (ExecutionType::Inbound, ExecutionType::Inbound)
                | (ExecutionType::Outbound, ExecutionType::Outbound)
        ) {
            panic!(
                "fixture {} declares execution_type {:?} but test expected {:?}",
                name, expected.execution_type, expected_type
            );
        }

        let exec_ctx = execution_context_from_value(&incoming, expected.execution_type, name);

        let runner = WasmRunner::new(&wasm_plugin, expected.execution_type, None);
        let result = runner
            .run(exec_ctx)
            .unwrap_or_else(|e| panic!("plugin execution failed for {:?}: {}", wasm_path, e));

        assert_eq!(
            result.should_continue, expected.should_continue,
            "decision mismatch for {}",
            name
        );
        match expected.execution_type {
            ExecutionType::Outbound => {
                let outbound = result
                    .execution_context
                    .into_outbound()
                    .unwrap_or_else(|_| panic!("expected outbound context for {}", name));

                let expected_status = expected.status.unwrap_or_else(|| {
                    panic!(
                        "outbound fixture {} must define a status field in expected_response.json",
                        name
                    )
                });
                assert_eq!(
                    outbound.status, expected_status,
                    "status mismatch for {}",
                    name
                );

                let actual_headers = lowercase_string_map(outbound.resp_headers.clone());
                for (key, value) in expected.resp_headers.iter() {
                    let actual = actual_headers
                        .get(key)
                        .unwrap_or_else(|| panic!("missing header `{}` for {}", key, name));
                    assert_eq!(actual, value, "header `{}` mismatch for {}", key, name);
                }
            }
            ExecutionType::Inbound => {
                let inbound = result
                    .execution_context
                    .into_inbound()
                    .unwrap_or_else(|_| panic!("expected inbound context for {}", name));

                assert!(
                    expected.status.is_none(),
                    "inbound fixture {} should not define a status field",
                    name
                );

                assert!(
                    expected.resp_headers.is_empty(),
                    "inbound fixture {} should not define resp_headers",
                    name
                );

                // No header mutation assertions yet; inbound plugins under test only read state.
                let _inbound = inbound;
            }
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

    fn execution_context_from_value(
        value: &Value,
        exec_type: ExecutionType,
        scenario: &str,
    ) -> ExecutionContext {
        let req_headers = lowercase_string_map(json_string_map(value.get("req_headers")));
        let query = lowercase_string_vec_map(json_string_vec_map(value.get("query")));
        let body = value.get("body").and_then(body_from_value);

        match exec_type {
            ExecutionType::Inbound => ExecutionContext::Inbound(ExecutionRequest {
                memory: None,
                req_headers,
                query,
                body,
            }),
            ExecutionType::Outbound => {
                let resp_headers = lowercase_string_map(json_string_map(value.get("resp_headers")));
                let status = value
                    .get("status")
                    .and_then(Value::as_i64)
                    .unwrap_or_else(|| {
                        panic!(
                            "outbound fixture {} must define a numeric status field",
                            scenario
                        )
                    }) as i32;

                ExecutionContext::Outbound(ExecutionResponse {
                    memory: None,
                    req_headers,
                    query,
                    body,
                    resp_headers,
                    status,
                })
            }
        }
    }

    fn expected_response_from_value(value: &Value, scenario: &str) -> ExpectedResponse {
        let should_continue = value
            .get("should_continue")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let execution_type = execution_type_from_value(value.get("execution_type"), scenario);
        let status = value
            .get("status")
            .and_then(Value::as_i64)
            .map(|s| s as i32);
        let resp_headers = lowercase_string_map(json_string_map(value.get("resp_headers")));

        ExpectedResponse {
            should_continue,
            status,
            resp_headers,
            execution_type,
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
        status: Option<i32>,
        resp_headers: HashMap<String, String>,
        execution_type: ExecutionType,
    }

    fn execution_type_from_value(value: Option<&Value>, scenario: &str) -> ExecutionType {
        let raw = value
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("fixture {} must define execution_type", scenario));

        match raw.to_ascii_lowercase().as_str() {
            "inbound" => ExecutionType::Inbound,
            "outbound" => ExecutionType::Outbound,
            other => panic!(
                "fixture {} has invalid execution_type '{}'; expected 'inbound' or 'outbound'",
                scenario, other
            ),
        }
    }
}
