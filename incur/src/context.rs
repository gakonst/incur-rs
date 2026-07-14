use std::{
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::Error;

/// A supported structured output format.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Token-Oriented Object Notation.
    #[default]
    Toon,
    /// Pretty-printed JSON.
    Json,
    /// YAML.
    #[cfg(feature = "yaml")]
    Yaml,
    /// Markdown tables and sections.
    Markdown,
    /// Newline-delimited JSON.
    Jsonl,
}

/// Controls whether successful data is shown in an interactive terminal.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputPolicy {
    /// Show output to humans and agents.
    #[default]
    All,
    /// Suppress data in a TTY unless a structured format or full envelope was requested.
    AgentOnly,
}

/// JSON configuration-file discovery settings.
#[derive(Clone, Debug)]
pub struct Config {
    /// Global flag used to choose a file, without leading dashes.
    pub flag: String,
    /// Candidate files in priority order. The first existing file is loaded.
    pub files: Vec<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self { flag: "config".to_owned(), files: Vec::new() }
    }
}

impl Config {
    /// Creates configuration using `--config` and `<cli>.json` discovery.
    pub fn new() -> Self {
        Self::default()
    }

    /// Changes the global configuration flag.
    #[must_use]
    pub fn flag(mut self, flag: impl Into<String>) -> Self {
        self.flag = flag.into().trim_start_matches('-').to_owned();
        self
    }

    /// Adds a candidate configuration file.
    #[must_use]
    pub fn file(mut self, path: impl Into<PathBuf>) -> Self {
        self.files.push(path.into());
        self
    }
}

impl OutputFormat {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "toon" => Some(Self::Toon),
            "json" => Some(Self::Json),
            #[cfg(feature = "yaml")]
            "yaml" => Some(Self::Yaml),
            "md" | "markdown" => Some(Self::Markdown),
            "jsonl" | "ndjson" => Some(Self::Jsonl),
            _ => None,
        }
    }
}

/// A command suggested after a successful or failed operation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cta {
    /// Subcommand path, without the executable name.
    pub command: String,
    /// Positional arguments appended in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Value>,
    /// Named options rendered as flags.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub options: Map<String, Value>,
    /// Explanation of why an agent may want to run the command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Cta {
    /// Creates a suggested command.
    pub fn new(command: impl Into<String>) -> Self {
        Self { command: command.into(), args: Vec::new(), options: Map::new(), description: None }
    }

    /// Appends one positional argument.
    #[must_use]
    pub fn arg(mut self, value: impl Into<Value>) -> Self {
        self.args.push(value.into());
        self
    }

    /// Adds one named option.
    #[must_use]
    pub fn option(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.options.insert(name.into(), value.into());
        self
    }

    /// Describes the suggested command.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// A group of suggested follow-up commands.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CtaBlock {
    /// Suggested commands.
    pub commands: Vec<Cta>,
    /// Heading displayed before the commands.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl CtaBlock {
    /// Creates a CTA block.
    pub fn new(commands: impl IntoIterator<Item = Cta>) -> Self {
        Self { commands: commands.into_iter().collect(), description: None }
    }

    /// Adds a heading to the CTA block.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// Information and helpers available to every command handler.
#[derive(Clone, Debug)]
pub struct Context {
    /// Whether output is being consumed by an agent or pipe.
    pub agent: bool,
    /// Root executable name.
    pub name: String,
    /// Resolved subcommand path, separated by spaces.
    pub command: String,
    /// Package version reported by the command definition.
    pub version: Option<String>,
    /// Selected output format.
    pub format: OutputFormat,
    /// Whether the caller explicitly selected an output format.
    pub format_explicit: bool,
    pub(crate) cta: Arc<Mutex<Option<CtaBlock>>>,
    pub(crate) vars: Arc<RwLock<Map<String, Value>>>,
}

impl Context {
    pub(crate) fn new(
        agent: bool,
        name: String,
        command: String,
        version: Option<String>,
        format: OutputFormat,
        format_explicit: bool,
    ) -> Self {
        Self {
            agent,
            name,
            command,
            version,
            format,
            format_explicit,
            cta: Arc::new(Mutex::new(None)),
            vars: Arc::new(RwLock::new(Map::new())),
        }
    }

    /// Attaches suggested next commands to a successful response.
    pub fn suggest(&self, cta: CtaBlock) {
        *self.cta.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(cta);
    }

    /// Creates a structured command error.
    pub fn error(
        &self,
        code: impl Into<std::borrow::Cow<'static, str>>,
        message: impl Into<String>,
    ) -> Error {
        Error::new(code, message)
    }

    /// Sets a value shared with later middleware and the command handler.
    pub fn set_var(&self, name: impl Into<String>, value: impl Into<Value>) {
        let name = name.into();
        let value = value.into();
        self.vars.write().unwrap_or_else(std::sync::PoisonError::into_inner).insert(name, value);
    }

    /// Reads a value set by middleware.
    pub fn var(&self, name: &str) -> Option<Value> {
        self.vars.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(name).cloned()
    }

    pub(crate) fn take_cta(&self) -> Option<CtaBlock> {
        self.cta.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    }
}
