//! A typed CLI framework for agents and humans.
//!
//! Define inputs with [`Incur`], outputs with [`IncurOutput`], and `incur` adds structured output,
//! JSON Schema, LLM manifests, token controls, Skills, and MCP transports.
//!
//! # Example
//!
//! ```no_run
//! use incur::prelude::*;
//!
//! #[derive(Debug, Incur)]
//! #[command(name = "greet", version, about = "A greeting CLI")]
//! struct Cli {
//!     /// Name to greet.
//!     name: String,
//! }
//!
//! # async fn example() {
//! let status = Cli::incur(|cli, _context| async move {
//!     Ok(json!({ "message": format!("hello {}", cli.name) }))
//! })
//! .serve()
//! .await;
//! # let _ = status;
//! # }
//! ```

#![doc(html_logo_url = "https://raw.githubusercontent.com/wevm/incur/main/.github/logo-light.svg")]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod app;
mod command;
mod context;
mod error;
mod filter;
mod format;
#[cfg(feature = "http")]
mod http;
mod manifest;
#[cfg(feature = "mcp")]
mod mcp;
mod middleware;
#[cfg(feature = "skills")]
mod skills;

pub use app::{App, Execution, Incur};
#[doc(hidden)]
pub use clap;
#[doc(hidden)]
pub use clap::{Args, Parser, Subcommand, ValueEnum};
/// Defines an Incur CLI from a struct or enum.
pub use clap_derive::Parser as Incur;
pub use context::{Config, Context, Cta, CtaBlock, OutputFormat, OutputPolicy};
pub use error::{Error, FieldError};
pub use eyre::{self, Result};
pub use incur_macros::{IncurOutput, main};
pub use manifest::{CommandInfo, Manifest, ToolAnnotations, ToolConfig};
pub use middleware::{Middleware, Next};
#[doc(hidden)]
pub use schemars;
#[doc(hidden)]
pub use schemars::JsonSchema as AgentOutput;
#[doc(hidden)]
pub use schemars::{JsonSchema, Schema, schema_for};
#[doc(hidden)]
pub use serde;
#[doc(hidden)]
pub use serde::{Deserialize, Serialize};
pub use serde_json::{Value, json};
#[doc(hidden)]
pub use tokio;

pub(crate) use error::InternalResult;

/// Convenient imports for defining an Incur CLI without naming implementation dependencies.
pub mod prelude {
    pub use crate::{
        App, Config, Context, Cta, CtaBlock, Error, Execution, FieldError, Incur, IncurOutput,
        Middleware, Next, OutputFormat, OutputPolicy, Result, ToolAnnotations, ToolConfig, Value,
        ValueEnum, json,
    };
    pub use eyre::{WrapErr, bail, ensure, eyre};

    // Clap's derive expansion resolves this crate name at its call site. Keeping the alias in the
    // prelude makes `#[derive(Incur)]` work with only an `incur` dependency.
    #[doc(hidden)]
    pub use crate::clap;
}

#[cfg(feature = "http")]
pub use crate::http::{HttpBody, HttpRequest, HttpResponse};
