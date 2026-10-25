use std::fmt;

/// Error a task can finish with. Maps to PHP exceptions at the language level.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// Task was cancelled via `Promise::cancel()` or by a combinator.
    Cancelled,
    /// `Promise::timeout()` deadline passed. PHP: `TimeoutException`.
    Timeout,
    /// User code threw. PHP: any `Throwable`.
    Exception(String),
    /// Rust panic inside a task, converted so it can't take down the scheduler.
    Panic(String),
    /// `Promise::any()` with every promise rejected. PHP: `AggregateException`.
    AllRejected(Vec<Error>),
    /// Socket / file error. PHP: `IOException`. Kept as kind + message so
    /// `Error` stays `Clone` (promises hand the same error to many waiters).
    Io(std::io::ErrorKind, String),
}

impl Error {
    pub fn exception(msg: impl Into<String>) -> Self {
        Error::Exception(msg.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Cancelled => write!(f, "task cancelled"),
            Error::Timeout => write!(f, "operation timed out"),
            Error::Exception(m) => write!(f, "{m}"),
            Error::Panic(m) => write!(f, "panic: {m}"),
            Error::AllRejected(errs) => write!(f, "all {} promises rejected", errs.len()),
            Error::Io(_, m) => write!(f, "io: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.kind(), e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
