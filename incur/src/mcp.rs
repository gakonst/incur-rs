use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::{App, Error, InternalResult as Result};

const PROTOCOL_VERSION: &str = "2025-11-25";

pub(crate) async fn serve_stdio<T>(app: App<T>) -> Result<()>
where
    T: clap::Parser + Send + 'static,
{
    let mut input = BufReader::new(tokio::io::stdin()).lines();
    let mut output = tokio::io::stdout();
    while let Some(line) = input.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                let response =
                    jsonrpc_error(Value::Null, -32700, format!("invalid JSON: {error}"), None);
                output.write_all(serde_json::to_string(&response)?.as_bytes()).await?;
                output.write_all(b"\n").await?;
                output.flush().await?;
                continue;
            }
        };
        if let Some(response) = handle_jsonrpc(&app, request).await {
            output.write_all(serde_json::to_string(&response)?.as_bytes()).await?;
            output.write_all(b"\n").await?;
            output.flush().await?;
        }
    }
    Ok(())
}

pub(crate) async fn handle_jsonrpc<T>(app: &App<T>, request: Value) -> Option<Value>
where
    T: clap::Parser + Send + 'static,
{
    let Some(object) = request.as_object() else {
        return Some(jsonrpc_error(
            Value::Null,
            -32600,
            "JSON-RPC request must be an object",
            None,
        ));
    };
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(jsonrpc_error(Value::Null, -32600, "jsonrpc must be \"2.0\"", None));
    }
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Some(jsonrpc_error(
            Value::Null,
            -32600,
            "JSON-RPC request requires a string method",
            None,
        ));
    };
    let notification = !object.contains_key("id");
    let id = object.get("id").cloned().unwrap_or(Value::Null);
    let response = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": {
                "name": app.manifest.name,
                "version": app.manifest.version.as_deref().unwrap_or("0.0.0"),
            },
            "instructions": app.mcp_instructions.clone().unwrap_or_else(|| format!(
                "Use these tools to invoke {} commands. Tool results are structured command envelopes.",
                app.manifest.name
            )),
        })),
        "ping" => Ok(json!({})),
        "notifications/initialized" | "notifications/cancelled" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": list_tools(app)})),
        "tools/call" => {
            let params = object.get("params").and_then(Value::as_object);
            let name = params.and_then(|params| params.get("name")).and_then(Value::as_str);
            let arguments = params
                .and_then(|params| params.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            match name {
                Some(name) => app.call_tool(name, arguments).await.map(tool_result),
                None => Err(Error::new("INVALID_PARAMS", "tools/call requires params.name")),
            }
        }
        _ => Err(Error::new("METHOD_NOT_FOUND", format!("unknown MCP method `{method}`"))),
    };

    if notification {
        return None;
    }

    Some(match response {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(error) => {
            let code = rpc_code(&error.code);
            let message = error.message.clone();
            jsonrpc_error(id, code, message, serde_json::to_value(error).ok())
        }
    })
}

fn tool_result(envelope: Value) -> Value {
    let ok = envelope.get("ok").and_then(Value::as_bool).unwrap_or(true);
    if !ok {
        let error = envelope.get("error").cloned().unwrap_or_else(|| envelope.clone());
        let text =
            if error.get("fieldErrors").and_then(Value::as_array).is_some_and(|v| !v.is_empty()) {
                serde_json::to_string(&error).unwrap_or_else(|_| "command failed".to_owned())
            } else {
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| "command failed".to_owned())
            };
        return json!({
            "content": [{"type": "text", "text": text}],
            "isError": true,
        });
    }

    let data = envelope.get("data").cloned().unwrap_or(Value::Null);
    let mut result = Map::from_iter([(
        "content".to_owned(),
        json!([{"type": "text", "text": serde_json::to_string(&data).unwrap_or_default()}]),
    )]);
    if data.is_object() {
        result.insert("structuredContent".to_owned(), data);
    }
    if let Some(cta) = envelope.pointer("/meta/cta").cloned() {
        result.insert("_meta".to_owned(), json!({"cta": cta}));
    }
    Value::Object(result)
}

