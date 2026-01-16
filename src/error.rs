use std::{fmt, io, path::PathBuf};

/// Unified error type for btwd startup failures
#[derive(Debug)]
pub enum BtwError {
    /// Required file is missing
    MissingFile { path: PathBuf, kind: &'static str },
    /// I/O error while reading a file
    ReadError { path: PathBuf, source: io::Error },
    /// Parse error for config or commands
    ParseError { path: PathBuf, kind: &'static str, message: String },
    /// .env loading error
    EnvLoadError { path: PathBuf, source: dotenvy::Error },
    /// XDG path resolution errors
    XdgError { message: String },

    /// Porcupine initialization failed (often due to incorrect arguments or missing files)
    PorcupineInitFailed { status: i32, messages: Vec<String> },
}

impl fmt::Display for BtwError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BtwError::MissingFile { path, kind } => {
                write!(
                    f,
                    "Missing required {} file: {}",
                    kind,
                    path.display()
                )
            }
            BtwError::ReadError { path, source } => {
                write!(
                    f,
                    "Failed to read file {}: {}",
                    path.display(),
                    source
                )
            }
            BtwError::ParseError { path, kind, message } => {
                write!(
                    f,
                    "Failed to parse {} file {}: {}",
                    kind,
                    path.display(),
                    message
                )
            }
            BtwError::EnvLoadError { path, source } => {
                write!(
                    f,
                    "Failed to load environment from {}: {}",
                    path.display(),
                    source
                )
            }
            BtwError::XdgError { message } => {
                write!(f, "XDG path resolution error: {}", message)
            }

            BtwError::PorcupineInitFailed { status, messages } => {
                if messages.is_empty() {
                    write!(f, "Porcupine init failed (status={})", status)
                } else {
                    write!(f, "Porcupine init failed (status={}): {}", status, messages.join(" | "))
                }
            }

        }
    }
}

impl std::error::Error for BtwError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BtwError::ReadError { source, .. } => Some(source),
            BtwError::EnvLoadError { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Convenient result alias for btwd
pub type Result<T> = std::result::Result<T, BtwError>;
