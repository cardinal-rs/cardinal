use ::wasmer::Memory;
use bytes::Bytes;
use std::collections::HashMap;

#[derive(Clone)]
pub struct ResponseState {
    headers: HashMap<String, String>,
    status: u16,
    status_overridden: bool,
}

impl ResponseState {
    pub fn with_default_status(status: u16) -> Self {
        Self::from_parts(HashMap::new(), status, false)
    }

    pub fn from_parts(
        headers: HashMap<String, String>,
        status: u16,
        status_overridden: bool,
    ) -> Self {
        Self {
            headers,
            status,
            status_overridden,
        }
    }

    pub fn headers(&self) -> &HashMap<String, String> {
        &self.headers
    }

    pub fn headers_mut(&mut self) -> &mut HashMap<String, String> {
        &mut self.headers
    }

    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn set_status(&mut self, status: u16) {
        self.status = status;
        self.status_overridden = true;
    }

    pub fn status_override(&self) -> Option<u16> {
        self.status_overridden.then_some(self.status)
    }
}

impl Default for ResponseState {
    fn default() -> Self {
        Self::with_default_status(0)
    }
}

mod host;
pub mod instance;
pub mod plugin;
pub mod runner;
pub mod utils;

pub mod wasmer {
    pub use wasmer::*;
}

#[derive(Clone, Default)]
pub struct ExecutionContext {
    memory: Option<Memory>,
    req_headers: HashMap<String, String>,
    query: HashMap<String, Vec<String>>,
    body: Option<Bytes>,
    response: ResponseState,
}

impl ExecutionContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_response(response: ResponseState) -> Self {
        Self {
            response,
            ..Self::default()
        }
    }

    pub fn from_parts(
        req_headers: HashMap<String, String>,
        query: HashMap<String, Vec<String>>,
        body: Option<Bytes>,
        response: ResponseState,
    ) -> Self {
        Self {
            memory: None,
            req_headers,
            query,
            body,
            response,
        }
    }

    pub fn replace_memory(&mut self, memory: Memory) {
        self.memory.replace(memory);
    }

    pub fn memory(&self) -> &Option<Memory> {
        &self.memory
    }

    pub fn memory_mut(&mut self) -> &mut Option<Memory> {
        &mut self.memory
    }

    pub fn req_headers(&self) -> &HashMap<String, String> {
        &self.req_headers
    }

    pub fn req_headers_mut(&mut self) -> &mut HashMap<String, String> {
        &mut self.req_headers
    }

    pub fn query(&self) -> &HashMap<String, Vec<String>> {
        &self.query
    }

    pub fn query_mut(&mut self) -> &mut HashMap<String, Vec<String>> {
        &mut self.query
    }

    pub fn body(&self) -> &Option<Bytes> {
        &self.body
    }

    pub fn set_body(&mut self, body: Option<Bytes>) {
        self.body = body;
    }

    pub fn response(&self) -> &ResponseState {
        &self.response
    }

    pub fn response_mut(&mut self) -> &mut ResponseState {
        &mut self.response
    }
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
        run_wasm_case("allow", ScenarioKind::Response);
    }

    #[test]
    fn wasm_plugin_blocks_flagged_requests() {
        run_wasm_case("block", ScenarioKind::Response);
    }

    #[test]
    fn wasm_plugin_requires_tenant() {
        run_wasm_case("require-tenant", ScenarioKind::Response);
    }

    #[test]
    fn wasm_inbound_plugin_allows_when_header_present() {
        run_wasm_case("inbound-allow", ScenarioKind::Request);
    }

    #[test]
    fn wasm_inbound_plugin_blocks_without_header() {
        run_wasm_case("inbound-block", ScenarioKind::Request);
    }

    fn run_wasm_case(name: &str, expected_type: ScenarioKind) {
        let case_dir = case_path(name);
        let wasm_path = case_dir.join("plugin.wasm");
        let incoming_path = case_dir.join("incoming_request.json");
        let expected_path = case_dir.join("expected_response.json");

        let wasm_plugin = WasmPlugin::from_path(&wasm_path)
            .unwrap_or_else(|e| panic!("failed to load plugin {:?}: {}", wasm_path, e));

        let incoming = load_json(&incoming_path);
        let expected = load_json(&expected_path);

        let expected = expected_response_from_value(&expected, name);
        if expected.execution_type != expected_type {
            panic!(
                "fixture {} declares execution_type {:?} but test expected {:?}",
                name, expected.execution_type, expected_type
            );
        }

        let exec_ctx = execution_context_from_value(&incoming, expected.execution_type, name);

        let runner = WasmRunner::new(&wasm_plugin, None);
        let result = runner
            .run(exec_ctx)
            .unwrap_or_else(|e| panic!("plugin execution failed for {:?}: {}", wasm_path, e));

        assert_eq!(
            result.should_continue, expected.should_continue,
            "decision mismatch for {}",
            name
        );
        let context = result.execution_context;
        match expected.execution_type {
            ScenarioKind::Response => {
                let response = context.response();
                let expected_status = expected.status.unwrap_or_else(|| {
                    panic!(
                        "outbound fixture {} must define a status field in expected_response.json",
                        name
                    )
                });
                assert_eq!(
                    i32::from(response.status()),
                    expected_status,
                    "status mismatch for {}",
                    name
                );

                let actual_headers = lowercase_string_map(response.headers().clone());
                for (key, value) in expected.resp_headers.iter() {
                    let actual = actual_headers
                        .get(key)
                        .unwrap_or_else(|| panic!("missing header `{}` for {}", key, name));
                    assert_eq!(actual, value, "header `{}` mismatch for {}", key, name);
                }
            }
            ScenarioKind::Request => {
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

                // No header mutation assertions yet; request plugins under test only read state.
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
        scenario_kind: ScenarioKind,
        scenario: &str,
    ) -> ExecutionContext {
        let req_headers = lowercase_string_map(json_string_map(value.get("req_headers")));
        let query = lowercase_string_vec_map(json_string_vec_map(value.get("query")));
        let body = value.get("body").and_then(body_from_value);

        let response_state = match scenario_kind {
            ScenarioKind::Request => response_state_from_value(value, 403),
            ScenarioKind::Response => response_state_from_value(value, 0),
        };

        ExecutionContext::from_parts(req_headers, query, body, response_state)
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

    fn response_state_from_value(value: &Value, default_status: u16) -> ResponseState {
        let headers = lowercase_string_map(json_string_map(value.get("resp_headers")));
        let override_status = value.get("status").and_then(Value::as_i64).and_then(|raw| {
            if raw > 0 {
                u16::try_from(raw).ok()
            } else {
                None
            }
        });

        match override_status {
            Some(status) => ResponseState::from_parts(headers, status, true),
            None => ResponseState::from_parts(headers, default_status, false),
        }
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

    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    enum ScenarioKind {
        Request,
        Response,
    }

    struct ExpectedResponse {
        should_continue: bool,
        status: Option<i32>,
        resp_headers: HashMap<String, String>,
        execution_type: ScenarioKind,
    }

    fn execution_type_from_value(value: Option<&Value>, scenario: &str) -> ScenarioKind {
        let raw = value
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("fixture {} must define execution_type", scenario));

        match raw.to_ascii_lowercase().as_str() {
            "inbound" => ScenarioKind::Request,
            "outbound" => ScenarioKind::Response,
            other => panic!(
                "fixture {} has invalid execution_type '{}'; expected 'inbound' or 'outbound'",
                scenario, other
            ),
        }
    }
}
