//! End-to-end tests for Clap parsing and every transport-facing execution path.

use std::sync::{Arc, Mutex};

use incur::{Config, Context, Error, Incur, JsonSchema, Parser, Subcommand, clap, json, schemars};
#[cfg(all(feature = "http", feature = "mcp"))]
use incur::{ToolAnnotations, ToolConfig};
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(name = "fixture", version = "1.2.3", about = "Test CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Greet somebody.
    Greet {
        /// Name to greet.
        name: String,
        /// Number of greetings.
        #[arg(long, default_value_t = 1)]
        count: usize,
        /// Shout the greeting.
        #[arg(long)]
        loud: bool,
    },
    /// Return a structured failure.
    Fail,
}

#[derive(Debug, JsonSchema, Serialize)]
struct Output {
    message: String,
    count: usize,
}

#[cfg(feature = "http")]
#[derive(Debug, Parser)]
#[command(name = "transport")]
struct TransportCli {
    #[arg(short = 'v', action = clap::ArgAction::Count)]
    verbose: u8,
    #[arg(short = 'I', action = clap::ArgAction::Append)]
    include: Vec<String>,
    #[arg(long, num_args = 2, action = clap::ArgAction::Set)]
    point: Vec<i32>,
    #[arg(long)]
    dry_run: bool,
}

#[cfg(feature = "http")]
#[derive(Debug, JsonSchema, Serialize)]
struct TransportOutput {
    verbose: u8,
    include: Vec<String>,
    point: Vec<i32>,
    dry_run: bool,
}

fn app() -> incur::App<Cli> {
    Cli::incur(|cli, _context| async move {
        match cli.command {
            Command::Greet { name, count, loud } => {
                let message = format!("hello {name}");
                Ok(Output { message: if loud { message.to_uppercase() } else { message }, count })
            }
            Command::Fail => Err(Error::new("NOPE", "requested failure").retryable(false).into()),
        }
    })
}

#[cfg(feature = "http")]
fn transport_app() -> incur::App<TransportCli> {
    TransportCli::incur(|cli, _context| async move {
        Ok(TransportOutput {
            verbose: cli.verbose,
            include: cli.include,
            point: cli.point,
            dry_run: cli.dry_run,
        })
    })
}

#[tokio::test]
async fn runs_clap_commands_with_agent_envelopes() {
    let result =
        app().execute_from(["fixture", "greet", "Ada", "--count", "2", "--format", "json"]).await;
    assert_eq!(result.exit_code, 0, "{}", result.stderr);
    let value: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["data"], json!({"message": "hello Ada", "count": 2}));
    assert_eq!(value["meta"]["command"], "greet");
}

#[tokio::test]
async fn renders_json_and_filters_data() {
    let result = app()
        .execute_from(["fixture", "greet", "Ada", "--format", "json", "--filter-output", "message"])
        .await;
    let value: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(value["data"], json!("hello Ada"));
}

#[test]
fn generates_openapi_for_reflected_commands() {
    let document = app().manifest().openapi();
    assert_eq!(document["openapi"], "3.1.0");
    assert_eq!(document["paths"]["/greet/{name}"]["post"]["operationId"], "greet");
    assert_eq!(
        document["paths"]["/greet/{name}"]["post"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["properties"]["data"]["type"],
        "object"
    );
}

#[tokio::test]
async fn schema_does_not_require_command_arguments() {
    let result = app().execute_from(["fixture", "greet", "--schema", "--json"]).await;
    assert_eq!(result.exit_code, 0);
    let value: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(value["input"]["type"], "object");
    assert_eq!(value["input"]["properties"]["name"]["type"], "string");
    assert_eq!(value["input"]["properties"]["count"]["type"], "integer");
    assert_eq!(value["input"]["properties"]["loud"]["type"], "boolean");
}

#[tokio::test]
async fn exposes_llm_manifest() {
    let result = app().execute_from(["fixture", "--llms"]).await;
    assert!(result.stdout.contains("`fixture greet <name>`"));
    assert!(result.stdout.contains("Greet somebody"));

    let result = app().execute_from(["fixture", "greet", "--llms-full", "--format", "json"]).await;
    let value: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(value["version"], "incur.v1");
    assert_eq!(value["commands"].as_array().unwrap().len(), 1);
    assert_eq!(value["commands"][0]["name"], "greet");
    assert_eq!(value["commands"][0]["schema"]["input"]["type"], "object");
}

#[tokio::test]
async fn reports_structured_errors_to_agents() {
    let result = app().execute_from(["fixture", "fail", "--format", "json"]).await;
    assert_eq!(result.exit_code, 1);
    let value: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "NOPE");
    assert_eq!(value["meta"]["command"], "fail");
}

#[tokio::test]
async fn middleware_wraps_the_handler() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let before = events.clone();
    let after = events.clone();
    let app = app().middleware(move |context: Context, next: incur::Next| {
        let before = before.clone();
        let after = after.clone();
        async move {
            before.lock().unwrap().push("before");
            let output = next.run(context).await?;
            after.lock().unwrap().push("after");
            Ok(output)
        }
    });
    let result = app.execute_from(["fixture", "greet", "Ada"]).await;
    assert_eq!(result.exit_code, 0);
    assert_eq!(*events.lock().unwrap(), ["before", "after"]);
}

