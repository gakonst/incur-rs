//! A multi-command CLI with value enums, defaults, and command-specific output variants.

use std::process::ExitCode;

use incur::prelude::*;

#[derive(Debug, Incur)]
#[command(name = "packages", version, about = "Manage project packages")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Incur)]
enum Command {
    /// Show package installation status.
    Status,
    /// Install a package.
    Install {
        /// Package name.
        package: String,
        /// Dependency section to update.
        #[arg(long, value_enum, default_value_t = Kind::Regular)]
        kind: Kind,
    },
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum Kind {
    #[default]
    Regular,
    Development,
    Optional,
}

#[derive(Debug, IncurOutput)]
enum Output {
    Status { clean: bool },
    Installed { package: String, section: String },
}

#[incur::main]
async fn main() -> ExitCode {
    Cli::incur(run).serve().await
}

async fn run(cli: Cli, _context: Context) -> Result<Output> {
    match cli.command {
        Command::Status => Ok(Output::Status { clean: true }),
        Command::Install { package, kind } => {
            Ok(Output::Installed { package, section: format!("{kind:?}").to_ascii_lowercase() })
        }
    }
}
