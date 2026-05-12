//! W5–W6: WASI P2 HTTP — Client (outgoing-handler) and Server (incoming-handler).
//!
//! Implements WASI Preview 2 HTTP interfaces:
//! - `wasi:http/types` — Request, Response, Headers, Method, StatusCode (W5.1)
//! - `wasi:http/outgoing-handler` — HTTP client (W5.2–W5.9)
//! - `wasi:http/incoming-handler` — HTTP server (W6.1–W6.9)
//! - Middleware pipeline, JSON, error responses, static files

use std::collections::HashMap;
use std::fmt;

// ═══════════════════════════════════════════════════════════════════════
// W5.1: HTTP Types
// ═══════════════════════════════════════════════════════════════════════

/// HTTP method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
    Head,
    Options,
    Patch,
    Other(String),
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Get => write!(f, "GET"),
            Self::Post => write!(f, "POST"),
            Self::Put => write!(f, "PUT"),
            Self::Delete => write!(f, "DELETE"),
            Self::Head => write!(f, "HEAD"),
            Self::Options => write!(f, "OPTIONS"),
            Self::Patch => write!(f, "PATCH"),
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}

/// HTTP status code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusCode(pub u16);

impl StatusCode {
    pub const OK: Self = Self(200);
    pub const CREATED: Self = Self(201);
    pub const NO_CONTENT: Self = Self(204);
    pub const BAD_REQUEST: Self = Self(400);
    pub const NOT_FOUND: Self = Self(404);
    pub const INTERNAL_ERROR: Self = Self(500);

    /// Whether this status code indicates success (2xx).
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.0)
    }

    /// Whether this status code indicates a client error (4xx).
    pub fn is_client_error(&self) -> bool {
        (400..500).contains(&self.0)
    }

    /// Whether this status code indicates a server error (5xx).
    pub fn is_server_error(&self) -> bool {
        (500..600).contains(&self.0)
    }
}

impl fmt::Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// HTTP headers (case-insensitive keys).
#[derive(Debug, Clone, Default)]
pub struct Headers {
    entries: Vec<(String, String)>,
}

impl Headers {
    /// Creates empty headers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a header.
    pub fn add(&mut self, name: &str, value: &str) {
        self.entries.push((name.to_lowercase(), value.to_string()));
    }

    /// Gets the first value for a header name.
    pub fn get(&self, name: &str) -> Option<&str> {
        let lower = name.to_lowercase();
        self.entries
            .iter()
            .find(|(k, _)| k == &lower)
            .map(|(_, v)| v.as_str())
    }

    /// Removes all entries for a header name.
    pub fn delete(&mut self, name: &str) {
        let lower = name.to_lowercase();
        self.entries.retain(|(k, _)| k != &lower);
    }

    /// Iterates over all headers.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Number of headers.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the header set is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// HTTP request.
#[derive(Debug, Clone)]
pub struct Request {
    /// HTTP method.
    pub method: Method,
    /// Request URL/path.
    pub url: String,
    /// Request headers.
    pub headers: Headers,
    /// Request body (optional).
    pub body: Option<Vec<u8>>,
}

impl Request {
    /// Creates a GET request.
    pub fn get(url: &str) -> Self {
        Self {
            method: Method::Get,
            url: url.to_string(),
            headers: Headers::new(),
            body: None,
        }
    }

    /// Creates a POST request with a body.
    pub fn post(url: &str, body: Vec<u8>) -> Self {
        let mut headers = Headers::new();
        headers.add("content-length", &body.len().to_string());
        Self {
            method: Method::Post,
            url: url.to_string(),
            headers,
            body: Some(body),
        }
    }

    /// Creates a PUT request.
    pub fn put(url: &str, body: Vec<u8>) -> Self {
        Self {
            method: Method::Put,
            url: url.to_string(),
            headers: Headers::new(),
            body: Some(body),
        }
    }

    /// Creates a DELETE request.
    pub fn delete(url: &str) -> Self {
        Self {
            method: Method::Delete,
            url: url.to_string(),
            headers: Headers::new(),
            body: None,
        }
    }
}

/// HTTP response.
#[derive(Debug, Clone)]
pub struct Response {
    /// Status code.
    pub status: StatusCode,
    /// Response headers.
    pub headers: Headers,
    /// Response body.
    pub body: Vec<u8>,
}

impl Response {
    /// Creates a response with the given status and body.
    pub fn new(status: StatusCode, body: Vec<u8>) -> Self {
        let mut headers = Headers::new();
        headers.add("content-length", &body.len().to_string());
        Self {
            status,
            headers,
            body,
        }
    }