#[cfg(feature = "http")]
#[tokio::test]
async fn maps_http_requests_to_commands() {
    let mut request = incur::HttpRequest::new(Vec::new());
    *request.uri_mut() = "/greet/Ada?count=3".parse().unwrap();
    let response = app().handle_http(request).await;
    assert_eq!(response.status(), 200);
    let value: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(value["data"]["count"], 3);
}

#[cfg(feature = "http")]
#[tokio::test]
async fn exposes_openapi_discovery_routes() {
    let mut request = incur::HttpRequest::new(Vec::new());
    *request.uri_mut() = "/.well-known/openapi.json".parse().unwrap();
    let response = app().handle_http(request).await;
    assert_eq!(response.status(), 200);
    let value: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(value["openapi"], "3.1.0");
    assert_eq!(value["paths"]["/greet/{name}"]["post"]["operationId"], "greet");
}

#[cfg(feature = "http")]
#[tokio::test]
async fn reconstructs_clap_option_actions_from_http() {
    let mut request = incur::HttpRequest::new(Vec::new());
    *request.uri_mut() =
        "/?verbose=2&include=src&include=tests&point=3&point=4&dry-run=true".parse().unwrap();
    let response = transport_app().handle_http(request).await;
    assert_eq!(response.status(), 200);
    let value: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(
        value["data"],
        json!({
            "verbose": 2,
            "include": ["src", "tests"],
            "point": [3, 4],
            "dry_run": true,
        })
    );
}

#[cfg(feature = "http")]
#[tokio::test]
async fn rejects_unknown_http_inputs_and_paths() {
    let mut request = incur::HttpRequest::new(Vec::new());
    *request.uri_mut() = "/greet/Ada/extra".parse().unwrap();
    assert_eq!(app().handle_http(request).await.status(), 404);

    let mut request = incur::HttpRequest::new(Vec::new());
    *request.uri_mut() = "/greet/Ada?unknown=true".parse().unwrap();
    let response = app().handle_http(request).await;
    assert_eq!(response.status(), 400);
    let value: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(value["error"]["code"], "VALIDATION_ERROR");
    assert_eq!(value["error"]["fieldErrors"][0]["path"], "unknown");
}

#[tokio::test]
async fn loads_command_option_defaults_from_config() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), r#"{"commands":{"greet":{"options":{"count":4,"loud":true}}}}"#)
        .unwrap();
    let result = app()
        .config(Config::new())
        .execute_from([
            "fixture",
            "greet",
            "Ada",
            "--config",
            file.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .await;
    assert_eq!(result.exit_code, 0, "{}", result.stderr);
    let value: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(value["data"], json!({"message": "HELLO ADA", "count": 4}));
}

#[cfg(all(feature = "http", feature = "mcp"))]
#[tokio::test]
async fn serves_mcp_over_http() {
    let app = app()
        .tool(
            "greet",
            ToolConfig {
                annotations: Some(ToolAnnotations {
                    read_only_hint: Some(true),
                    ..Default::default()
                }),
                instructions: Some("Use the person's preferred name.".to_owned()),
                ..Default::default()
            },
        )
        .mcp_instructions("Use fixture tools in tests.");
    let mut request = incur::HttpRequest::new(
        serde_json::to_vec(&json!({"jsonrpc":"2.0","id":0,"method":"initialize"})).unwrap(),
    );
    *request.uri_mut() = "/mcp".parse().unwrap();
    let response = app.handle_http(request).await;
    let value: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(value["result"]["instructions"], "Use fixture tools in tests.");

    let mut request = incur::HttpRequest::new(
        serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).unwrap(),
    );
    *request.uri_mut() = "/mcp".parse().unwrap();
    let response = app.handle_http(request).await;
    let value: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert!(
        value["result"]["tools"].as_array().unwrap().iter().any(|tool| tool["name"] == "greet")
    );
    let greet = value["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "greet")
        .unwrap();
    assert_eq!(greet["annotations"]["readOnlyHint"], true);
    assert_eq!(greet["_meta"]["instructions"], "Use the person's preferred name.");

    let mut request = incur::HttpRequest::new(
        serde_json::to_vec(&json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"tools/call",
            "params":{"name":"greet","arguments":{"name":"Ada","count":2}}
        }))
        .unwrap(),
    );
    *request.uri_mut() = "/mcp".parse().unwrap();
    let response = app.handle_http(request).await;
    let value: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(value["result"]["structuredContent"]["message"], "hello Ada");

    let mut request = incur::HttpRequest::new(
        serde_json::to_vec(&json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{"name":"fail","arguments":{}}
        }))
        .unwrap(),
    );
    *request.uri_mut() = "/mcp".parse().unwrap();
    let response = app.handle_http(request).await;
    let value: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(value["result"]["isError"], true);
    assert_eq!(value["result"]["structuredContent"]["error"]["code"], "NOPE");
}
