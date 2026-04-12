//! Error types for filling-pilot-edge

#[derive(Debug, Clone)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub enum ErrorKind {
    Config,
    S7,
    Io,
    Codec,
    Other,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Error {}

impl Error {
    pub fn config(msg: impl Into<String>) -> Self {
        Self { kind: ErrorKind::Config, message: msg.into() }
    }
    pub fn s7(msg: impl Into<String>) -> Self {
        Self { kind: ErrorKind::S7, message: msg.into() }
    }
    pub fn io(msg: impl Into<String>) -> Self {
        Self { kind: ErrorKind::Io, message: msg.into() }
    }
    pub fn codec(msg: impl Into<String>) -> Self {
        Self { kind: ErrorKind::Codec, message: msg.into() }
    }
    pub fn other(msg: impl Into<String>) -> Self {
        Self { kind: ErrorKind::Other, message: msg.into() }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self { Self::io(e.to_string()) }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self { Self::config(format!("JSON: {}", e)) }
}
impl From<String> for Error {
    fn from(s: String) -> Self { Self::other(s) }
}
impl From<&str> for Error {
    fn from(s: &str) -> Self { Self::other(s.to_string()) }
}
impl From<s7_connector::error::S7Error> for Error {
    fn from(e: s7_connector::error::S7Error) -> Self { Self::s7(e.to_string()) }
}

pub type Result<T> = std::result::Result<T, Error>;