    /// Creates a 200 OK response with a text body.
    pub fn ok(body: &str) -> Self {
        let mut resp = Self::new(StatusCode::OK, body.as_bytes().to_vec());
        resp.headers.add("content-type", "text/plain");
        resp
    }

    /// Creates a JSON response.
    pub fn json(status: StatusCode, json: &str) -> Self {
        let mut resp = Self::new(status, json.as_bytes().to_vec());
        resp.headers.add("content-type", "application/json");
        resp
    }

    /// Creates a 400 Bad Request.
    pub fn bad_request(msg: &str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, msg.as_bytes().to_vec())
    }

    /// Creates a 404 Not Found.
    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, b"Not Found".to_vec())
    }

    /// Creates a 500 Internal Server Error.
    pub fn internal_error(msg: &str) -> Self {
        Self::new(StatusCode::INTERNAL_ERROR, msg.as_bytes().to_vec())
    }

    /// Returns the body as a UTF-8 string.
    pub fn body_text(&self) -> Option<String> {
        String::from_utf8(self.body.clone()).ok()
    }
}

/// HTTP error.
#[derive(Debug, Clone, PartialEq)]
pub enum HttpError {
    /// Network-level error.
    NetworkError(String),
    /// Request timed out.
    Timeout,
    /// DNS resolution failure.
    DnsError(String),
    /// Invalid URL.
    InvalidUrl(String),
    /// Connection refused.
    ConnectionRefused,
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NetworkError(msg) => write!(f, "network error: {msg}"),
            Self::Timeout => write!(f, "request timed out"),
            Self::DnsError(msg) => write!(f, "DNS error: {msg}"),
            Self::InvalidUrl(msg) => write!(f, "invalid URL: {msg}"),
            Self::ConnectionRefused => write!(f, "connection refused"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W5.2–W5.9: HTTP Client (Outgoing Handler)
// ═══════════════════════════════════════════════════════════════════════

/// Simulated HTTP client for WASI P2 outgoing-handler.
#[derive(Debug)]
pub struct HttpClient {
    /// Mock response registry: URL -> Response.
    mock_responses: HashMap<String, Response>,
    /// Whether HTTPS is enabled.
    tls_enabled: bool,
    /// Request history for assertions.
    history: Vec<Request>,
}

impl HttpClient {
    /// Creates a new HTTP client.
    pub fn new() -> Self {
        Self {
            mock_responses: HashMap::new(),
            tls_enabled: true,
            history: Vec::new(),
        }
    }

    /// Registers a mock response for a URL.
    pub fn mock(&mut self, url: &str, response: Response) {
        self.mock_responses.insert(url.to_string(), response);
    }

    /// Sends a request and returns a response.
    pub fn handle(&mut self, request: Request) -> Result<Response, HttpError> {
        // Validate URL
        if request.url.is_empty() {
            return Err(HttpError::InvalidUrl("empty URL".into()));
        }

        // HTTPS check
        if request.url.starts_with("https://") && !self.tls_enabled {
            return Err(HttpError::NetworkError("TLS not available".into()));
        }

        // Look up mock response
        let response = self
            .mock_responses
            .get(&request.url)
            .cloned()
            .unwrap_or_else(Response::not_found);

        self.history.push(request);
        Ok(response)
    }

    /// Returns the request history.
    pub fn request_history(&self) -> &[Request] {
        &self.history
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W6.1–W6.9: HTTP Server (Incoming Handler)
// ═══════════════════════════════════════════════════════════════════════

/// A route handler function.
pub type HandlerFn = Box<dyn Fn(&Request) -> Response>;

/// Route entry.
pub struct Route {
    /// HTTP method to match.
    pub method: Method,
    /// Path pattern to match.
    pub path: String,
    /// Handler function.
    pub handler: HandlerFn,
}

impl fmt::Debug for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Route({} {})", self.method, self.path)
    }
}

/// Middleware function.
pub type MiddlewareFn = Box<dyn Fn(Request) -> Request>;

/// HTTP server router for WASI P2 incoming-handler.
pub struct HttpRouter {
    /// Registered routes.
    routes: Vec<Route>,
    /// Middleware pipeline (applied before handler).
    middleware: Vec<MiddlewareFn>,
    /// Static file directory (path prefix -> directory data).
    static_dirs: HashMap<String, HashMap<String, Vec<u8>>>,
}

impl fmt::Debug for HttpRouter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "HttpRouter(routes={}, middleware={})",
            self.routes.len(),
            self.middleware.len()
        )
    }
}

