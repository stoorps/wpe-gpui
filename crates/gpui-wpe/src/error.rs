use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Closed,
    Cancelled,
    Unsupported(&'static str),
    InvalidInput(String),
    PolicyDenied(String),
    Native(String),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => f.write_str("the webview is closed"),
            Self::Cancelled => f.write_str("the operation was cancelled"),
            Self::Unsupported(s) => write!(f, "unsupported capability: {s}"),
            Self::InvalidInput(s) => write!(f, "invalid input: {s}"),
            Self::PolicyDenied(s) => write!(f, "policy denied: {s}"),
            Self::Native(s) => write!(f, "native webview error: {s}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        Self::Native(format!("{e:#}"))
    }
}
pub type Result<T> = std::result::Result<T, Error>;
