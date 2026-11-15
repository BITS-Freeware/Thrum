//! Thrum command line client library.

pub mod api;
pub mod chat;
pub mod files;
pub mod session;
pub mod store;
pub mod ws;

#[cfg(test)]
mod device_security;

use url::Url;

/// Errors from base URL handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseUrlError {
    /// The string did not parse as a URL.
    Parse(String),
    /// The URL did not use http or https.
    UnsupportedScheme(String),
}

impl std::fmt::Display for BaseUrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BaseUrlError::Parse(raw) => write!(f, "invalid server URL: {raw:?}"),
            BaseUrlError::UnsupportedScheme(scheme) => {
                write!(f, "unsupported scheme {scheme:?}: use http or https")
            }
        }
    }
}

impl std::error::Error for BaseUrlError {}

/// Normalize a server base URL.
///
/// The base URL is only the origin: scheme, host and optional port. Any
/// path, query or fragment is dropped, and scheme and host are lowercased,
/// so `HTTPS://Example.com:8080/chat` becomes `https://example.com:8080`.
pub fn normalize_base_url(raw: &str) -> Result<String, BaseUrlError> {
    let mut parsed = Url::parse(raw).map_err(|error| BaseUrlError::Parse(error.to_string()))?;
    let scheme = parsed.scheme().to_string();
    if scheme != "http" && scheme != "https" {
        return Err(BaseUrlError::UnsupportedScheme(scheme));
    }
    parsed.set_path("");
    parsed.set_query(None);
    parsed.set_fragment(None);
    let mut normalized = parsed.to_string();
    while normalized.ends_with('/') {
        normalized.pop();
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercases_and_strips_extras() {
        assert_eq!(
            normalize_base_url("HTTPS://Example.com:8080/chat/").expect("valid"),
            "https://example.com:8080"
        );
        assert_eq!(
            normalize_base_url("http://LOCALHOST/").expect("valid"),
            "http://localhost"
        );
    }

    #[test]
    fn keeps_nonstandard_ports() {
        assert_eq!(
            normalize_base_url("http://127.0.0.1:8080").expect("valid"),
            "http://127.0.0.1:8080"
        );
    }

    #[test]
    fn rejects_bad_schemes_and_garbage() {
        assert_eq!(
            normalize_base_url("ftp://example.com"),
            Err(BaseUrlError::UnsupportedScheme("ftp".to_string()))
        );
        assert!(normalize_base_url("not a url").is_err());
    }
}
