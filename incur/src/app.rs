use std::{
    collections::BTreeMap,
    ffi::OsString,
    fmt,
    future::Future,
    io::{IsTerminal, Write},
    marker::PhantomData,
    process::ExitCode,
    sync::{Arc, Mutex},
    time::Instant,
};

use clap::{CommandFactory, FromArgMatches, Parser};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::{
    Config, Context, Error, InternalResult as Result, JsonSchema, Manifest, Middleware,
    OutputFormat, OutputPolicy, ToolConfig,
    command::{self, GlobalOptions},
    filter, format,
    middleware::{BoxHandler, Next},
};

/// The captured result of one command invocation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Execution {
    /// Bytes intended for stdout, decoded as UTF-8.
    pub stdout: String,
    /// Bytes intended for stderr, decoded as UTF-8.
    pub stderr: String,
    /// Process exit code.
    pub exit_code: u8,
}

impl Execution {
    fn success(stdout: impl Into<String>) -> Self {
        Self { stdout: stdout.into(), stderr: String::new(), exit_code: 0 }
    }

    fn failure(stderr: impl Into<String>, exit_code: u8) -> Self {
        Self { stdout: String::new(), stderr: stderr.into(), exit_code }
    }
}

type ParsedHandler<T> = Arc<dyn Fn(T, Context) -> crate::middleware::BoxFuture + Send + Sync>;

/// A configured agent-friendly CLI application.
pub struct App<T> {
    pub(crate) command: clap::Command,
    pub(crate) handler: ParsedHandler<T>,
    pub(crate) manifest: Manifest,
    pub(crate) middlewares: Vec<Arc<dyn Middleware>>,
    pub(crate) command_middlewares: BTreeMap<String, Vec<Arc<dyn Middleware>>>,
    pub(crate) output_policy: OutputPolicy,
    pub(crate) command_output_policies: BTreeMap<String, OutputPolicy>,
    pub(crate) config: Option<Config>,
    #[cfg(feature = "mcp")]
    pub(crate) mcp_instructions: Option<String>,
    _marker: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for App<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("App")
            .field("name", &self.manifest.name)
            .field("commands", &self.manifest.commands.len())
            .field("middlewares", &self.middlewares.len())
            .field("command_middlewares", &self.command_middlewares.len())
            .finish_non_exhaustive()
    }
}

