//! A nested production-style CLI with per-command schemas, policy, and MCP safety annotations.

use std::process::ExitCode;

use incur::prelude::*;

#[derive(Debug, Incur)]
#[command(name = "cloud", version, about = "Operate cloud projects")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Incur)]
enum Command {
    /// Project operations.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
}

#[derive(Debug, Incur)]
enum ProjectCommand {
    /// Inspect a project without changing it.
    Get { project: String },
    /// Permanently delete a project.
    Delete {
        project: String,
        #[arg(long)]
        confirm: bool,
    },
}

#[derive(Debug, IncurOutput)]
struct Output {
    project: String,
    state: String,
}

#[incur::main]
async fn main() -> ExitCode {
    Cli::incur(run)
        .output_policy_for("project delete", OutputPolicy::AgentOnly)
        .tool(
            "project get",
            ToolConfig {
                annotations: Some(ToolAnnotations {
                    read_only_hint: Some(true),
                    idempotent_hint: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .tool(
            "project delete",
            ToolConfig {
                annotations: Some(ToolAnnotations {
                    destructive_hint: Some(true),
                    idempotent_hint: Some(true),
                    open_world_hint: Some(true),
                    ..Default::default()
                }),
                instructions: Some("Require explicit user confirmation before calling.".into()),
                ..Default::default()
            },
        )
        .serve()
        .await
}

async fn run(cli: Cli, context: Context) -> Result<Output> {
    match cli.command {
        Command::Project { command: ProjectCommand::Get { project } } => {
            Ok(Output { project, state: "active".to_owned() })
        }
        Command::Project { command: ProjectCommand::Delete { project, confirm } } => {
            if !confirm {
                return Err(context
                    .error("CONFIRMATION_REQUIRED", "pass --confirm after user approval")
                    .retryable(false)
                    .into());
            }
            Ok(Output { project, state: "deleted".to_owned() })
        }
    }
}
