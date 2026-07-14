//! Middleware, shared variables, config-file defaults, and an agent-only output policy.

use std::process::ExitCode;

use incur::prelude::*;

#[derive(Debug, Incur)]
#[command(name = "deploy", version, about = "Deploy services")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Incur)]
enum Command {
    /// Deploy a service.
    Run {
        service: String,
        #[arg(long, default_value = "staging")]
        environment: String,
    },
}

#[derive(Debug, IncurOutput)]
struct Output {
    request_id: String,
    url: String,
}

#[incur::main]
async fn main() -> ExitCode {
    Cli::incur(run)
        .config(Config::new().file("deploy.json").file("~/.config/deploy/config.json"))
        .middleware(|context: Context, next: Next| async move {
            context.set_var("requestId", "req_example");
            next.run(context).await
        })
        .output_policy(OutputPolicy::AgentOnly)
        .serve()
        .await
}

async fn run(cli: Cli, context: Context) -> Result<Output> {
    let Command::Run { service, environment } = cli.command;
    let request_id = context
        .var("requestId")
        .and_then(|value| match value {
            Value::String(value) => Some(value),
            _ => None,
        })
        .unwrap_or_else(|| "req_unknown".to_owned());
    Ok(Output { request_id, url: format!("https://{service}.{environment}.example.com") })
}
