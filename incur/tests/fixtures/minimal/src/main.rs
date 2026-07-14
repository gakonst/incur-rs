use std::process::ExitCode;

use incur::prelude::*;

#[derive(Debug, Incur)]
#[command(name = "greet")]
struct Cli {
    name: String,
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
    Ok(Output { message: format!("hello {}", cli.name) })
}
