#[cfg(any(feature = "http", feature = "mcp"))]
use std::ffi::OsString;

use clap::{Arg, ArgAction, Command, builder::ValueParser};
use serde_json::{Map, Value};

use crate::{CommandInfo, Error, InternalResult as Result, OutputFormat};

pub(crate) const INCUR_FORMAT: &str = "__incur_format";
pub(crate) const INCUR_JSON: &str = "__incur_json";
pub(crate) const INCUR_FULL_OUTPUT: &str = "__incur_full_output";
pub(crate) const INCUR_FILTER_OUTPUT: &str = "__incur_filter_output";
pub(crate) const INCUR_LLMS: &str = "__incur_llms";
pub(crate) const INCUR_LLMS_FULL: &str = "__incur_llms_full";
pub(crate) const INCUR_MCP: &str = "__incur_mcp";
pub(crate) const INCUR_SCHEMA: &str = "__incur_schema";
pub(crate) const INCUR_TOKEN_COUNT: &str = "__incur_token_count";
pub(crate) const INCUR_TOKEN_LIMIT: &str = "__incur_token_limit";
pub(crate) const INCUR_TOKEN_OFFSET: &str = "__incur_token_offset";

#[derive(Clone, Debug, Default)]
pub(crate) struct GlobalOptions {
    pub(crate) filter_output: Option<String>,
    pub(crate) format: OutputFormat,
    pub(crate) format_explicit: bool,
    pub(crate) full_output: bool,
    pub(crate) token_count: bool,
    pub(crate) token_limit: Option<usize>,
    pub(crate) token_offset: usize,
}

impl GlobalOptions {
    pub(crate) fn from_matches(matches: &clap::ArgMatches) -> Self {
        let format_value = find_value::<String>(matches, INCUR_FORMAT);
        let json = find_value::<bool>(matches, INCUR_JSON).copied().unwrap_or(false);
        Self {
            filter_output: find_value::<String>(matches, INCUR_FILTER_OUTPUT).cloned(),
            format: if json {
                OutputFormat::Json
            } else {
                format_value.map(String::as_str).and_then(OutputFormat::parse).unwrap_or_default()
            },
            format_explicit: json || format_value.is_some(),
            full_output: find_value::<bool>(matches, INCUR_FULL_OUTPUT).copied().unwrap_or(false),
            token_count: find_value::<bool>(matches, INCUR_TOKEN_COUNT).copied().unwrap_or(false),
            token_limit: find_value::<usize>(matches, INCUR_TOKEN_LIMIT).copied(),
            token_offset: find_value::<usize>(matches, INCUR_TOKEN_OFFSET).copied().unwrap_or(0),
        }
    }
}

fn find_value<'a, T: Clone + Send + Sync + 'static>(
    matches: &'a clap::ArgMatches,
    id: &str,
) -> Option<&'a T> {
    matches
        .try_get_one::<T>(id)
        .ok()
        .flatten()
        .or_else(|| matches.subcommand().and_then(|(_, nested)| find_value(nested, id)))
}

