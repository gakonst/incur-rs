//! The smallest useful Incur CLI: one typed input and one typed output.

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
    Ok(Output { message: if cli.loud { message.to_uppercase() } else { message } })
}