fn jsonrpc_error(id: Value, code: i32, message: impl Into<String>, data: Option<Value>) -> Value {
    let mut error = Map::from_iter([
        ("code".to_owned(), Value::Number(code.into())),
        ("message".to_owned(), Value::String(message.into())),
    ]);
    if let Some(data) = data {
        error.insert("data".to_owned(), data);
    }
    json!({"jsonrpc": "2.0", "id": id, "error": error})
}

fn list_tools<T>(app: &App<T>) -> Vec<Value> {
    let mut tools = app
        .manifest
        .commands
        .iter()
        .filter(|command| command.mcp)
        .map(|command| {
            let mut tool = Map::from_iter([
                ("name".to_owned(), Value::String(command.tool_name())),
                ("inputSchema".to_owned(), command.input_schema.clone()),
                ("outputSchema".to_owned(), command.output_schema.clone()),
            ]);
            if let Some(description) = &command.description {
                tool.insert("description".to_owned(), Value::String(description.clone()));
            }
            if let Some(annotations) = &command.annotations {
                tool.insert(
                    "annotations".to_owned(),
                    serde_json::to_value(annotations).unwrap_or_default(),
                );
            }
            if let Some(instructions) = &command.instructions {
                tool.insert("_meta".to_owned(), json!({"instructions": instructions}));
            }
            Value::Object(tool)
        })
        .collect::<Vec<_>>();
    tools.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    tools
}

fn rpc_code(code: &str) -> i32 {
    match code {
        "METHOD_NOT_FOUND" => -32601,
        "INVALID_PARAMS" | "VALIDATION_ERROR" => -32602,
        _ => -32603,
    }
}

pub(crate) fn register(name: &str, command: &str, target: Option<&str>) -> Result<Vec<String>> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Error::new("HOME_NOT_FOUND", "HOME is not set"))?;
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let codex_home =
        std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".codex"));
    let normalized_target = target.map(|target| match target.to_ascii_lowercase().as_str() {
        "claude-code" => "claude".to_owned(),
        value => value.to_owned(),
    });
    let targets = [
        ("claude", home.join(".claude.json"), "mcpServers"),
        ("cursor", home.join(".cursor/mcp.json"), "mcpServers"),
        ("amp", config_home.join("amp/settings.json"), "amp.mcpServers"),
    ];
    let mut written = Vec::new();
    for (agent, path, key) in targets {
        if normalized_target.as_deref().is_some_and(|target| target != agent) {
            continue;
        }
        if target.is_none() && !path.exists() && !path.parent().is_some_and(Path::exists) {
            continue;
        }
        register_json(&path, key, name, command)?;
        written.push(path.display().to_string());
    }
    let codex_path = codex_home.join("config.toml");
    if normalized_target.as_deref().is_none_or(|target| target == "codex")
        && (normalized_target.is_some() || codex_home.exists())
    {
        register_codex(&codex_path, name, command)?;
        written.push(codex_path.display().to_string());
    }
    if written.is_empty() {
        return Err(Error::new(
            "AGENT_NOT_FOUND",
            target.map_or_else(
                || "no supported agent installation was detected".to_owned(),
                |target| format!("unsupported or unavailable agent `{target}`"),
            ),
        ));
    }
    Ok(written)
}

fn register_codex(path: &Path, name: &str, command: &str) -> Result<()> {
    let source = if path.exists() { fs::read_to_string(path)? } else { String::new() };
    let mut document = source
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| Error::new("CONFIG_ERROR", error.to_string()))?;
    let tokens = split_command(command)?;
    let Some((executable, args)) = tokens.split_first() else {
        return Err(Error::new("CONFIG_ERROR", "MCP command cannot be empty"));
    };
    document["mcp_servers"][name]["command"] = toml_edit::value(executable);
    let mut array = toml_edit::Array::new();
    for argument in args {
        array.push(argument);
    }
    document["mcp_servers"][name]["args"] = toml_edit::value(array);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, document.to_string())?;
    Ok(())
}

