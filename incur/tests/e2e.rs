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

#[derive(Debug, Parser)]
#[command(name = "collisions")]
struct CollisionCli {
    #[command(subcommand)]
    command: CollisionCommand,
}

#[derive(Debug, Subcommand)]
enum CollisionCommand {
    Completions { shell: String },
    Skills { action: String },
    Mcp { action: String },
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

fn app_with_cta() -> incur::App<Cli> {
    Cli::incur(|cli, context| async move {
        context.suggest(incur::CtaBlock::new([incur::Cta::new("greet").arg("Grace")]));
        match cli.command {
            Command::Greet { name, count, loud } => {
                let message = format!("hello {name}");
                Ok(Output { message: if loud { message.to_uppercase() } else { message }, count })
            }
            Command::Fail => Err(Error::new("NOPE", "requested failure").into()),
        }
    })
}

fn collision_app() -> incur::App<CollisionCli> {
    CollisionCli::incur(|cli, _context| async move {
        let (command, value) = match cli.command {
            CollisionCommand::Completions { shell } => ("completions", shell),
            CollisionCommand::Skills { action } => ("skills", action),
            CollisionCommand::Mcp { action } => ("mcp", action),
        };
        Ok(json!({"command": command, "value": value}))
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

#[cfg(all(feature = "http", feature = "mcp"))]
fn mcp_request(body: Vec<u8>) -> incur::HttpRequest {
    let mut request = incur::HttpRequest::new(body);
    *request.method_mut() = "POST".parse().unwrap();
    *request.uri_mut() = "/mcp".parse().unwrap();
    request
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
async fn validates_formats_for_manifest_shortcuts() {
    let result = app().execute_from(["fixture", "--llms", "--format", "invalid"]).await;
    assert_eq!(result.exit_code, 2);
    assert!(result.stderr.contains("unsupported output format"));

    let result = app().execute_from(["fixture", "--schema", "--format"]).await;
    assert_eq!(result.exit_code, 2);
    assert!(result.stderr.contains("--format"));
}

#[cfg(feature = "completions")]
#[tokio::test]
async fn builtin_completion_help_has_no_side_effect() {
    let result = app().execute_from(["fixture", "completions", "--help"]).await;
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("Usage:"));
}

#[cfg(feature = "skills")]
#[tokio::test]
async fn builtin_skills_help_and_values_are_parsed_before_side_effects() {
    let result = app().execute_from(["fixture", "skills", "add", "--help"]).await;
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("Usage:"));

    let result = app().execute_from(["fixture", "skills", "add", "--depth", "invalid"]).await;
    assert_eq!(result.exit_code, 2);
    assert!(result.stderr.contains("invalid value"));
}

#[cfg(feature = "mcp")]
#[tokio::test]
async fn builtin_mcp_help_has_no_side_effect() {
    let result = app().execute_from(["fixture", "mcp", "add", "--help"]).await;
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("Usage:"));
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

#[tokio::test]
async fn middleware_reentry_returns_an_error_instead_of_panicking() {
    let app = app().middleware(|context: Context, next: incur::Next| async move {
        let second = next.clone();
        next.run(context.clone()).await?;
        second.run(context).await
    });
    let result = app.execute_from(["fixture", "greet", "Ada", "--format", "json"]).await;
    assert_eq!(result.exit_code, 1);
    let value: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(value["error"]["code"], "MIDDLEWARE_REENTRY");
}

#[tokio::test]
async fn puts_ctas_in_envelope_metadata() {
    let result = app_with_cta().execute_from(["fixture", "greet", "Ada", "--format", "json"]).await;
    let value: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(value["meta"]["cta"]["commands"][0]["command"], "greet");
    assert!(value.get("cta").is_none());
}

#[cfg(feature = "tokens")]
#[tokio::test]
async fn token_pagination_preserves_a_valid_envelope() {
    let result = app()
        .execute_from(["fixture", "greet", "Ada", "--format", "json", "--token-limit", "3"])
        .await;
    assert_eq!(result.exit_code, 0, "{}", result.stderr);
    let value: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(value["ok"], true);
    assert!(value["data"].as_str().unwrap().contains("[truncated:"));
    assert!(value["meta"]["nextOffset"].is_number());

    let result = app().execute_from(["fixture", "greet", "Ada", "--token-limit", "0"]).await;
    assert_eq!(result.exit_code, 2);
}

#[tokio::test]
async fn disabled_features_do_not_advertise_dead_commands() {
    let result = app().execute_from(["fixture", "--help"]).await;
    assert_eq!(result.exit_code, 0);
    #[cfg(not(feature = "completions"))]
    assert!(!result.stdout.contains("completions"));
    #[cfg(not(feature = "skills"))]
    assert!(!result.stdout.contains("skills"));
    #[cfg(not(feature = "mcp"))]
    {
        assert!(!result.stdout.contains("mcp"));
        assert!(!result.stdout.contains("--mcp"));
    }
    #[cfg(not(feature = "tokens"))]
    assert!(!result.stdout.contains("--token-"));
    #[cfg(not(feature = "yaml"))]
    assert!(!result.stdout.contains("yaml"));
}

#[tokio::test]
async fn user_commands_win_over_builtin_names() {
    for (command, value) in [("completions", "zsh"), ("skills", "add"), ("mcp", "add")] {
        let result =
            collision_app().execute_from(["collisions", command, value, "--format", "json"]).await;
        assert_eq!(result.exit_code, 0, "{}: {}", command, result.stderr);
        let output: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(output["data"], json!({"command": command, "value": value}));
    }
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
    let request = mcp_request(
        serde_json::to_vec(&json!({"jsonrpc":"2.0","id":0,"method":"initialize"})).unwrap(),
    );
    let response = app.handle_http(request).await;
    let value: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(value["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(value["result"]["instructions"], "Use fixture tools in tests.");

    let request = mcp_request(
        serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).unwrap(),
    );
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

    let request = mcp_request(
        serde_json::to_vec(&json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"tools/call",
            "params":{"name":"greet","arguments":{"name":"Ada","count":2}}
        }))
        .unwrap(),
    );
    let response = app.handle_http(request).await;
    let value: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(value["result"]["structuredContent"]["message"], "hello Ada");

    let request = mcp_request(
        serde_json::to_vec(&json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{"name":"fail","arguments":{}}
        }))
        .unwrap(),
    );
    let response = app.handle_http(request).await;
    let value: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(value["result"]["isError"], true);
    assert_eq!(value["result"]["content"][0]["text"], "requested failure");
}

