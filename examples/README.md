# Examples

These examples build on each other. Run any CLI example with `--help`, `--llms`, `--schema`, or
`--mcp` to inspect the behavior Incur adds to the same typed command definition.

1. [`01_greet.rs`](01_greet.rs) — one command, two arguments, and a typed output.
2. [`02_subcommands.rs`](02_subcommands.rs) — subcommands, value enums, defaults, and output enums.
3. [`03_ctas_and_errors.rs`](03_ctas_and_errors.rs) — stable error codes and structured next steps.
4. [`04_middleware_and_config.rs`](04_middleware_and_config.rs) — middleware variables, JSON config
   defaults, and agent-only output.
5. [`05_http_and_mcp.rs`](05_http_and_mcp.rs) — invoke one command graph through HTTP and MCP.
6. [`06_tool_metadata.rs`](06_tool_metadata.rs) — nested commands, safety annotations, tool
   instructions, and per-command policy.

For example:

```sh
cargo run -p incur-examples --bin 01_greet -- Ada
cargo run -p incur-examples --bin 02_subcommands -- install tracing --kind development
cargo run -p incur-examples --bin 03_ctas_and_errors -- get 7 --format json
cargo run -p incur-examples --bin 04_middleware_and_config -- run api --json
cargo run -p incur-examples --bin 05_http_and_mcp
cargo run -p incur-examples --bin 06_tool_metadata -- --llms-full
```