fn register_json(path: &Path, key: &str, name: &str, command: &str) -> Result<()> {
    let mut config =
        if path.exists() { serde_json::from_slice::<Value>(&fs::read(path)?)? } else { json!({}) };
    let root = config.as_object_mut().ok_or_else(|| {
        Error::new("CONFIG_ERROR", format!("{} is not a JSON object", path.display()))
    })?;
    let servers = root.entry(key).or_insert_with(|| json!({}));
    let servers = servers.as_object_mut().ok_or_else(|| {
        Error::new("CONFIG_ERROR", format!("{key} in {} is not an object", path.display()))
    })?;
    let tokens = split_command(command)?;
    let Some((executable, args)) = tokens.split_first() else {
        return Err(Error::new("CONFIG_ERROR", "MCP command cannot be empty"));
    };
    servers.insert(name.to_owned(), json!({"command": executable, "args": args}));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{}\n", serde_json::to_string_pretty(&config)?))?;
    Ok(())
}

fn split_command(input: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut started = false;
    for character in input.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            started = true;
        } else if character == '\\' && quote != Some('\'') {
            escaped = true;
            started = true;
        } else if let Some(active) = quote {
            if character == active {
                quote = None;
            } else {
                current.push(character);
            }
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
            started = true;
        } else if character.is_whitespace() {
            if started {
                tokens.push(std::mem::take(&mut current));
                started = false;
            }
        } else {
            current.push(character);
            started = true;
        }
    }
    if escaped {
        return Err(Error::new("CONFIG_ERROR", "MCP command ends with an escape character"));
    }
    if quote.is_some() {
        return Err(Error::new("CONFIG_ERROR", "MCP command contains an unterminated quote"));
    }
    if started {
        tokens.push(current);
    }
    Ok(tokens)
}

impl From<io::ErrorKind> for Error {
    fn from(kind: io::ErrorKind) -> Self {
        Self::new("IO_ERROR", kind.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{register_codex, register_json, split_command};

    #[test]
    fn splits_quoted_and_escaped_registration_commands() {
        assert_eq!(
            split_command(r#""/path/to my/cli" --name hello\ world """#).unwrap(),
            ["/path/to my/cli", "--name", "hello world", ""]
        );
    }

    #[test]
    fn rejects_incomplete_registration_commands() {
        assert!(split_command(r#"cli "unterminated"#).is_err());
        assert!(split_command("cli \\").is_err());
    }

    #[test]
    fn preserves_existing_agent_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let json_path = temp.path().join("agent.json");
        std::fs::write(&json_path, r#"{"theme":"dark"}"#).unwrap();
        register_json(&json_path, "mcpServers", "fixture", r#""/path/to cli" --mcp"#).unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&json_path).unwrap()).unwrap();
        assert_eq!(json["theme"], "dark");
        assert_eq!(json["mcpServers"]["fixture"]["command"], "/path/to cli");
        assert_eq!(json["mcpServers"]["fixture"]["args"], serde_json::json!(["--mcp"]));

        let toml_path = temp.path().join("config.toml");
        std::fs::write(&toml_path, "model = \"gpt\"\n").unwrap();
        register_codex(&toml_path, "fixture", "fixture --mcp").unwrap();
        let toml = std::fs::read_to_string(toml_path).unwrap();
        assert!(toml.contains("model = \"gpt\""));
        let document = toml.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(document["mcp_servers"]["fixture"]["command"].as_str(), Some("fixture"));
        assert_eq!(
            document["mcp_servers"]["fixture"]["args"].as_array().unwrap().get(0).unwrap().as_str(),
            Some("--mcp")
        );
    }
}