#[cfg(all(feature = "http", feature = "mcp"))]
#[tokio::test]
async fn validates_mcp_http_and_jsonrpc_protocol_edges() {
    let mut get = incur::HttpRequest::new(Vec::new());
    *get.uri_mut() = "/mcp".parse().unwrap();
    let response = app().handle_http(get).await;
    assert_eq!(response.status(), 405);
    assert_eq!(response.headers()["allow"], "POST");

    let invalid =
        mcp_request(serde_json::to_vec(&json!({"jsonrpc":"1.0","id":1,"method":"ping"})).unwrap());
    let response = app().handle_http(invalid).await;
    let value: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(value["error"]["code"], -32600);

    let notification = mcp_request(
        serde_json::to_vec(&json!({
            "jsonrpc":"2.0",
            "method":"notifications/initialized"
        }))
        .unwrap(),
    );
    let response = app().handle_http(notification).await;
    assert_eq!(response.status(), 202);
    assert!(response.body().is_empty());

    let unknown_tool = mcp_request(
        serde_json::to_vec(&json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"tools/call",
            "params":{"name":"does_not_exist","arguments":{}}
        }))
        .unwrap(),
    );
    let response = app().handle_http(unknown_tool).await;
    let value: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(value["error"]["code"], -32602);

    let call_with_cta = mcp_request(
        serde_json::to_vec(&json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{"name":"greet","arguments":{"name":"Ada"}}
        }))
        .unwrap(),
    );
    let response = app_with_cta().handle_http(call_with_cta).await;
    let value: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(value["result"]["_meta"]["cta"]["commands"][0]["command"], "greet");
}
