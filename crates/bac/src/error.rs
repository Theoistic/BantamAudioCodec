//! The one error type of the crate.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// the data is not what the format says it must be
    Format(String),
    /// valid, but outside what this version handles
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Format(m) => write!(f, "{m}"),
            Error::Unsupported(m) => write!(f, "not supported: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub(crate) fn bad(m: impl Into<String>) -> Error {
    Error::Format(m.into())
}