pub(crate) fn augmented(mut command: Command, config_flag: Option<&str>) -> Result<Command> {
    check_conflicts(&command, config_flag)?;
    command = command
        .disable_help_subcommand(true)
        .arg(
            Arg::new(INCUR_FILTER_OUTPUT)
                .long("filter-output")
                .global(true)
                .value_name("KEYS")
                .help("Filter output by key paths (for example foo,bar.baz,a[0,3])"),
        )
        .arg(
            Arg::new(INCUR_FORMAT)
                .long("format")
                .global(true)
                .value_name("FORMAT")
                .value_parser(["toon", "json", "yaml", "md", "jsonl"])
                .help("Output format"),
        )
        .arg(
            Arg::new(INCUR_JSON)
                .long("json")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Shortcut for --format json"),
        )
        .arg(
            Arg::new(INCUR_FULL_OUTPUT)
                .long("full-output")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Show the complete output envelope"),
        )
        .arg(
            Arg::new(INCUR_LLMS)
                .long("llms")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Print a compact LLM-readable command manifest"),
        )
        .arg(
            Arg::new(INCUR_LLMS_FULL)
                .long("llms-full")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Print the complete LLM-readable command manifest"),
        )
        .arg(
            Arg::new(INCUR_MCP)
                .long("mcp")
                .action(ArgAction::SetTrue)
                .help("Start an MCP server over stdio"),
        )
        .arg(
            Arg::new(INCUR_SCHEMA)
                .long("schema")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Show JSON Schema for the resolved command"),
        )
        .arg(
            Arg::new(INCUR_TOKEN_COUNT)
                .long("token-count")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Print output token count instead of output"),
        )
        .arg(
            Arg::new(INCUR_TOKEN_LIMIT)
                .long("token-limit")
                .global(true)
                .value_name("N")
                .value_parser(clap::value_parser!(usize))
                .help("Limit output to N tokens"),
        )
        .arg(
            Arg::new(INCUR_TOKEN_OFFSET)
                .long("token-offset")
                .global(true)
                .value_name("N")
                .value_parser(clap::value_parser!(usize))
                .help("Skip the first N output tokens"),
        );
    if let Some(flag) = config_flag {
        command = command
            .arg(
                Arg::new("__incur_config")
                    .long(flag.to_owned())
                    .global(true)
                    .value_name("PATH")
                    .help("Load option defaults from a JSON file"),
            )
            .arg(
                Arg::new("__incur_no_config")
                    .long(format!("no-{flag}"))
                    .global(true)
                    .action(ArgAction::SetTrue)
                    .help("Disable configuration-file loading"),
            );
    }

    if command.find_subcommand("completions").is_none() {
        command = command.subcommand(
            Command::new("completions").about("Generate a shell completion script").arg(
                Arg::new("shell").required(true).value_parser([
                    "bash",
                    "elvish",
                    "fish",
                    "powershell",
                    "zsh",
                ]),
            ),
        );
    }
    if command.find_subcommand("skills").is_none() {
        command = command.subcommand(
            Command::new("skills")
                .about("Sync skill files to coding agents")
                .subcommand(
                    Command::new("add")
                        .about("Generate and install skill files")
                        .arg(
                            Arg::new("project")
                                .long("project")
                                .action(ArgAction::SetTrue)
                                .help("Install in the current project instead of globally"),
                        )
                        .arg(
                            Arg::new("depth")
                                .long("depth")
                                .default_value("1")
                                .value_parser(clap::value_parser!(usize)),
                        ),
                )
                .subcommand(Command::new("list").about("List generated skills")),
        );
    }
    if command.find_subcommand("mcp").is_none() {
        command = command.subcommand(
            Command::new("mcp").about("Register as an MCP server").subcommand(
                Command::new("add")
                    .about("Register this executable as an MCP server")
                    .arg(Arg::new("command").short('c').long("command").value_name("COMMAND"))
                    .arg(Arg::new("agent").long("agent").value_name("AGENT")),
            ),
        );
    }
    Ok(command)
}

fn check_conflicts(command: &Command, config_flag: Option<&str>) -> Result<()> {
    const RESERVED: &[&str] = &[
        "filter-output",
        "format",
        "full-output",
        "json",
        "llms",
        "llms-full",
        "mcp",
        "schema",
        "token-count",
        "token-limit",
        "token-offset",
    ];
    fn visit(command: &Command, config_flag: Option<&str>) -> Option<String> {
        for argument in command.get_arguments() {
            if let Some(long) = argument.get_long()
                && (RESERVED.contains(&long)
                    || config_flag.is_some_and(|flag| long == flag || long == format!("no-{flag}")))
            {
                return Some(long.to_owned());
            }
        }
        command.get_subcommands().find_map(|command| visit(command, config_flag))
    }
    if let Some(name) = visit(command, config_flag) {
        return Err(Error::new(
            "GLOBAL_OPTION_CONFLICT",
            format!("--{name} is reserved by incur and cannot be declared by the application"),
        ));
    }
    Ok(())
}

pub(crate) fn command_path(matches: &clap::ArgMatches) -> Vec<String> {
    let mut path = Vec::new();
    let mut current = matches;
    while let Some((name, nested)) = current.subcommand() {
        path.push(name.to_owned());
        current = nested;
    }
    path
}

pub(crate) fn collect(command: &Command, output_schema: &Value) -> Vec<CommandInfo> {
    let mut commands = Vec::new();
    collect_inner(command, &mut Vec::new(), output_schema, &mut commands);
    commands
}

fn collect_inner(
    command: &Command,
    prefix: &mut Vec<String>,
    output_schema: &Value,
    commands: &mut Vec<CommandInfo>,
) {
    let children = command.get_subcommands().collect::<Vec<_>>();
    let user_arguments = command.get_arguments().filter(|argument| !is_builtin(argument)).count();
    if children.is_empty() || user_arguments > 0 || prefix.is_empty() && children.is_empty() {
        commands.push(info(command, prefix, output_schema.clone()));
    }
    for child in children {
        prefix.push(child.get_name().to_owned());
        collect_inner(child, prefix, output_schema, commands);
        prefix.pop();
    }
}

