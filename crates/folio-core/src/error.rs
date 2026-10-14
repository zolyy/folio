use std::fmt;

/// Every failure the engine can report. Messages are written for the person
/// (or agent) who has to act on them: they say what was wrong and, where it
/// helps, what is allowed.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// The bytes at `offset` could not be understood.
    Parse {
        offset: usize,
        msg: String,
    },
    /// Valid PDF, but a feature Folio does not handle yet.
    Unsupported(String),
    /// A request that cannot be satisfied (bad page range, missing object, ...).
    Invalid(String),
    /// The password is missing or wrong.
    Password(String),
    /// The document's security does not allow this without the permissions password.
    Permission(String),
    /// A safety cap was hit (nesting depth, decoded size, object count).
    Limit(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn parse(offset: usize, msg: impl Into<String>) -> Self {
        Error::Parse {
            offset,
            msg: msg.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "i/o error: {e}"),
            Error::Parse { offset, msg } => write!(f, "parse error at byte {offset}: {msg}"),
            Error::Unsupported(m) => write!(f, "not supported yet: {m}"),
            Error::Invalid(m) => write!(f, "{m}"),
            Error::Password(m) => write!(f, "{m}"),
            Error::Permission(m) => write!(f, "{m}"),
            Error::Limit(m) => write!(f, "safety limit reached: {m}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
