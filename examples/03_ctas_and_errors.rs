//! Structured failures and call-to-actions that help agents recover or continue.

use std::process::ExitCode;

use incur::prelude::*;

#[derive(Debug, Incur)]
#[command(name = "items", version, about = "Manage example items")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Incur)]
enum Command {
    /// Create an item.
    Create { name: String },
    /// Get an item by ID.
    Get { id: u64 },
}

#[derive(Debug, IncurOutput)]
struct Item {
    id: u64,
    name: String,
}

#[incur::main]
async fn main() -> ExitCode {
    Cli::incur(run).serve().await
}

async fn run(cli: Cli, context: Context) -> Result<Item> {
    match cli.command {
        Command::Create { name } => {
            let item = Item { id: 42, name };
            context.suggest(
                CtaBlock::new([Cta::new("get")
                    .arg(item.id)
                    .description("Inspect the created item")])
                .description("Next:"),
            );
            Ok(item)
        }
        Command::Get { id: 42 } => Ok(Item { id: 42, name: "example".to_owned() }),
        Command::Get { id } => Err(Error::new("NOT_FOUND", format!("item {id} does not exist"))
            .retryable(false)
            .cta(CtaBlock::new([Cta::new("create").arg("example")]))
            .into()),
    }
}