impl<T> Clone for App<T> {
    fn clone(&self) -> Self {
        Self {
            command: self.command.clone(),
            handler: self.handler.clone(),
            manifest: self.manifest.clone(),
            middlewares: self.middlewares.clone(),
            command_middlewares: self.command_middlewares.clone(),
            output_policy: self.output_policy,
            command_output_policies: self.command_output_policies.clone(),
            config: self.config.clone(),
            #[cfg(feature = "mcp")]
            mcp_instructions: self.mcp_instructions.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T> App<T>
where
    T: Parser + Send + 'static,
{
    /// Creates an app from a derived command type and an async command handler.
    pub fn new<O, H, Fut>(handler: H) -> Self
    where
        O: JsonSchema + Serialize + Send + 'static,
        H: Fn(T, Context) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = eyre::Result<O>> + Send + 'static,
    {
        let command = T::command().disable_help_subcommand(true);
        let mut reflected = command.clone();
        reflected.build();
        let output_schema = serde_json::to_value(schemars::schema_for!(O))
            .expect("schemars schemas always serialize as JSON");
        let manifest = Manifest {
            name: command.get_name().to_owned(),
            version: command.get_version().map(str::to_owned),
            description: command
                .get_about()
                .or_else(|| command.get_long_about())
                .map(ToString::to_string),
            commands: command::collect(&reflected, &output_schema),
        };
        let handler = Arc::new(handler);
        let erased = Arc::new(move |input: T, context: Context| {
            let handler = handler.clone();
            Box::pin(async move {
                let output = handler(input, context).await.map_err(Error::from)?;
                serde_json::to_value(output).map_err(Into::into)
            }) as crate::middleware::BoxFuture
        });
        Self {
            command,
            handler: erased,
            manifest,
            middlewares: Vec::new(),
            command_middlewares: BTreeMap::new(),
            output_policy: OutputPolicy::All,
            command_output_policies: BTreeMap::new(),
            config: None,
            #[cfg(feature = "mcp")]
            mcp_instructions: None,
            _marker: PhantomData,
        }
    }

    /// Adds onion-style middleware around every command invocation.
    #[must_use]
    pub fn middleware(mut self, middleware: impl Middleware) -> Self {
        self.middlewares.push(Arc::new(middleware));
        self
    }

    /// Adds middleware that only wraps one space-separated subcommand path.
    #[must_use]
    pub fn middleware_for(mut self, path: impl Into<String>, middleware: impl Middleware) -> Self {
        self.command_middlewares.entry(path.into()).or_default().push(Arc::new(middleware));
        self
    }

    /// Sets the default interactive output policy.
    #[must_use]
    pub const fn output_policy(mut self, policy: OutputPolicy) -> Self {
        self.output_policy = policy;
        self
    }

    /// Overrides the interactive output policy for one subcommand path.
    #[must_use]
    pub fn output_policy_for(mut self, path: impl Into<String>, policy: OutputPolicy) -> Self {
        self.command_output_policies.insert(path.into(), policy);
        self
    }

    /// Configures the MCP tool generated for one subcommand path.
    #[must_use]
    pub fn tool(mut self, path: &str, config: ToolConfig) -> Self {
        if let Some(command) =
            self.manifest.commands.iter_mut().find(|command| command.name == path)
        {
            command.tool_name_override = config.name;
            if config.description.is_some() {
                command.description = config.description;
            }
            command.annotations = config.annotations;
            command.instructions = config.instructions;
            command.mcp = !config.hidden;
        }
        self
    }

    /// Enables JSON configuration files for named-option defaults.
    #[must_use]
    pub fn config(mut self, config: Config) -> Self {
        self.config = Some(config);
        self
    }

    /// Overrides the instructions advertised when MCP clients initialize.
    #[cfg(feature = "mcp")]
    #[must_use]
    pub fn mcp_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.mcp_instructions = Some(instructions.into());
        self
    }

    /// Overrides the output schema for one subcommand path.
    #[must_use]
    pub fn output_schema_for<O: JsonSchema>(mut self, path: &str) -> Self {
        if let Some(command) =
            self.manifest.commands.iter_mut().find(|command| command.name == path)
        {
            command.output_schema = serde_json::to_value(schemars::schema_for!(O))
                .expect("schemars schemas always serialize as JSON");
        }
        self
    }

    /// Returns the generated command manifest.
    pub const fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Parses the process arguments, executes the selected command, and writes its output.
    pub async fn serve(self) -> ExitCode {
        let argv = std::env::args_os().collect::<Vec<_>>();

        #[cfg(feature = "mcp")]
        if argv.iter().skip(1).any(|arg| arg == "--mcp") {
            if let Err(error) = command::augmented(
                self.command.clone(),
                self.config.as_ref().map(|config| config.flag.as_str()),
            ) {
                let _ = writeln!(std::io::stderr(), "{error}");
                return ExitCode::from(error.exit_code);
            }
            return match crate::mcp::serve_stdio(self).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    let _ = writeln!(std::io::stderr(), "{error}");
                    ExitCode::from(error.exit_code)
                }
            };
        }

        let agent = !std::io::stdout().is_terminal();
        let execution = self.execute_inner(argv, agent).await;
        if !execution.stdout.is_empty() {
            let _ = writeln!(std::io::stdout(), "{}", execution.stdout.trim_end());
        }
        if !execution.stderr.is_empty() {
            let _ = writeln!(std::io::stderr(), "{}", execution.stderr.trim_end());
        }
        ExitCode::from(execution.exit_code)
    }

    /// Executes an argv sequence without writing to the process streams.
    ///
    /// Captured executions use agent mode, so successful output includes the complete envelope.
    pub async fn execute_from<I, S>(&self, argv: I) -> Execution
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString> + Clone,
    {
        self.execute_inner(argv.into_iter().map(Into::into).collect(), true).await
    }

