use std::fmt;

/// Errors produced while reading original data or mod files.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error, String),
    Corrupt(String),
    NotFound(String),
    Config(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn corrupt(msg: impl Into<String>) -> Self {
        Error::Corrupt(msg.into())
    }
    pub fn io(e: std::io::Error, ctx: impl Into<String>) -> Self {
        Error::Io(e, ctx.into())
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Error::NotFound(msg.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e, ctx) => write!(f, "io error ({ctx}): {e}"),
            Error::Corrupt(s) => write!(f, "corrupt data: {s}"),
            Error::NotFound(s) => write!(f, "not found: {s}"),
            Error::Config(s) => write!(f, "config: {s}"),
        }
    }
}

impl std::error::Error for Error {}