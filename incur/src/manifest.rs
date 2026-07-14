use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// MCP behavior hints advertised to agent clients.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    /// Human-readable tool title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Whether the tool only reads state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only_hint: Option<bool>,
    /// Whether the tool may perform destructive updates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destructive_hint: Option<bool>,
    /// Whether repeated calls with the same input have no additional effect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotent_hint: Option<bool>,
    /// Whether the tool may interact with external entities.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_world_hint: Option<bool>,
}

/// Per-command MCP and skill-generation settings.
#[derive(Clone, Debug, Default)]
pub struct ToolConfig {
    /// Override the MCP tool name.
    pub name: Option<String>,
    /// Override the description exposed through MCP.
    pub description: Option<String>,
    /// Tool behavior annotations.
    pub annotations: Option<ToolAnnotations>,
    /// Extra instructions included in tool metadata and generated skills.
    pub instructions: Option<String>,
    /// Hide the command from MCP while keeping it available on the CLI.
    pub hidden: bool,
}

/// One parsed argument or option, used to bridge transports to argv.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct InputInfo {
    /// Argument identifier.
    pub name: String,
    /// Long option name.
    pub long: Option<String>,
    /// Short option name.
    pub short: Option<char>,
    /// Help text.
    pub description: Option<String>,
    /// Whether the command requires this input.
    pub required: bool,
    /// Whether the input is positional.
    pub positional: bool,
    /// Positional index, or [`usize::MAX`] for options.
    pub index: usize,
    /// Whether multiple values are accepted.
    pub array: bool,
    /// Whether an array option repeats its flag for each value.
    pub repeatable: bool,
    /// Whether the option is a boolean switch.
    pub boolean: bool,
    /// Whether the option counts repeated flag occurrences.
    pub count: bool,
    /// Whether the switch sets its field to false when present.
    pub false_action: bool,
    /// JSON Schema for the input.
    pub schema: Value,
}

/// Agent-readable information about one executable command.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandInfo {
    /// Space-separated subcommand path. Empty for a root command.
    pub name: String,
    /// Command description.
    pub description: Option<String>,
    /// Merged JSON Schema for positional arguments and options.
    pub input_schema: Value,
    /// JSON Schema for successful output.
    pub output_schema: Value,
    /// MCP tool-name override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name_override: Option<String>,
    /// MCP behavior annotations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ToolAnnotations>,
    /// Extra agent instructions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Whether MCP clients may discover this command.
    pub mcp: bool,
    /// Positional input metadata.
    #[serde(skip)]
    pub(crate) positionals: Vec<InputInfo>,
    /// Named option metadata.
    #[serde(skip)]
    pub(crate) options: Vec<InputInfo>,
}

impl CommandInfo {
    /// Returns the MCP-compatible tool name.
    pub fn tool_name(&self) -> String {
        self.tool_name_override.clone().unwrap_or_else(|| {
            if self.name.is_empty() { "root".to_owned() } else { self.name.replace(' ', "_") }
        })
    }

    /// Returns a shell-style command signature.
    pub fn signature(&self, root: &str) -> String {
        let mut pieces = vec![root.to_owned()];
        if !self.name.is_empty() {
            pieces.push(self.name.clone());
        }
        pieces.extend(self.positionals.iter().map(|input| {
            if input.required { format!("<{}>", input.name) } else { format!("[{}]", input.name) }
        }));
        pieces.join(" ")
    }
}

/// A complete CLI manifest.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Executable name.
    pub name: String,
    /// CLI version.
    pub version: Option<String>,
    /// CLI description.
    pub description: Option<String>,
    /// Executable commands.
    pub commands: Vec<CommandInfo>,
}

impl Manifest {
    /// Returns a manifest containing only one command subtree while preserving full paths.
    pub fn filtered(&self, path: &str) -> Self {
        if path.is_empty() {
            return self.clone();
        }
        let prefix = format!("{path} ");
        let mut manifest = self.clone();
        manifest
            .commands
            .retain(|command| command.name == path || command.name.starts_with(&prefix));
        manifest.commands.sort_by(|left, right| left.name.cmp(&right.name));
        manifest
    }

    /// Returns a manifest scoped to one command or command group.
    pub fn scoped(&self, path: &str) -> Self {
        if path.is_empty() {
            return self.clone();
        }
        let prefix = format!("{path} ");
        let mut commands = self
            .commands
            .iter()
            .filter_map(|command| {
                let mut command = command.clone();
                command.name = if command.name == path {
                    String::new()
                } else {
                    command.name.strip_prefix(&prefix)?.to_owned()
                };
                Some(command)
            })
            .collect::<Vec<_>>();
        commands.sort_by(|left, right| left.name.cmp(&right.name));
        Self {
            name: format!("{} {path}", self.name),
            version: self.version.clone(),
            description: commands
                .iter()
                .find(|command| command.name.is_empty())
                .and_then(|command| command.description.clone()),
            commands,
        }
    }

    /// Returns the compact structured `incur.v1` command index.
    pub fn index_value(&self) -> Value {
        let mut source = self.commands.iter().collect::<Vec<_>>();
        source.sort_by(|left, right| left.name.cmp(&right.name));
        let commands = source
            .into_iter()
            .map(|command| {
                let mut entry =
                    Map::from_iter([("name".to_owned(), Value::String(command.name.clone()))]);
                if let Some(description) = &command.description {
                    entry.insert("description".to_owned(), Value::String(description.clone()));
                }
                Value::Object(entry)
            })
            .collect::<Vec<_>>();
        json!({"version": "incur.v1", "commands": commands})
    }