fn info(command: &Command, path: &[String], output_schema: Value) -> CommandInfo {
    let mut properties = Map::new();
    let mut required = Vec::new();
    let mut positionals = Vec::new();
    let mut options = Vec::new();
    for argument in command.get_arguments().filter(|argument| !is_builtin(argument)) {
        let input = input_info(argument);
        properties.insert(input.name.clone(), input.schema.clone());
        if input.required {
            required.push(Value::String(input.name.clone()));
        }
        if input.positional {
            positionals.push(input);
        } else {
            options.push(input);
        }
    }
    positionals.sort_by_key(|input| input.index);
    let mut schema = Map::from_iter([
        ("type".to_owned(), Value::String("object".to_owned())),
        ("properties".to_owned(), Value::Object(properties)),
        ("additionalProperties".to_owned(), Value::Bool(false)),
    ]);
    if !required.is_empty() {
        schema.insert("required".to_owned(), Value::Array(required));
    }
    CommandInfo {
        name: path.join(" "),
        description: command
            .get_about()
            .or_else(|| command.get_long_about())
            .map(ToString::to_string),
        input_schema: Value::Object(schema),
        output_schema,
        tool_name_override: None,
        annotations: None,
        instructions: None,
        mcp: true,
        positionals,
        options,
    }
}

fn is_builtin(argument: &Arg) -> bool {
    argument.get_id().as_str().starts_with("__incur_")
        || matches!(argument.get_id().as_str(), "help" | "version")
}

fn input_info(argument: &Arg) -> crate::manifest::InputInfo {
    let schema = argument_schema(argument);
    crate::manifest::InputInfo {
        name: argument.get_id().as_str().to_owned(),
        long: argument.get_long().map(str::to_owned),
        short: argument.get_short(),
        description: argument
            .get_help()
            .or_else(|| argument.get_long_help())
            .map(ToString::to_string),
        required: argument.is_required_set(),
        positional: argument.get_index().is_some(),
        index: argument.get_index().unwrap_or(usize::MAX),
        array: is_array(argument),
        repeatable: matches!(argument.get_action(), ArgAction::Append),
        boolean: matches!(argument.get_action(), ArgAction::SetTrue | ArgAction::SetFalse),
        count: matches!(argument.get_action(), ArgAction::Count),
        false_action: matches!(argument.get_action(), ArgAction::SetFalse),
        schema,
    }
}

fn argument_schema(argument: &Arg) -> Value {
    let mut schema = Map::new();
    let boolean = matches!(argument.get_action(), ArgAction::SetTrue | ArgAction::SetFalse);
    if boolean {
        schema.insert("type".to_owned(), Value::String("boolean".to_owned()));
    } else if let Some(values) = argument.get_value_parser().possible_values() {
        let values = values
            .filter(|value| !value.is_hide_set())
            .map(|value| Value::String(value.get_name().to_owned()))
            .collect::<Vec<_>>();
        if !values.is_empty() {
            schema.insert("type".to_owned(), Value::String("string".to_owned()));
            schema.insert("enum".to_owned(), Value::Array(values));
        }
    }
    if !schema.contains_key("type") {
        let kind = value_kind(argument);
        schema.insert("type".to_owned(), Value::String(kind.to_owned()));
    }
    if let Some(description) = argument.get_help().or_else(|| argument.get_long_help()) {
        schema.insert("description".to_owned(), Value::String(description.to_string()));
    }
    let defaults = argument.get_default_values();
    if is_array(argument) {
        let item = Value::Object(schema);
        let mut outer = Map::from_iter([
            ("type".to_owned(), Value::String("array".to_owned())),
            ("items".to_owned(), item),
        ]);
        if !defaults.is_empty() {
            outer.insert(
                "default".to_owned(),
                Value::Array(
                    defaults
                        .iter()
                        .map(|value| parse_scalar(&value.to_string_lossy(), value_kind(argument)))
                        .collect(),
                ),
            );
        }
        return Value::Object(outer);
    }
    if let [default] = defaults {
        schema.insert(
            "default".to_owned(),
            parse_scalar(&default.to_string_lossy(), value_kind(argument)),
        );
    }
    Value::Object(schema)
}

fn value_kind(argument: &Arg) -> &'static str {
    if matches!(argument.get_action(), ArgAction::SetTrue | ArgAction::SetFalse) {
        return "boolean";
    }
    if matches!(argument.get_action(), ArgAction::Count) {
        return "integer";
    }
    let parser = argument.get_value_parser();
    if numeric_parser(parser, true) {
        "integer"
    } else if numeric_parser(parser, false) {
        "number"
    } else {
        "string"
    }
}

