//! Thrum core: protocol types and client logic shared by the CLI and web clients.
//!
//! The core holds everything that must behave identically in every client:
//! validation of account names, MLS group handling, and message framing.

/// Errors returned by core operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A proposed account name is not valid.
    InvalidUserName(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::InvalidUserName(raw) => write!(f, "invalid user name: {raw:?}"),
        }
    }
}

impl std::error::Error for Error {}

/// Result alias for core operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Maximum length of an account name.
pub const USER_NAME_MAX_LEN: usize = 32;

/// A validated account name.
///
/// The server enforces uniqueness case-insensitively, so the canonical form
/// of a name is lowercase. Allowed characters are ASCII letters, digits and
/// underscores; the name is 1 to 32 characters and starts with a letter or
/// a digit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserName(String);

impl UserName {
    /// Validate and canonicalize an account name.
    pub fn new(raw: &str) -> Result<Self> {
        let bytes = raw.as_bytes();
        if bytes.is_empty() || bytes.len() > USER_NAME_MAX_LEN {
            return Err(Error::InvalidUserName(raw.to_string()));
        }
        let first = bytes[0];
        let ok_start = first.is_ascii_alphanumeric();
        if !ok_start || !bytes.iter().all(u8::is_ascii_alphanumeric_or_underscore) {
            return Err(Error::InvalidUserName(raw.to_string()));
        }
        Ok(Self(raw.to_ascii_lowercase()))
    }

    /// The canonical (lowercase) form of the name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

trait AsciiAlphanumericOrUnderscore {
    fn is_ascii_alphanumeric_or_underscore(&self) -> bool;
}

impl AsciiAlphanumericOrUnderscore for u8 {
    fn is_ascii_alphanumeric_or_underscore(&self) -> bool {
        self.is_ascii_alphanumeric() || *self == b'_'
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_and_canonicalizes_valid_names() {
        let name = UserName::new("Alice_01").expect("valid name");
        assert_eq!(name.as_str(), "alice_01");
    }

    #[test]
    fn rejects_empty_and_overlong_names() {
        assert!(UserName::new("").is_err());
        assert!(UserName::new(&"a".repeat(USER_NAME_MAX_LEN + 1)).is_err());
    }

    #[test]
    fn rejects_bad_characters_and_bad_start() {
        assert!(UserName::new("has space").is_err());
        assert!(UserName::new("no-dash").is_err());
        assert!(UserName::new("_underscore").is_err());
        assert!(UserName::new("ünicode").is_err());
    }

    #[test]
    fn max_length_name_is_accepted() {
        let name = UserName::new(&"a".repeat(USER_NAME_MAX_LEN)).expect("valid name");
        assert_eq!(name.as_str().len(), USER_NAME_MAX_LEN);
    }
}