    pub(crate) async fn execute_inner(&self, argv: Vec<OsString>, agent: bool) -> Execution {
        if let Some(execution) = self.special(&argv).await {
            return execution;
        }

        let argv = match self.apply_config(argv) {
            Ok(argv) => argv,
            Err(error) => return Execution::failure(error.to_string(), error.exit_code),
        };
        let mut matches = match command::augmented(
            self.command.clone(),
            self.config.as_ref().map(|config| config.flag.as_str()),
        )
        .and_then(|command| command.try_get_matches_from(argv).map_err(clap_error))
        {
            Ok(matches) => matches,
            Err(error) => return execution_error(error),
        };
        let globals = GlobalOptions::from_matches(&matches);
        let path = command::command_path(&matches).join(" ");
        let input = match T::from_arg_matches_mut(&mut matches) {
            Ok(input) => input,
            Err(error) => {
                let error = clap_error(error);
                return execution_error(error);
            }
        };
        self.execute_parsed(input, path, globals, agent).await
    }

    async fn execute_parsed(
        &self,
        input: T,
        path: String,
        globals: GlobalOptions,
        agent: bool,
    ) -> Execution {
        let context = Context::new(
            agent,
            self.manifest.name.clone(),
            path.clone(),
            self.manifest.version.clone(),
            globals.format,
            globals.format_explicit,
        );
        let cta_context = context.clone();
        let handler = self.handler.clone();
        let input = Arc::new(Mutex::new(Some(input)));
        let leaf: BoxHandler = Arc::new(move |context| {
            let input = input.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
            match input {
                Some(input) => handler(input, context),
                None => Box::pin(async {
                    Err(Error::new(
                        "MIDDLEWARE_REENTRY",
                        "middleware may continue an invocation only once",
                    ))
                }),
            }
        });
        let mut middlewares = self.middlewares.clone();
        if let Some(command_middlewares) = self.command_middlewares.get(&path) {
            middlewares.extend(command_middlewares.iter().cloned());
        }
        let next = Next { middlewares: Arc::new(middlewares), handler: leaf, index: 0 };
        let start = Instant::now();
        let result = next.run(context).await;
        let duration = format_duration(start.elapsed());

        match result {
            Ok(mut data) => {
                if let Some(filter) = &globals.filter_output {
                    data = filter::apply(&data, filter);
                }
                #[cfg(feature = "tokens")]
                if globals.token_count {
                    return match format::render(&data, globals.format) {
                        Ok(rendered) => Execution::success(tokenize(&rendered).len().to_string()),
                        Err(error) => self.render_error(error, globals.format, agent, &path),
                    };
                }
                let cta = cta_context.take_cta();
                #[cfg(feature = "skills")]
                let cta = cta.or_else(|| crate::skills::stale_cta(&self.manifest));
                let mut meta = Map::from_iter([
                    ("command".to_owned(), Value::String(path.clone())),
                    ("duration".to_owned(), Value::String(duration)),
                ]);
                if let Some(cta) = &cta
                    && let Ok(cta) = serde_json::to_value(cta)
                {
                    meta.insert("cta".to_owned(), cta);
                }
                let full_output = globals.full_output || agent;
                let policy =
                    self.command_output_policies.get(&path).copied().unwrap_or(self.output_policy);
                let suppress_data = !agent
                    && policy == OutputPolicy::AgentOnly
                    && !globals.format_explicit
                    && !globals.full_output;

                #[cfg(feature = "tokens")]
                if !suppress_data && (globals.token_offset > 0 || globals.token_limit.is_some()) {
                    let rendered = match format::render(&data, globals.format) {
                        Ok(rendered) => rendered,
                        Err(error) => {
                            return self.render_error(error, globals.format, agent, &path);
                        }
                    };
                    let page =
                        paginate_tokens(&rendered, globals.token_offset, globals.token_limit);
                    if page.truncated {
                        if full_output {
                            if let Some(next_offset) = page.next_offset {
                                meta.insert("nextOffset".to_owned(), json!(next_offset));
                            }
                            data = Value::String(page.text);
                        } else {
                            let mut rendered = page.text;
                            if let Some(cta) = cta {
                                rendered.push_str(&render_cta(&self.manifest.name, &cta));
                            }
                            return Execution::success(rendered);
                        }
                    }
                }

                let value = if full_output {
                    Value::Object(Map::from_iter([
                        ("ok".to_owned(), Value::Bool(true)),
                        ("data".to_owned(), data),
                        ("meta".to_owned(), Value::Object(meta)),
                    ]))
                } else {
                    data
                };
                let mut rendered = if suppress_data {
                    String::new()
                } else {
                    match format::render(&value, globals.format) {
                        Ok(rendered) => rendered,
                        Err(error) => {
                            return self.render_error(error, globals.format, agent, &path);
                        }
                    }
                };
                if !agent && let Some(cta) = cta {
                    rendered.push_str(&render_cta(&self.manifest.name, &cta));
                }
                Execution::success(rendered)
            }
            Err(error) => self.render_error(error, globals.format, agent, &path),
        }
    }