    /// Returns the complete structured `incur.v1` command manifest.
    pub fn full_value(&self) -> Value {
        let mut source = self.commands.iter().collect::<Vec<_>>();
        source.sort_by(|left, right| left.name.cmp(&right.name));
        let commands = source
            .into_iter()
            .map(|command| {
                let mut entry = Map::from_iter([
                    ("name".to_owned(), Value::String(command.name.clone())),
                    (
                        "schema".to_owned(),
                        json!({
                            "input": command.input_schema,
                            "output": command.output_schema,
                        }),
                    ),
                ]);
                if let Some(description) = &command.description {
                    entry.insert("description".to_owned(), Value::String(description.clone()));
                }
                if let Some(instructions) = &command.instructions {
                    entry.insert("instructions".to_owned(), Value::String(instructions.clone()));
                }
                Value::Object(entry)
            })
            .collect::<Vec<_>>();
        json!({"version": "incur.v1", "commands": commands})
    }

    /// Renders a compact Markdown index for `--llms`.
    pub fn markdown_index(&self) -> String {
        let mut lines = vec![format!("# {}", self.name)];
        if let Some(description) = &self.description {
            lines.extend([String::new(), description.clone()]);
        }
        lines.extend([
            String::new(),
            "| Command | Description |".to_owned(),
            "| --- | --- |".to_owned(),
        ]);
        lines.extend(self.commands.iter().map(|command| {
            format!(
                "| `{}` | {} |",
                command.signature(&self.name),
                command.description.as_deref().unwrap_or_default()
            )
        }));
        lines.extend([
            String::new(),
            format!(
                "Run `{} --llms-full` for the full manifest. Run `{} <command> --schema` for JSON Schema.",
                self.name, self.name
            ),
        ]);
        lines.join("\n")
    }

    /// Renders the complete skill-style Markdown manifest.
    pub fn markdown_full(&self) -> String {
        let mut sections = vec![self.markdown_index()];
        for command in &self.commands {
            let title = command.signature(&self.name);
            let mut section = format!("## {title}");
            if let Some(description) = &command.description {
                section.push_str(&format!("\n\n{description}"));
            }
            section.push_str("\n\n### Input Schema\n\n```json\n");
            section
                .push_str(&serde_json::to_string_pretty(&command.input_schema).unwrap_or_default());
            section.push_str("\n```\n\n### Output Schema\n\n```json\n");
            section.push_str(
                &serde_json::to_string_pretty(&command.output_schema).unwrap_or_default(),
            );
            section.push_str("\n```");
            sections.push(section);
        }
        sections.join("\n\n")
    }

    /// Finds a command by its space-separated path.
    pub fn command(&self, path: &str) -> Option<&CommandInfo> {
        self.commands.iter().find(|command| command.name == path)
    }

    /// Finds a command by its MCP tool name.
    pub fn tool(&self, name: &str) -> Option<&CommandInfo> {
        self.commands.iter().find(|command| command.mcp && command.tool_name() == name)
    }

    /// Generates an OpenAPI 3.1 document for the HTTP command interface.
    pub fn openapi(&self) -> Value {
        let mut paths = Map::new();
        for command in &self.commands {
            let mut path = if command.name.is_empty() {
                "/".to_owned()
            } else {
                format!("/{}", command.name.replace(' ', "/"))
            };
            for positional in &command.positionals {
                path.push_str(&format!("/{{{}}}", positional.name));
            }
            let parameters = command
                .positionals
                .iter()
                .map(|input| {
                    json!({
                        "name": input.name,
                        "in": "path",
                        "required": true,
                        "description": input.description,
                        "schema": input.schema,
                    })
                })
                .chain(command.options.iter().map(|input| {
                    json!({
                        "name": input.long.as_deref().unwrap_or(&input.name),
                        "in": "query",
                        "required": input.required,
                        "description": input.description,
                        "schema": input.schema,
                    })
                }))
                .collect::<Vec<_>>();
            let operation = json!({
                "operationId": command.tool_name(),
                "summary": command.description,
                "parameters": parameters,
                "responses": {
                    "200": {
                        "description": "Command succeeded",
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "ok": {"const": true},
                                        "data": command.output_schema,
                                        "meta": {"type": "object"}
                                    },
                                    "required": ["ok", "data", "meta"]
                                }
                            }
                        }
                    },
                    "400": {"description": "Invalid command input"},
                    "500": {"description": "Command failed"}
                }
            });
            paths.insert(
                path,
                Value::Object(Map::from_iter([(http_method(&command.name).to_owned(), operation)])),
            );
        }
        json!({
            "openapi": "3.1.0",
            "info": {
                "title": self.name,
                "version": self.version.as_deref().unwrap_or("0.0.0"),
                "description": self.description,
            },
            "paths": paths,
        })
    }
}

fn http_method(command: &str) -> &'static str {
    let action = command.split_whitespace().last().unwrap_or_default();
    if ["get", "list", "search", "read", "show", "status", "check", "inspect"].contains(&action) {
        "get"
    } else if ["update", "set", "edit", "patch"].contains(&action) {
        "patch"
    } else if ["delete", "remove", "destroy"].contains(&action) {
        "delete"
    } else {
        "post"
    }
}
