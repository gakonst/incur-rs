# incur-rs

A Rust port of [wevm/incur](https://github.com/wevm/incur) for CLIs that need to work equally well
for humans and coding agents.

Your application defines its CLI once with familiar Rust derives. From that command graph, `incur`
derives JSON Schema, LLM manifests, Skills, shell completions, HTTP routes, and MCP tools. Output is
typed, serializable, and described to agents by the same derive.

## Features

- `#[derive(Incur)]` for command inputs and `#[derive(IncurOutput)]` for typed outputs.
- Agent discovery through `skills add`, `mcp add`, `--mcp`, `--llms`, and `--llms-full`.
- JSON Schema for every command through `--schema` and MCP `tools/list`.
- Token-efficient TOON output by default, plus JSON, YAML, Markdown, and JSONL.
- Structured success/error envelopes, typed error codes, retryability, and call-to-actions.
- Output filtering and exact `cl100k_base` token counting/pagination.
- Onion-style global and per-command middleware with shared context variables.
- JSON configuration defaults with `argv > config > declared default` precedence.
- Static shell completions generated directly from the command graph.
- HTTP command serving and MCP-over-HTTP using dependency-light `http` request/response types.
- Per-command output policy, output schema, MCP naming, instructions, and behavior annotations.
- OpenAPI 3.1 generation from the same reflected command manifest.

## Install

```toml
[dependencies]
incur = "0.1"
```

That is the only dependency application crates need. The minimum supported Rust version is 1.88.

## Quick start

```rust
use std::process::ExitCode;

use incur::prelude::*;

#[derive(Debug, Incur)]
#[command(name = "greet", version, about = "A greeting CLI")]
struct Cli {
    /// Name to greet.
    name: String,

    /// Render the greeting in uppercase.
    #[arg(long, short)]
    loud: bool,
}

#[derive(Debug, IncurOutput)]
struct Output {
    message: String,
}

#[incur::main]
async fn main() -> ExitCode {
    Cli::incur(run).serve().await
}

async fn run(cli: Cli, _context: Context) -> Result<Output> {
    let message = format!("hello {}", cli.name);
    Ok(Output {
        message: if cli.loud { message.to_uppercase() } else { message },
    })
}
```

Handler `Result` is Eyre's result type, re-exported by the prelude together with `bail!`, `ensure!`,
and `WrapErr`. Returning an `incur::Error` through Eyre preserves its stable code, retryability,
field errors, and CTAs.

```console
$ greet Ada
message: hello Ada

$ greet Ada --format json
{
  "ok": true,
  "data": { "message": "hello Ada" },
  "meta": { "command": "", "duration": "23µs" }
}
```

`Context::agent` is `true` when stdout is not a terminal. Agent/pipe output includes the complete
envelope; human TTY output shows data only unless `--full-output` or a format is explicitly chosen.

## Subcommands

Use ordinary Rust enums for subcommands. The same enum reaches the handler, while Incur
recursively exposes each leaf as an MCP tool and skill command.

```rust
use incur::prelude::*;

#[derive(Debug, Incur)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Incur)]
enum Command {
    /// Show repository status.
    Status,
    /// Install a package.
    Install {
        package: Option<String>,
        #[arg(long, short = 'D')]
        save_dev: bool,
    },
}
```

Parsing, aliases, defaults, required values, environment variables, value enums, validation, and
help all come from the same `Incur` derive. Incur augments every command with:

```text
--filter-output <KEYS>  --format <FORMAT>      --full-output
--json                  --llms / --llms-full  --mcp
--schema                --token-count          --token-limit <N>
--token-offset <N>
```

and the `completions`, `skills`, and `mcp` integrations.

## Structured errors and CTAs

```rust
use incur::{Cta, CtaBlock, Error};

// From a handler:
context.suggest(
    CtaBlock::new([
        Cta::new("get").arg(42).description("View the new item"),
        Cta::new("list").option("state", "open"),
    ])
    .description("Suggested commands:"),
);

return Err(Error::new("AUTH_REQUIRED", "log in before deploying")
    .retryable(false)
    .cta(CtaBlock::new([Cta::new("auth login")]))
    .into());
```

CTAs are rendered as commands for humans and remain structured in CLI/MCP/HTTP envelopes.

## Middleware and variables

```rust
let app = Cli::incur(run)
    .middleware(|context: incur::Context, next: incur::Next| async move {
        context.set_var("requestId", "req_123");
        let started = std::time::Instant::now();
        let output = next.run(context).await?;
        eprintln!("took {:?}", started.elapsed());
        Ok(output)
    })
    .middleware_for("deploy", require_auth);
```

Middleware runs in registration order and unwinds onion-style. `Context::set_var` and
`Context::var` share JSON-compatible data through the chain.

## Command metadata

The handler's output type supplies the default output schema. Override it or MCP metadata for a
specific path when different enum variants return different shapes:

```rust
use incur::{OutputPolicy, ToolAnnotations, ToolConfig};

let app = Cli::incur(run)
    .output_schema_for::<DeployOutput>("deploy")
    .output_policy_for("internal sync", OutputPolicy::AgentOnly)
    .tool(
        "deploy",
        ToolConfig {
            annotations: Some(ToolAnnotations {
                destructive_hint: Some(true),
                idempotent_hint: Some(false),
                open_world_hint: Some(true),
                ..Default::default()
            }),
            instructions: Some("Confirm the target environment first.".into()),
            ..Default::default()
        },
    );
```

Destructive hints are also written into generated skill instructions.

## Agent discovery

```console
# Compact command index or full schemas
$ my-cli --llms
$ my-cli --llms-full
$ my-cli deploy --schema --json

# Install generated skills globally or in the current repository
$ my-cli skills add
$ my-cli skills add --project --depth 2

# Generate completion source
$ my-cli completions zsh

# Serve or register MCP
$ my-cli --mcp
$ my-cli mcp add --agent cursor
```

Skills use the canonical `~/.agents/skills` or `.agents/skills` directory and link detected agents
that require their own skill directory. MCP registration supports Codex, Claude Code, Cursor, and
Amp configurations. Explicit `--agent` targeting prevents unrelated configuration changes.

## Configuration files

```rust
use incur::Config;

let app = Cli::incur(run).config(
    Config::new()
        .file("my-cli.json")
        .file("~/.config/my-cli/config.json"),
);
```

The JSON hierarchy mirrors the CLI's subcommands:

```json
{
  "options": { "verbose": true },
  "commands": {
    "deploy": { "options": { "region": "us-west-2", "force": false } }
  }
}
```

Use `--config <path>` to choose a file and `--no-config` to disable loading. Only named options are
loaded; positional arguments are never taken from configuration.

## HTTP and MCP

With the default `http` feature, `App::handle_http` maps paths, query parameters, and JSON bodies to
the same handler:

```rust
let mut request = incur::HttpRequest::new(Vec::new());
*request.uri_mut() = "/users/42?verbose=true".parse()?;
let response = app.handle_http(request).await;
# Ok::<(), Box<dyn std::error::Error>>(())
```

| HTTP request | CLI equivalent |
| --- | --- |
| `GET /users?limit=5` | `my-cli users --limit 5` |
| `GET /users/42` | `my-cli users 42` |
| `POST /users` with `{"name":"Ada"}` | `my-cli users --name Ada` |
| `POST /mcp` | MCP JSON-RPC over HTTP |

HTTP always returns JSON envelopes. The types are `http::Request<Vec<u8>>` and
`http::Response<Vec<u8>>`, so adapters for Axum, Hyper, Actix, or Lambda do not require those
frameworks in `incur` itself. `app.manifest().openapi()` generates an OpenAPI 3.1 document for the
same routes, also served at `/openapi.json`, `/openapi.yml`, `/openapi.yaml`, and
`/.well-known/openapi.json`.

## Progressive examples

The [examples learning path](examples/README.md) starts with a one-command greeting
and builds through subcommands, structured errors and CTAs, middleware and config, HTTP/MCP, and
per-tool safety metadata. Every example is compiled by CI.

## Cargo features

| Cargo feature | Default | Capability |
| --- | --- | --- |
| `toon` | yes | TOON encoding; JSON fallback when disabled |
| `skills` | yes | skill generation and installation |
| `mcp` | yes | stdio/HTTP MCP and agent registration |
| `http` | yes | dependency-light HTTP command serving |

Feature combinations are checked independently in CI.

## Development

The repository follows Alloy's Rust project conventions: an explicit MSRV, workspace lints,
warnings-as-errors Clippy, nightly rustfmt and rustdoc, feature-powerset checks, cargo-deny, typos,
read-only GitHub Actions permissions, pinned third-party actions, and a cross-platform test matrix.

```sh
cargo test --workspace --all-features --all-targets
cargo +nightly fmt --all -- --check
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets --all-features
cargo +1.88 check --workspace --all-features
```

## License

Licensed under either Apache-2.0 or MIT, at your option.