    fn render_error(
        &self,
        error: Error,
        output_format: OutputFormat,
        agent: bool,
        command: &str,
    ) -> Execution {
        let exit_code = error.exit_code;
        if agent {
            let envelope = json!({
                "ok": false,
                "error": error,
                "meta": { "command": command },
            });
            match format::render(&envelope, output_format) {
                Ok(output) => Execution { stdout: output, stderr: String::new(), exit_code },
                Err(format_error) => Execution::failure(format_error.to_string(), exit_code),
            }
        } else {
            let mut output = format!("{}: {}", error.code, error.message);
            if let Some(cta) = error.cta {
                output.push_str(&render_cta(&self.manifest.name, &cta));
            }
            Execution::failure(output, exit_code)
        }
    }

    async fn special(&self, argv: &[OsString]) -> Option<Execution> {
        let args = argv.iter().skip(1).filter_map(|value| value.to_str()).collect::<Vec<_>>();
        if args.contains(&"--llms-full") {
            let path = resolve_raw_path(&self.command, &args);
            let manifest = self.manifest.scoped(&path);
            let format = match raw_format(&args) {
                Ok(format) => format,
                Err(error) => return Some(execution_error(error)),
            };
            return Some(if raw_format_explicit(&args) && format != OutputFormat::Markdown {
                match format::render(&self.manifest.filtered(&path).full_value(), format) {
                    Ok(output) => Execution::success(output),
                    Err(error) => Execution::failure(error.to_string(), error.exit_code),
                }
            } else {
                Execution::success(manifest.markdown_full())
            });
        }
        if args.contains(&"--llms") {
            let path = resolve_raw_path(&self.command, &args);
            let manifest = self.manifest.scoped(&path);
            let format = match raw_format(&args) {
                Ok(format) => format,
                Err(error) => return Some(execution_error(error)),
            };
            return Some(if raw_format_explicit(&args) && format != OutputFormat::Markdown {
                match format::render(&self.manifest.filtered(&path).index_value(), format) {
                    Ok(output) => Execution::success(output),
                    Err(error) => Execution::failure(error.to_string(), error.exit_code),
                }
            } else {
                Execution::success(manifest.markdown_index())
            });
        }
        if args.contains(&"--schema") {
            let path = resolve_raw_path(&self.command, &args);
            let schema = self
                .manifest
                .command(&path)
                .map(|command| {
                    json!({"input": command.input_schema, "output": command.output_schema})
                })
                .unwrap_or_else(|| json!({"commands": self.manifest.commands}));
            let format = match raw_format(&args) {
                Ok(format) => format,
                Err(error) => return Some(execution_error(error)),
            };
            return Some(match format::render(&schema, format) {
                Ok(output) => Execution::success(output),
                Err(error) => Execution::failure(error.to_string(), error.exit_code),
            });
        }
        #[cfg(feature = "completions")]
        if self.command.find_subcommand("completions").is_none()
            && args.first() == Some(&"completions")
        {
            let matches = match self.builtin_matches(argv) {
                Ok(matches) => matches,
                Err(execution) => return Some(execution),
            };
            let shell = matches
                .subcommand_matches("completions")
                .and_then(|matches| matches.get_one::<String>("shell"))
                .map(String::as_str);
            return Some(self.completions(shell));
        }
        #[cfg(feature = "skills")]
        if self.command.find_subcommand("skills").is_none() && args.first() == Some(&"skills") {
            let matches = match self.builtin_matches(argv) {
                Ok(matches) => matches,
                Err(execution) => return Some(execution),
            };
            let Some(skills) = matches.subcommand_matches("skills") else {
                return Some(Execution::failure("expected skills add or skills list", 2));
            };
            return Some(match skills.subcommand() {
                Some(("add", add)) => {
                    let global = !add.get_flag("project");
                    let depth = add.get_one::<usize>("depth").copied().unwrap_or(1);
                    match crate::skills::install(&self.manifest, global, depth) {
                        Ok(paths) => Execution::success(json!({"skills": paths}).to_string()),
                        Err(error) => Execution::failure(error.to_string(), error.exit_code),
                    }
                }
                Some(("list", _)) => {
                    Execution::success(crate::skills::list(&self.manifest, 1).to_string())
                }
                _ => Execution::failure("expected skills add or skills list", 2),
            });
        }
        #[cfg(feature = "mcp")]
        if self.command.find_subcommand("mcp").is_none() && args.first() == Some(&"mcp") {
            let matches = match self.builtin_matches(argv) {
                Ok(matches) => matches,
                Err(execution) => return Some(execution),
            };
            let Some(add) = matches
                .subcommand_matches("mcp")
                .and_then(|matches| matches.subcommand_matches("add"))
            else {
                return Some(Execution::failure("expected mcp add", 2));
            };
            let command = add
                .get_one::<String>("command")
                .cloned()
                .unwrap_or_else(|| format!("{} --mcp", self.manifest.name));
            let agent = add.get_one::<String>("agent").map(String::as_str);
            return Some(match crate::mcp::register(&self.manifest.name, &command, agent) {
                Ok(paths) => {
                    Execution::success(json!({"command": command, "paths": paths}).to_string())
                }
                Err(error) => Execution::failure(error.to_string(), error.exit_code),
            });
        }
        None
    }

