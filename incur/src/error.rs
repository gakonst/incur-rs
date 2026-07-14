use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::CtaBlock;

/// The framework's structured internal result type.
pub(crate) type InternalResult<T> = std::result::Result<T, Error>;

/// A validation failure tied to one input field.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldError {
    /// Dot-separated path to the invalid field.
    pub path: String,
    /// Human-readable validation message.
    pub message: String,
}

/// A structured command or framework error.
#[derive(Clone, Debug, Serialize, thiserror::Error)]
#[error("{message}")]
#[serde(rename_all = "camelCase")]
pub struct Error {
    /// Stable, machine-readable error code.
    pub code: Cow<'static, str>,
    /// Human-readable error message.
    pub message: String,
    /// Whether retrying the operation may succeed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    /// Field-specific validation failures.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub field_errors: Vec<FieldError>,
    /// Suggested recovery commands.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cta: Option<Box<CtaBlock>>,
    /// Requested process exit code.
    #[serde(skip)]
    pub exit_code: u8,
}

impl Error {
    /// Creates a command error with exit code `1`.
    pub fn new(code: impl Into<Cow<'static, str>>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: None,
            field_errors: Vec::new(),
            cta: None,
            exit_code: 1,
        }
    }

    /// Creates a validation error with exit code `2`.
    pub fn validation(message: impl Into<String>, field_errors: Vec<FieldError>) -> Self {
        Self {
            code: Cow::Borrowed("VALIDATION_ERROR"),
            message: message.into(),
            retryable: Some(false),
            field_errors,
            cta: None,
            exit_code: 2,
        }
    }

    /// Marks whether the failed operation is safe to retry.
    #[must_use]
    pub const fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = Some(retryable);
        self
    }

    /// Sets suggested recovery commands.
    #[must_use]
    pub fn cta(mut self, cta: CtaBlock) -> Self {
        self.cta = Some(Box::new(cta));
        self
    }

    /// Sets the process exit code.
    #[must_use]
    pub const fn exit_code(mut self, exit_code: u8) -> Self {
        self.exit_code = exit_code;
        self
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::new("IO_ERROR", error.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::new("JSON_ERROR", error.to_string())
    }
}

#[cfg(feature = "yaml")]
impl From<serde_yaml::Error> for Error {
    fn from(error: serde_yaml::Error) -> Self {
        Self::new("YAML_ERROR", error.to_string())
    }
}

impl From<eyre::Report> for Error {
    fn from(report: eyre::Report) -> Self {
        match report.downcast::<Self>() {
            Ok(error) => error,
            Err(report) => Self::new("COMMAND_ERROR", format!("{report:#}")),
        }
    }
}
