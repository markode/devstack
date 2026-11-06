use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum DevStackError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("unsupported platform: {0}")]
    UnsupportedPlatform(String),
    #[error("version not found: {0}")]
    VersionNotFound(String),
    #[error("archive error: {0}")]
    Archive(String),
}

pub type Result<T> = std::result::Result<T, DevStackError>;

impl Serialize for DevStackError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<&str> for DevStackError {
    fn from(value: &str) -> Self {
        Self::Message(value.to_string())
    }
}

impl From<String> for DevStackError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}