fn numeric_parser(parser: &ValueParser, integer: bool) -> bool {
    macro_rules! is_type {
        ($type:ty) => {{
            let candidate: ValueParser = clap::value_parser!($type).into();
            parser.type_id() == candidate.type_id()
        }};
    }
    if integer {
        is_type!(i8)
            || is_type!(i16)
            || is_type!(i32)
            || is_type!(i64)
            || is_type!(i128)
            || is_type!(isize)
            || is_type!(u8)
            || is_type!(u16)
            || is_type!(u32)
            || is_type!(u64)
            || is_type!(u128)
            || is_type!(usize)
    } else {
        is_type!(f32) || is_type!(f64)
    }
}

fn is_array(argument: &Arg) -> bool {
    matches!(argument.get_action(), ArgAction::Append)
        || argument.get_num_args().is_some_and(|range| range.max_values() > 1)
}

fn parse_scalar(value: &str, kind: &str) -> Value {
    match kind {
        "boolean" => value.parse().map(Value::Bool).unwrap_or_else(|_| Value::String(value.into())),
        "integer" => value
            .parse::<i64>()
            .map(|value| Value::Number(value.into()))
            .unwrap_or_else(|_| Value::String(value.into())),
        "number" => value
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(value.into())),
        _ => Value::String(value.into()),
    }
}

#[cfg(any(feature = "http", feature = "mcp"))]
pub(crate) fn argv_for_tool(
    root: &str,
    command: &CommandInfo,
    input: &Value,
) -> Result<Vec<OsString>> {
    let object = input.as_object().ok_or_else(|| {
        Error::validation(
            "MCP tool arguments must be a JSON object",
            vec![crate::FieldError { path: String::new(), message: "expected object".to_owned() }],
        )
    })?;
    let unknown = object
        .keys()
        .filter(|name| {
            !command.positionals.iter().any(|input| &input.name == *name)
                && !command.options.iter().any(|input| &input.name == *name)
        })
        .map(|name| crate::FieldError {
            path: name.clone(),
            message: "unknown input field".to_owned(),
        })
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(Error::validation("command input contains unknown fields", unknown));
    }
    let mut argv = vec![OsString::from(root)];
    argv.extend(command.name.split_whitespace().map(OsString::from));
    for positional in &command.positionals {
        if let Some(value) = object.get(&positional.name) {
            push_positional(&mut argv, value);
        }
    }
    for option in &command.options {
        let Some(value) = object.get(&option.name) else {
            continue;
        };
        let flag = option_flag(option);
        if option.boolean {
            let enabled = value
                .as_bool()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
                .ok_or_else(|| input_type_error(option, "expected boolean"))?;
            if enabled != option.false_action {
                argv.push(flag.into());
            }
        } else if option.count {
            let count = value
                .as_u64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
                .and_then(|count| usize::try_from(count).ok())
                .ok_or_else(|| input_type_error(option, "expected non-negative integer"))?;
            argv.extend(std::iter::repeat_n(OsString::from(flag), count));
        } else if let Value::Array(values) = value {
            if option.repeatable {
                for value in values {
                    argv.push(flag.clone().into());
                    argv.push(json_scalar(value).into());
                }
            } else if !values.is_empty() {
                argv.push(flag.into());
                argv.extend(values.iter().map(json_scalar).map(OsString::from));
            }
        } else {
            argv.push(flag.into());
            argv.push(json_scalar(value).into());
        }
    }
    Ok(argv)
}

#[cfg(any(feature = "http", feature = "mcp"))]
fn input_type_error(option: &crate::manifest::InputInfo, message: &str) -> Error {
    Error::validation(
        "command input has an invalid field",
        vec![crate::FieldError { path: option.name.clone(), message: message.to_owned() }],
    )
}

#[cfg(any(feature = "http", feature = "mcp"))]
fn option_flag(option: &crate::manifest::InputInfo) -> String {
    if let Some(long) = &option.long {
        format!("--{long}")
    } else if let Some(short) = option.short {
        format!("-{short}")
    } else {
        format!("--{}", option.name)
    }
}

#[cfg(any(feature = "http", feature = "mcp"))]
fn push_positional(argv: &mut Vec<OsString>, value: &Value) {
    if let Value::Array(values) = value {
        argv.extend(values.iter().map(json_scalar).map(OsString::from));
    } else {
        argv.push(json_scalar(value).into());
    }
}

pub(crate) fn json_scalar(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}
