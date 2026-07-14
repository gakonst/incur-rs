# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added

- Initial Clap-native Rust port of Incur.
- Structured output, schemas, filters, token pagination, CTAs, and middleware.
- Skills, completions, MCP stdio/HTTP, JSON config defaults, and HTTP command routing.
- OpenAPI discovery, structured/scoped LLM manifests, and per-tool safety metadata.
- Six progressively more advanced examples covering the full public API.

### Fixed

- Preserve valid structured envelopes during token pagination and place CTAs in `meta.cta`.
- Validate JSON-RPC requests, HTTP MCP methods, notifications, and unknown tool errors.
- Return a structured error when middleware continues an invocation more than once.
- Keep disabled integrations out of help and avoid hijacking user commands with built-in names.
- Never replace a user-owned skill directory while linking generated skills.

### Changed

- Split completions, tokenization, and YAML into default-on Cargo features for a lean core build.
- Advertise the current stable MCP protocol revision (`2025-11-25`).
- Pin current Node 24-compatible CI actions and run `cargo-deny` in a locally controlled job.
