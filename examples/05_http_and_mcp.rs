//! Serve one typed command graph through both HTTP routing and MCP-over-HTTP.

use incur::prelude::*;

#[derive(Debug, Incur)]
#[command(name = "directory", version, about = "Query a user directory")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Incur)]
enum Command {
    /// Return one user or a page of users.
    Users {
        id: Option<u64>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

#[derive(Debug, IncurOutput)]
struct Output {
    users: Vec<User>,
}

#[derive(Debug, IncurOutput)]
struct User {
    id: u64,
    name: String,
}

#[incur::main]
async fn main() -> Result<()> {
    let app = Cli::incur(run).mcp_instructions("Use directory tools for user lookup requests.");

    // GET /users/42 is the HTTP equivalent of `directory users 42`.
    let mut request = incur::HttpRequest::new(Vec::new());
    *request.uri_mut() = "/users/42".parse()?;
    print_response(app.clone(), request).await;

    // The same app exposes its command graph as MCP tools at /mcp.
    let mut request = incur::HttpRequest::new(
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list"
        })
        .to_string()
        .into_bytes(),
    );
    *request.uri_mut() = "/mcp".parse()?;
    print_response(app, request).await;
    Ok(())
}

async fn print_response(app: App<Cli>, request: incur::HttpRequest) {
    let response = app.handle_http(request).await;
    println!("{}", String::from_utf8_lossy(response.body()));
}

async fn run(cli: Cli, _context: Context) -> Result<Output> {
    let Command::Users { id, limit } = cli.command;
    let users = if let Some(id) = id {
        vec![User { id, name: format!("user-{id}") }]
    } else {
        (1..=limit.min(3)).map(|id| User { id: id as u64, name: format!("user-{id}") }).collect()
    };
    Ok(Output { users })
}
