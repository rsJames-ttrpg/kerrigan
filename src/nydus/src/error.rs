use std::fmt;

#[derive(Debug)]
pub enum Error {
    Request(reqwest::Error),
    Api { status: u16, body: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(e) => write!(f, "HTTP request failed: {e}"),
            Self::Api { status, body } => write!(f, "Overseer API error ({status}): {body}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Request(e) => Some(e),
            Self::Api { .. } => None,
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Self::Request(e)
    }
}