    #[cfg(any(feature = "completions", feature = "skills", feature = "mcp"))]
    fn builtin_matches(
        &self,
        argv: &[OsString],
    ) -> std::result::Result<clap::ArgMatches, Execution> {
        command::augmented(
            self.command.clone(),
            self.config.as_ref().map(|config| config.flag.as_str()),
        )
        .and_then(|command| command.try_get_matches_from(argv).map_err(clap_error))
        .map_err(execution_error)
    }

    #[cfg(feature = "completions")]
    fn completions(&self, shell: Option<&str>) -> Execution {
        use clap_complete::{Shell, generate};
        let shell = match shell.and_then(|shell| shell.parse::<Shell>().ok()) {
            Some(shell) => shell,
            None => {
                return Execution::failure("expected bash, elvish, fish, powershell, or zsh", 2);
            }
        };
        let mut command = match command::augmented(
            self.command.clone(),
            self.config.as_ref().map(|config| config.flag.as_str()),
        ) {
            Ok(command) => command,
            Err(error) => return Execution::failure(error.to_string(), error.exit_code),
        };
        let mut output = Vec::new();
        generate(shell, &mut command, &self.manifest.name, &mut output);
        Execution::success(String::from_utf8_lossy(&output).into_owned())
    }

    #[cfg(feature = "mcp")]
    pub(crate) async fn call_tool(&self, name: &str, input: Value) -> Result<Value> {
        let command = self
            .manifest
            .tool(name)
            .ok_or_else(|| Error::new("INVALID_PARAMS", format!("unknown tool `{name}`")))?;
        let mut argv = command::argv_for_tool(&self.manifest.name, command, &input)?;
        argv.extend([OsString::from("--format"), OsString::from("json")]);
        let execution = self.execute_inner(argv, true).await;
        if !execution.stdout.is_empty() {
            serde_json::from_str(&execution.stdout).or_else(|_| Ok(Value::String(execution.stdout)))
        } else {
            Err(Error::new("TOOL_ERROR", execution.stderr).exit_code(execution.exit_code))
        }
    }

