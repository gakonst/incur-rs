use std::ffi::OsString;

use http::{Request, Response, StatusCode, header};
use serde_json::{Map, Value, json};

use crate::{App, Error, InternalResult as Result, command};

/// In-memory HTTP request and response body.
pub type HttpBody = Vec<u8>;
/// Request accepted by [`App::handle_http`].
pub type HttpRequest = Request<HttpBody>;
/// Response returned by [`App::handle_http`].
pub type HttpResponse = Response<HttpBody>;

impl<T> App<T>
where
    T: clap::Parser + Send + 'static,
{
    /// Handles an HTTP request using the same command runtime as [`App::serve`].
    ///
    /// `GET /users?limit=5` maps to `users --limit 5`. Extra path segments become positional
    /// arguments. JSON request-body fields are merged with query parameters. `/mcp` accepts MCP
    /// Streamable HTTP JSON-RPC requests.
    pub async fn handle_http(&self, request: HttpRequest) -> HttpResponse {
        if matches!(request.uri().path(), "/openapi.json" | "/.well-known/openapi.json") {
            return json_response(
                StatusCode::OK,
                serde_json::to_vec_pretty(&self.manifest.openapi()).unwrap_or_default(),
            );
        }
        if matches!(request.uri().path(), "/openapi.yml" | "/openapi.yaml") {
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/yaml; charset=utf-8")
                .body(
                    serde_yaml::to_string(&self.manifest.openapi())
                        .unwrap_or_default()
                        .into_bytes(),
                )
                .expect("a static HTTP response is valid");
        }
        if request.uri().path() == "/mcp" {
            return self.handle_mcp_http(request).await;
        }
        match self.http_argv(&request) {
            Ok(argv) => {
                let execution = self.execute_inner(argv, true).await;
                let status = if execution.exit_code == 0 {
                    StatusCode::OK
                } else if execution.exit_code == 2 {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
                let body = if execution.stdout.is_empty() && execution.exit_code != 0 {
                    serde_json::to_vec(&json!({
                        "ok": false,
                        "error": {
                            "code": "VALIDATION_ERROR",
                            "message": execution.stderr,
                            "retryable": false,
                        }
                    }))
                    .unwrap_or_default()
                } else if execution.stdout.is_empty() {
                    execution.stderr.into_bytes()
                } else {
                    execution.stdout.into_bytes()
                };
                json_response(status, body)
            }
            Err(error) => {
                let status = if error.code == "NOT_FOUND" {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::BAD_REQUEST
                };
                json_response(
                    status,
                    serde_json::to_vec(&json!({"ok": false, "error": error})).unwrap_or_default(),
                )
            }
        }
    }

    async fn handle_mcp_http(&self, _request: HttpRequest) -> HttpResponse {
        #[cfg(feature = "mcp")]
        {
            let message = match serde_json::from_slice::<Value>(_request.body()) {
                Ok(message) => message,
                Err(error) => {
                    return json_response(
                        StatusCode::BAD_REQUEST,
                        serde_json::to_vec(&json!({
                            "jsonrpc": "2.0",
                            "id": null,
                            "error": { "code": -32700, "message": error.to_string() },
                        }))
                        .unwrap_or_default(),
                    );
                }
            };
            let response = crate::mcp::handle_jsonrpc(self, message).await.unwrap_or(Value::Null);
            json_response(
                if response.is_null() { StatusCode::ACCEPTED } else { StatusCode::OK },
                serde_json::to_vec(&response).unwrap_or_default(),
            )
        }
        #[cfg(not(feature = "mcp"))]
        json_response(
            StatusCode::NOT_IMPLEMENTED,
            br#"{"error":"the mcp feature is disabled"}"#.to_vec(),
        )
    }

    fn http_argv(&self, request: &HttpRequest) -> Result<Vec<OsString>> {
        let segments = request
            .uri()
            .path()
            .trim_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(percent_decode)
            .collect::<Vec<_>>();
        let command = self
            .manifest
            .commands
            .iter()
            .filter(|command| {
                let path = command.name.split_whitespace().collect::<Vec<_>>();
                path.iter().zip(&segments).all(|(left, right)| *left == right)
                    && path.len() <= segments.len()
            })
            .max_by_key(|command| command.name.split_whitespace().count())
            .or_else(|| self.manifest.command(""))
            .ok_or_else(|| {
                Error::new("NOT_FOUND", format!("unknown command path `{}`", request.uri().path()))
            })?;
        let command_depth = command.name.split_whitespace().count();
        let remaining = &segments[command_depth..];
        let mut input = Map::new();
        let mut consumed = 0;
        for argument in &command.positionals {
            if argument.array {
                let values =
                    remaining[consumed..].iter().cloned().map(Value::String).collect::<Vec<_>>();
                if !values.is_empty() {
                    input.insert(argument.name.clone(), Value::Array(values));
                }
                consumed = remaining.len();
                break;
            }
            if let Some(value) = remaining.get(consumed) {
                input.insert(argument.name.clone(), Value::String(value.clone()));
                consumed += 1;
            }
        }
        if consumed < remaining.len() {
            return Err(Error::new(
                "NOT_FOUND",
                format!("unknown command path `{}`", request.uri().path()),
            ));
        }
        if let Some(query) = request.uri().query() {
            for (key, value) in form_urlencoded::parse(query.as_bytes()) {
                let key = canonical_input_name(command, &key).unwrap_or_else(|| key.into_owned());
                insert_repeated(&mut input, key, Value::String(value.into_owned()));
            }
        }
        if !request.body().is_empty() {
            let body = serde_json::from_slice::<Value>(request.body())?;
            let object = body.as_object().ok_or_else(|| {
                Error::validation(
                    "HTTP request body must be a JSON object",
                    vec![crate::FieldError {
                        path: String::new(),
                        message: "expected object".to_owned(),
                    }],
                )
            })?;
            for (key, value) in object {
                let key = canonical_input_name(command, key).unwrap_or_else(|| key.clone());
                input.insert(key, value.clone());
            }
        }
        let mut argv = command::argv_for_tool(&self.manifest.name, command, &Value::Object(input))?;
        argv.extend([OsString::from("--format"), OsString::from("json")]);
        Ok(argv)
    }
}

fn canonical_input_name(command: &crate::CommandInfo, key: &str) -> Option<String> {
    command
        .positionals
        .iter()
        .chain(&command.options)
        .find(|input| {
            input.name == key
                || input.long.as_deref() == Some(key)
                || input.short.is_some_and(|short| {
                    let mut characters = key.chars();
                    characters.next() == Some(short) && characters.next().is_none()
                })
        })
        .map(|input| input.name.clone())
}

fn insert_repeated(map: &mut Map<String, Value>, key: String, value: Value) {
    match map.remove(&key) {
        None => {
            map.insert(key, value);
        }
        Some(Value::Array(mut values)) => {
            values.push(value);
            map.insert(key, Value::Array(values));
        }
        Some(existing) => {
            map.insert(key, Value::Array(vec![existing, value]));
        }
    }
}

fn percent_decode(value: &str) -> String {
    percent_encoding::percent_decode_str(value).decode_utf8_lossy().into_owned()
}

fn json_response(status: StatusCode, body: Vec<u8>) -> HttpResponse {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(body)
        .expect("a static HTTP response is valid")
}
