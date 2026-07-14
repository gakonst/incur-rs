# Contributing to incur-rs

Contributions are welcome. For a substantial API or behavior change, open an issue first so the
Clap-facing design and upstream Incur compatibility can be agreed on before implementation.

The project follows the [Rust Code of Conduct](https://www.rust-lang.org/policies/code-of-conduct)
and uses Conventional Commits for commit messages.

## Development checks

Install Rust 1.88, stable, and nightly, then run:

```sh
cargo test --workspace --all-features --all-targets
cargo +1.88 check --workspace --all-features --all-targets
cargo +nightly fmt --all -- --check
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-features --all-targets
cargo deny check
typos
```

Behavior changes should include focused unit or end-to-end tests. Public API changes should also
update rustdoc, the README, and the closest example in `examples`.

Keep commits narrowly scoped and do not include generated `target` or `Cargo.lock` files.