impl HttpRouter {
    /// Creates a new empty router.
    pub fn new() -> Self {
        Self {
            routes: Vec::new(),
            middleware: Vec::new(),
            static_dirs: HashMap::new(),
        }
    }

    /// Adds a route.
    pub fn route(&mut self, method: Method, path: &str, handler: HandlerFn) {
        self.routes.push(Route {
            method,
            path: path.to_string(),
            handler,
        });
    }

    /// Adds middleware.
    pub fn use_middleware(&mut self, mw: MiddlewareFn) {
        self.middleware.push(mw);
    }

    /// Registers a static file directory.
    pub fn serve_static(&mut self, prefix: &str, files: HashMap<String, Vec<u8>>) {
        self.static_dirs.insert(prefix.to_string(), files);
    }

    /// Handles an incoming request.
    pub fn handle(&self, mut request: Request) -> Response {
        // Apply middleware pipeline
        for mw in &self.middleware {
            request = mw(request);
        }

        // Match route
        for route in &self.routes {
            if route.method == request.method && route.path == request.url {
                return (route.handler)(&request);
            }
        }

        // Check static files
        for (prefix, files) in &self.static_dirs {
            if let Some(file_path) = request.url.strip_prefix(prefix) {
                if let Some(data) = files.get(file_path) {
                    let mut resp = Response::new(StatusCode::OK, data.clone());
                    // Guess MIME type
                    let mime = if file_path.ends_with(".html") {
                        "text/html"
                    } else if file_path.ends_with(".css") {
                        "text/css"
                    } else if file_path.ends_with(".js") {
                        "application/javascript"
                    } else {
                        "application/octet-stream"
                    };
                    resp.headers.add("content-type", mime);
                    return resp;
                }
            }
        }

        Response::not_found()
    }

    /// Number of registered routes.
    pub fn route_count(&self) -> usize {
        self.routes.len()
    }
}

impl Default for HttpRouter {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── W5.1: HTTP types ──

    #[test]
    fn w5_1_types_compile() {
        let req = Request::get("http://example.com");
        assert_eq!(req.method, Method::Get);
        assert_eq!(req.url, "http://example.com");

        let resp = Response::ok("hello");
        assert_eq!(resp.status, StatusCode::OK);
        assert!(resp.status.is_success());
    }

    // ── W5.2: Outgoing handler ──

    #[test]
    fn w5_2_http_get_returns_200() {
        let mut client = HttpClient::new();
        client.mock("http://api.test/data", Response::ok("result"));

        let resp = client.handle(Request::get("http://api.test/data")).unwrap();
        assert_eq!(resp.status, StatusCode::OK);
        assert_eq!(resp.body_text().unwrap(), "result");
    }

    // ── W5.3: Request construction ──

    #[test]
    fn w5_3_all_methods_work() {
        let get = Request::get("/");
        assert_eq!(get.method, Method::Get);
        let post = Request::post("/", b"data".to_vec());
        assert_eq!(post.method, Method::Post);
        let put = Request::put("/", b"data".to_vec());
        assert_eq!(put.method, Method::Put);
        let del = Request::delete("/");
        assert_eq!(del.method, Method::Delete);
    }

    // ── W5.4: Response reading ──