    fn apply_config(&self, mut argv: Vec<OsString>) -> Result<Vec<OsString>> {
        let Some(config) = &self.config else {
            return Ok(argv);
        };
        let args_owned =
            argv.iter().filter_map(|value| value.to_str().map(str::to_owned)).collect::<Vec<_>>();
        let args = args_owned.iter().map(String::as_str).collect::<Vec<_>>();
        let no_flag = format!("--no-{}", config.flag);
        if args.iter().any(|argument| *argument == no_flag) {
            return Ok(argv);
        }
        let flag = format!("--{}", config.flag);
        let explicit = flag_value(&args[1..], &flag).map(expand_home);
        let candidates = if let Some(explicit) = explicit {
            vec![explicit]
        } else if config.files.is_empty() {
            vec![std::path::PathBuf::from(format!("{}.json", self.manifest.name))]
        } else {
            config.files.iter().map(|path| expand_home(&path.to_string_lossy())).collect()
        };
        let path = candidates.into_iter().find(|path| path.exists());
        let Some(path) = path else {
            if args
                .iter()
                .any(|argument| *argument == flag || argument.starts_with(&format!("{flag}=")))
            {
                return Err(Error::new("CONFIG_NOT_FOUND", "configuration file was not found"));
            }
            return Ok(argv);
        };
        let value = serde_json::from_slice::<Value>(&std::fs::read(&path)?)?;
        let path_name = resolve_raw_path(&self.command, &args[1..]);
        let Some(command) = self.manifest.command(&path_name) else {
            return Ok(argv);
        };
        let mut node = &value;
        for segment in path_name.split_whitespace() {
            let Some(next) = node.get("commands").and_then(|commands| commands.get(segment)) else {
                return Ok(argv);
            };
            node = next;
        }
        let Some(defaults) = node.get("options").and_then(Value::as_object) else {
            return Ok(argv);
        };
        for option in &command.options {
            let flag = if let Some(long) = &option.long {
                format!("--{long}")
            } else if let Some(short) = option.short {
                format!("-{short}")
            } else {
                continue;
            };
            if option_present(&args[1..], option.long.as_deref(), option.short) {
                continue;
            }
            let Some(value) = defaults
                .get(&option.name)
                .or_else(|| option.long.as_deref().and_then(|long| defaults.get(long)))
            else {
                continue;
            };
            append_config_option(&mut argv, &flag, option, value);
        }
        Ok(argv)
    }
}

/// Extension trait that turns any derived command type into an [`App`].
pub trait Incur: Parser + CommandFactory + FromArgMatches + Send + Sized + 'static {
    /// Wraps this command type with Incur's agent-oriented runtime.
    fn incur<O, H, Fut>(handler: H) -> App<Self>
    where
        O: JsonSchema + Serialize + Send + 'static,
        H: Fn(Self, Context) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = eyre::Result<O>> + Send + 'static,
    {
        App::new(handler)
    }
}

impl<T> Incur for T where T: Parser + CommandFactory + FromArgMatches + Send + Sized + 'static {}

fn clap_error(error: clap::Error) -> Error {
    let code = match error.kind() {
        clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
            "CLAP_DISPLAY"
        }
        _ => "ARGUMENT_ERROR",
    };
    let exit_code = error.exit_code().clamp(0, u8::MAX.into()) as u8;
    Error::new(code, error.to_string()).exit_code(exit_code)
}

fn execution_error(error: Error) -> Execution {
    if error.exit_code == 0 {
        Execution::success(error.message)
    } else {
        Execution::failure(error.message, error.exit_code)
    }
}

fn format_duration(duration: std::time::Duration) -> String {
    if duration.as_millis() == 0 {
        format!("{}µs", duration.as_micros())
    } else {
        format!("{}ms", duration.as_millis())
    }
}

#[cfg(feature = "tokens")]
fn tokenize(value: &str) -> Vec<u32> {
    tiktoken_rs::cl100k_base_singleton().encode_ordinary(value)
}

#[cfg(feature = "tokens")]
fn slice_tokens(value: &str, start: usize, end: usize) -> String {
    let tokenizer = tiktoken_rs::cl100k_base_singleton();
    let tokens = tokenizer.encode_ordinary(value);
    tokenizer
        .decode(tokens[start.min(tokens.len())..end.min(tokens.len())].to_vec())
        .unwrap_or_default()
}

#[cfg(feature = "tokens")]
struct TokenPage {
    text: String,
    truncated: bool,
    next_offset: Option<usize>,
}

