use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use wasmer::Memory;

#[derive(Clone, Debug)]
pub struct ResponseState {
    headers: HeaderMap,
    status: u16,
    status_overridden: bool,
}

impl ResponseState {
    pub fn with_default_status(status: u16) -> Self {
        Self::from_parts(HeaderMap::new(), status, false)
    }

    pub fn from_parts(headers: HeaderMap, status: u16, status_overridden: bool) -> Self {
        Self {
            headers,
            status,
            status_overridden,
        }
    }

    pub fn from_hash_map(
        headers: HashMap<String, String>,
        status: u16,
        status_overridden: bool,
    ) -> Self {
        let map = header_map_from_hashmap(headers);
        Self::from_parts(map, status, status_overridden)
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    pub fn insert_header(&mut self, name: HeaderName, value: HeaderValue) {
        self.headers.insert(name, value);
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

#[derive(Debug)]
struct QueryStore {
    entries: HashMap<String, Arc<Vec<String>>>,
}

impl QueryStore {
    fn new(entries: HashMap<String, Vec<String>>) -> Self {
        let mut map = HashMap::with_capacity(entries.len());
        for (key, values) in entries {
            let normalized = key.to_ascii_lowercase();
            map.insert(normalized, Arc::new(values));
        }
        Self { entries: map }
    }

    fn get_first(&self, key: &str) -> Option<String> {
        let normalized = key.to_ascii_lowercase();
        self.entries
            .get(&normalized)
            .and_then(|values| values.first().cloned())
    }

    fn to_hash_map(&self) -> HashMap<String, Vec<String>> {
        self.entries
            .iter()
            .map(|(key, values)| (key.clone(), (**values).clone()))
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct RequestState {
    headers: Arc<HeaderMap>,
    query: Arc<QueryStore>,
    body: Option<Bytes>,
    persistent_vars: Arc<RwLock<HashMap<String, String>>>,
}

impl RequestState {
    pub fn new(
        headers: HashMap<String, String>,
        query: HashMap<String, Vec<String>>,
        body: Option<Bytes>,
        persistent_vars: Arc<RwLock<HashMap<String, String>>>,
    ) -> Self {
        let header_map = header_map_from_hashmap(headers);
        let query_store = QueryStore::new(query);
        Self {
            headers: Arc::new(header_map),
            query: Arc::new(query_store),
            body,
            persistent_vars,
        }
    }

    pub fn empty() -> Self {
        Self {
            headers: Arc::new(HeaderMap::new()),
            query: Arc::new(QueryStore::new(HashMap::new())),
            body: None,
            persistent_vars: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn header_bytes(&self, name: &str) -> Option<Vec<u8>> {
        let header_name = HeaderName::from_bytes(name.as_bytes()).ok()?;
        self.headers
            .get(&header_name)
            .map(|value| value.as_bytes().to_vec())
    }

    pub fn query_first(&self, key: &str) -> Option<String> {
        self.query.get_first(key)
    }

    pub fn query_entries(&self) -> HashMap<String, Vec<String>> {
        self.query.to_hash_map()
    }

    pub fn body(&self) -> Option<&Bytes> {
        self.body.as_ref()
    }

    pub fn set_body(&mut self, body: Option<Bytes>) {
        self.body = body;
    }

    pub fn persistent_vars(&self) -> &Arc<RwLock<HashMap<String, String>>> {
        &self.persistent_vars
    }
}

impl Default for RequestState {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Clone, Default, Debug)]
pub struct ExecutionContext {
    memory: Option<Memory>,
    request: RequestState,
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
        persistent_vars: Arc<RwLock<HashMap<String, String>>>,
    ) -> Self {
        let request = RequestState::new(req_headers, query, body, persistent_vars);
        Self {
            memory: None,
            request,
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

    pub fn request(&self) -> &RequestState {
        &self.request
    }

    pub fn request_mut(&mut self) -> &mut RequestState {
        &mut self.request
    }

    pub fn response(&self) -> &ResponseState {
        &self.response
    }

    pub fn response_mut(&mut self) -> &mut ResponseState {
        &mut self.response
    }

    pub fn persistent_vars(&self) -> &Arc<RwLock<HashMap<String, String>>> {
        self.request.persistent_vars()
    }
}

pub type SharedExecutionContext = Arc<RwLock<ExecutionContext>>;

fn header_map_from_hashmap(headers: HashMap<String, String>) -> HeaderMap {
    let mut header_map = HeaderMap::with_capacity(headers.len());
    for (key, value) in headers {
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            header_map.append(name, val);
        }
    }
    header_map
}