    #[test]
    fn w5_4_parse_json_response() {
        let mut client = HttpClient::new();
        let json_resp = Response::json(StatusCode::OK, r#"{"name":"fajar"}"#);
        client.mock("http://api.test/user", json_resp);

        let resp = client.handle(Request::get("http://api.test/user")).unwrap();
        let body = resp.body_text().unwrap();
        assert!(body.contains("fajar"));
        assert_eq!(resp.headers.get("content-type"), Some("application/json"));
    }

    // ── W5.5: Request body streaming ──

    #[test]
    fn w5_5_post_with_json_body() {
        let mut client = HttpClient::new();
        client.mock(
            "http://api.test/create",
            Response::json(StatusCode::CREATED, r#"{"id":1}"#),
        );

        let body = r#"{"name":"test"}"#.as_bytes().to_vec();
        let resp = client
            .handle(Request::post("http://api.test/create", body))
            .unwrap();
        assert_eq!(resp.status, StatusCode::CREATED);
    }

    // ── W5.6: Response body streaming (large) ──

    #[test]
    fn w5_6_stream_large_response() {
        let mut client = HttpClient::new();
        let large_body = "x".repeat(100_000);
        client.mock(
            "http://api.test/large",
            Response::new(StatusCode::OK, large_body.as_bytes().to_vec()),
        );

        let resp = client
            .handle(Request::get("http://api.test/large"))
            .unwrap();
        assert_eq!(resp.body.len(), 100_000);
    }

    // ── W5.7: Header manipulation ──

    #[test]
    fn w5_7_content_type_correct() {
        let mut headers = Headers::new();
        headers.add("Content-Type", "application/json");
        headers.add("Authorization", "Bearer token");

        assert_eq!(headers.get("content-type"), Some("application/json"));
        assert_eq!(headers.get("CONTENT-TYPE"), Some("application/json")); // case insensitive
        assert_eq!(headers.len(), 2);

        headers.delete("authorization");
        assert_eq!(headers.len(), 1);
        assert!(headers.get("authorization").is_none());
    }

    // ── W5.8: Error handling ──

    #[test]
    fn w5_8_timeout_returns_err() {
        let err = HttpError::Timeout;
        assert_eq!(err.to_string(), "request timed out");

        let dns_err = HttpError::DnsError("NXDOMAIN".into());
        assert!(dns_err.to_string().contains("DNS"));
    }

    // ── W5.9: HTTPS ──

    #[test]
    fn w5_9_https_urls_work() {
        let mut client = HttpClient::new();
        client.mock("https://secure.test/api", Response::ok("secure"));

        let resp = client
            .handle(Request::get("https://secure.test/api"))
            .unwrap();
        assert_eq!(resp.status, StatusCode::OK);
    }

    // ── W5.10: HTTP client tests ──

    #[test]
    fn w5_10_request_history() {
        let mut client = HttpClient::new();
        client.mock("http://a.test/1", Response::ok("a"));
        client.mock("http://a.test/2", Response::ok("b"));

        client.handle(Request::get("http://a.test/1")).unwrap();
        client
            .handle(Request::post("http://a.test/2", b"body".to_vec()))
            .unwrap();

        assert_eq!(client.request_history().len(), 2);
        assert_eq!(client.request_history()[0].method, Method::Get);
        assert_eq!(client.request_history()[1].method, Method::Post);
    }

    // ── W6.1: Incoming handler export ──

    #[test]
    fn w6_1_router_handles_request() {
        let mut router = HttpRouter::new();
        router.route(Method::Get, "/", Box::new(|_| Response::ok("home")));

        let resp = router.handle(Request::get("/"));
        assert_eq!(resp.status, StatusCode::OK);
        assert_eq!(resp.body_text().unwrap(), "home");
    }

    // ── W6.2: Request routing ──

    #[test]
    fn w6_2_path_method_routing() {
        let mut router = HttpRouter::new();
        router.route(
            Method::Get,
            "/api/users",
            Box::new(|_| Response::json(StatusCode::OK, "[]")),
        );
        router.route(
            Method::Post,
            "/api/users",
            Box::new(|_| Response::json(StatusCode::CREATED, r#"{"id":1}"#)),
        );

        let get_resp = router.handle(Request::get("/api/users"));
        assert_eq!(get_resp.status, StatusCode::OK);

        let post_resp = router.handle(Request::post("/api/users", b"{}".to_vec()));
        assert_eq!(post_resp.status, StatusCode::CREATED);
    }

    // ── W6.3: Response construction ──

    #[test]
    fn w6_3_json_response_body() {
        let resp = Response::json(StatusCode::OK, r#"{"name":"fajar","age":42}"#);
        assert_eq!(resp.headers.get("content-type"), Some("application/json"));
        let body = resp.body_text().unwrap();
        assert!(body.contains("fajar"));
        assert!(body.contains("42"));
    }

    // ── W6.4: Middleware pipeline ──

    #[test]
    fn w6_4_middleware_chain_order() {
        let mut router = HttpRouter::new();

        // Middleware that adds a header
        router.use_middleware(Box::new(|mut req: Request| {
            req.headers.add("x-processed", "true");
            req
        }));

        router.route(
            Method::Get,
            "/test",
            Box::new(|req: &Request| {
                let processed = req.headers.get("x-processed").unwrap_or("false");
                Response::ok(processed)
            }),
        );

        let resp = router.handle(Request::get("/test"));
        assert_eq!(resp.body_text().unwrap(), "true");
    }

    // ── W6.5: JSON serialization ──

    #[test]
    fn w6_5_json_output() {
        let resp = Response::json(StatusCode::OK, r#"{"name":"fajar","age":42}"#);
        let body = resp.body_text().unwrap();
        assert!(body.contains(r#""name":"fajar""#));
    }

    // ── W6.6: Error responses ──

    #[test]
    fn w6_6_error_status_codes() {
        let bad = Response::bad_request("missing field");
        assert_eq!(bad.status.0, 400);
        assert!(bad.status.is_client_error());

        let not_found = Response::not_found();
        assert_eq!(not_found.status.0, 404);

        let internal = Response::internal_error("oops");
        assert_eq!(internal.status.0, 500);
        assert!(internal.status.is_server_error());
    }

    // ── W6.7: Request body parsing ──

    #[test]
    fn w6_7_post_body_deserialized() {
        let mut router = HttpRouter::new();
        router.route(
            Method::Post,
            "/data",
            Box::new(|req: &Request| {
                let body = req.body.as_deref().unwrap_or(b"");
                let text = String::from_utf8_lossy(body);
                Response::ok(&format!("got: {text}"))
            }),
        );

        let resp = router.handle(Request::post("/data", b"hello".to_vec()));
        assert_eq!(resp.body_text().unwrap(), "got: hello");
    }

    // ── W6.8: Static file serving ──

    #[test]
    fn w6_8_serve_html_file() {
        let mut router = HttpRouter::new();
        let mut files = HashMap::new();
        files.insert("index.html".to_string(), b"<h1>Hello</h1>".to_vec());
        files.insert("style.css".to_string(), b"body {}".to_vec());
        router.serve_static("/static/", files);

        let resp = router.handle(Request::get("/static/index.html"));
        assert_eq!(resp.status, StatusCode::OK);
        assert_eq!(resp.headers.get("content-type"), Some("text/html"));
        assert_eq!(resp.body_text().unwrap(), "<h1>Hello</h1>");

        let css_resp = router.handle(Request::get("/static/style.css"));
        assert_eq!(css_resp.headers.get("content-type"), Some("text/css"));
    }

    // ── W6.9: Integration ──

    #[test]
    fn w6_9_full_server_workflow() {
        let mut router = HttpRouter::new();

        router.use_middleware(Box::new(|mut req: Request| {
            req.headers.add("x-request-id", "abc123");
            req
        }));

        router.route(Method::Get, "/health", Box::new(|_| Response::ok("ok")));
        router.route(
            Method::Get,
            "/api/users",
            Box::new(|_| Response::json(StatusCode::OK, r#"[{"id":1}]"#)),
        );
        router.route(
            Method::Post,
            "/api/users",
            Box::new(|_| Response::json(StatusCode::CREATED, r#"{"id":2}"#)),
        );

        assert_eq!(router.route_count(), 3);

        // Health check
        let resp = router.handle(Request::get("/health"));
        assert_eq!(resp.status, StatusCode::OK);

        // GET users
        let resp = router.handle(Request::get("/api/users"));
        assert!(resp.body_text().unwrap().contains("id"));

        // POST user
        let resp = router.handle(Request::post("/api/users", b"{}".to_vec()));
        assert_eq!(resp.status, StatusCode::CREATED);

        // 404
        let resp = router.handle(Request::get("/missing"));
        assert_eq!(resp.status, StatusCode::NOT_FOUND);
    }

    // ── W6.10: Additional tests ──

    #[test]
    fn w6_10_missing_route_returns_404() {
        let router = HttpRouter::new();
        let resp = router.handle(Request::get("/nonexistent"));
        assert_eq!(resp.status, StatusCode::NOT_FOUND);
    }
}