#[cfg(feature = "tokens")]
fn paginate_tokens(value: &str, offset: usize, limit: Option<usize>) -> TokenPage {
    let total = tokenize(value).len();
    let start = offset.min(total);
    let end = limit.map(|limit| start.saturating_add(limit).min(total)).unwrap_or(total);
    if start == 0 && end == total {
        return TokenPage { text: value.to_owned(), truncated: false, next_offset: None };
    }
    let text = format!(
        "{}\n[truncated: showing tokens {start}–{end} of {total}]",
        slice_tokens(value, start, end)
    );
    TokenPage { text, truncated: true, next_offset: (end < total).then_some(end) }
}

fn render_cta(name: &str, cta: &crate::CtaBlock) -> String {
    let mut output = String::new();
    output.push_str("\n\n");
    output.push_str(cta.description.as_deref().unwrap_or("Next steps:"));
    for command in &cta.commands {
        output.push_str("\n  ");
        output.push_str(name);
        output.push(' ');
        output.push_str(&command.command);
        for argument in &command.args {
            output.push(' ');
            output.push_str(&command::json_scalar(argument));
        }
        for (option, value) in &command.options {
            output.push_str(" --");
            output.push_str(option);
            if value != &Value::Bool(true) {
                output.push(' ');
                output.push_str(&command::json_scalar(value));
            }
        }
        if let Some(description) = &command.description {
            output.push_str("  # ");
            output.push_str(description);
        }
    }
    output
}

fn resolve_raw_path(command: &clap::Command, args: &[&str]) -> String {
    let mut path = Vec::new();
    let mut current = command;
    for argument in args {
        if argument.starts_with('-') {
            break;
        }
        let Some(next) = current.find_subcommand(argument) else {
            break;
        };
        path.push((*argument).to_owned());
        current = next;
    }
    path.join(" ")
}

fn raw_format(args: &[&str]) -> Result<OutputFormat> {
    if args.contains(&"--json") {
        return Ok(OutputFormat::Json);
    }
    let Some(value) = flag_value(args, "--format") else {
        if raw_format_explicit(args) {
            return Err(Error::new("ARGUMENT_ERROR", "--format requires a value").exit_code(2));
        }
        return Ok(OutputFormat::default());
    };
    OutputFormat::parse(value).ok_or_else(|| {
        Error::new("ARGUMENT_ERROR", format!("unsupported output format `{value}`")).exit_code(2)
    })
}

fn raw_format_explicit(args: &[&str]) -> bool {
    args.contains(&"--json")
        || args
            .iter()
            .any(|argument| *argument == "--format" || argument.strip_prefix("--format=").is_some())
}

fn flag_value<'a>(args: &'a [&str], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|argument| *argument == flag)
        .and_then(|index| args.get(index + 1).copied())
        .or_else(|| args.iter().find_map(|argument| argument.strip_prefix(&format!("{flag}="))))
}

fn expand_home(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return std::path::PathBuf::from(home).join(rest);
    }
    std::path::PathBuf::from(path)
}

fn option_present(args: &[&str], long: Option<&str>, short: Option<char>) -> bool {
    let long = long.map(|long| format!("--{long}"));
    args.iter().any(|argument| {
        long.as_ref()
            .is_some_and(|long| *argument == long || argument.starts_with(&format!("{long}=")))
            || short.is_some_and(|short| {
                argument
                    .strip_prefix('-')
                    .filter(|rest| !rest.starts_with('-'))
                    .is_some_and(|rest| rest.contains(short))
            })
    })
}

fn append_config_option(
    argv: &mut Vec<OsString>,
    flag: &str,
    option: &crate::manifest::InputInfo,
    value: &Value,
) {
    if option.boolean {
        if value.as_bool() == Some(!option.false_action) {
            argv.push(flag.into());
        }
    } else if option.count {
        if let Some(count) = value.as_u64().and_then(|count| usize::try_from(count).ok()) {
            argv.extend(std::iter::repeat_n(OsString::from(flag), count));
        }
    } else if let Value::Array(values) = value {
        if option.repeatable {
            for value in values {
                argv.push(flag.into());
                argv.push(command::json_scalar(value).into());
            }
        } else if !values.is_empty() {
            argv.push(flag.into());
            argv.extend(values.iter().map(command::json_scalar).map(OsString::from));
        }
    } else {
        argv.push(flag.into());
        argv.push(command::json_scalar(value).into());
    }
}
