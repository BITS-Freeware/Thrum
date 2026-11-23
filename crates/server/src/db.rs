//! Database connections and migrations.
//!
//! Two backends are supported, selected by the connection string:
//! SQLite (a filesystem path, optionally with a `sqlite://` prefix) and
//! PostgreSQL (a `postgres://` URL). Both run the same logical schema;
//! each backend has its own migration set because column types differ.
//!
//! Runtime SQL uses `$1`-style placeholders, which both backends accept
//! natively, and binding is positional, so every query runs unchanged on
//! either backend.

use sqlx::AnyPool;

/// Which backend a pool is connected to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Embedded SQLite; catch-up ordering uses the rowid column.
    Sqlite,
    /// PostgreSQL; catch-up ordering uses identity columns (seq).
    Postgres,
}

/// Which migration directory to apply for a backend.
fn migrations(backend: Backend) -> sqlx::migrate::Migrator {
    match backend {
        // Compile-time embedded; same versions and order as SQLite's.
        Backend::Postgres => sqlx::migrate!("./migrations-pg"),
        Backend::Sqlite => sqlx::migrate!("./migrations"),
    }
}

/// Create a missing SQLite database file readable by its owner only. SQLite
/// gives the `-wal` and `-shm` files it creates the database file's mode,
/// so they stay private too. An existing file keeps the mode it has.
fn create_private_database(path: &std::path::Path) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

/// Which backend `url` selects: a `postgres://` or `postgresql://` URL (in
/// any case) is PostgreSQL, anything else a SQLite file.
pub fn backend_of(url: &str) -> Backend {
    let scheme = url
        .split_once("://")
        .map(|(scheme, _)| scheme.to_ascii_lowercase());
    if matches!(scheme.as_deref(), Some("postgres" | "postgresql")) {
        Backend::Postgres
    } else {
        Backend::Sqlite
    }
}

/// `url` with its password, in the userinfo or a `password=` query
/// parameter, replaced by `***`, so it can be logged.
pub fn redact_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let (authority, tail) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    let authority = match authority.rsplit_once('@') {
        Some((userinfo, host)) => match userinfo.split_once(':') {
            Some((user, _)) => format!("{user}:***@{host}"),
            None => format!("{userinfo}@{host}"),
        },
        None => authority.to_string(),
    };
    let tail = match tail.split_once('?') {
        Some((path, query)) => {
            let query: Vec<String> = query
                .split('&')
                .map(|pair| match pair.split_once('=') {
                    Some((key, _)) if key.eq_ignore_ascii_case("password") => format!("{key}=***"),
                    _ => pair.to_string(),
                })
                .collect();
            format!("{path}?{}", query.join("&"))
        }
        None => tail.to_string(),
    };
    format!("{scheme}://{authority}{tail}")
}

/// `text` (an error message about `url`) with `url`'s password removed
/// wherever it appears, as well as the URL itself.
pub fn scrub(text: &str, url: &str) -> String {
    let mut text = text.replace(url, &redact_url(url));
    let mut secrets = Vec::new();
    if let Some((_, rest)) = url.split_once("://") {
        let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
        if let Some((userinfo, _)) = authority.rsplit_once('@')
            && let Some((_, password)) = userinfo.split_once(':')
        {
            secrets.push(password.to_string());
        }
    }
    if let Some((_, query)) = url.split_once('?') {
        for pair in query.split('&') {
            if let Some((key, value)) = pair.split_once('=')
                && key.eq_ignore_ascii_case("password")
            {
                secrets.push(value.to_string());
            }
        }
    }
    for secret in secrets.into_iter().filter(|secret| !secret.is_empty()) {
        text = text.replace(&secret, "***");
    }
    text
}

/// Open (or create) the database described by `url` and apply migrations.
///
/// A plain path is a SQLite file; `postgres://` selects PostgreSQL.
pub async fn connect(url: &str) -> sqlx::Result<(AnyPool, Backend)> {
    sqlx::any::install_default_drivers();
    let backend = backend_of(url);

    // Build the connection URL the any layer will hand to the backend.
    // The SQLite driver treats everything after the scheme as a raw
    // filesystem path, so the URL must be `sqlite:///<abs path>?mode=rwc`
    // (empty authority); `mode=rwc` creates the file when missing. WAL is
    // then set once because it persists in the database file.
    let connect_url = match backend {
        Backend::Sqlite => {
            let raw = url
                .strip_prefix("sqlite://")
                .or_else(|| url.strip_prefix("sqlite:"))
                .unwrap_or(url);
            let (path, query) = match raw.split_once('?') {
                Some((path, query)) => (path, Some(query)),
                None => (raw, None),
            };
            let absolute = std::path::Path::new(path);
            let absolute = if absolute.is_absolute() {
                absolute.to_path_buf()
            } else {
                std::env::current_dir().map_err(sqlx::Error::Io)?.join(path)
            };
            const PATH: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
                .add(b' ')
                .add(b'"')
                .add(b'#')
                .add(b'%')
                .add(b'<')
                .add(b'>')
                .add(b'?')
                .add(b'`')
                .add(b'{')
                .add(b'}');
            if query.is_none_or(|query| !query.contains("mode=")) {
                create_private_database(&absolute).map_err(sqlx::Error::Io)?;
            }
            let path_text = absolute.to_string_lossy().into_owned();
            let encoded = percent_encoding::utf8_percent_encode(&path_text, PATH);
            match query {
                Some(query) => format!("sqlite:///{encoded}?mode=rwc&{query}"),
                None => format!("sqlite:///{encoded}?mode=rwc"),
            }
        }
        // The scheme may be written in any case; the driver wants it lower.
        Backend::Postgres => match url.split_once("://") {
            Some((scheme, rest)) => format!("{}://{rest}", scheme.to_ascii_lowercase()),
            None => url.to_string(),
        },
    };

    // One plain connection first: an unreachable server fails here at once,
    // where the pool would keep retrying until its acquire timeout.
    if backend == Backend::Postgres {
        use sqlx::Connection;
        sqlx::AnyConnection::connect(&connect_url)
            .await?
            .close()
            .await?;
    }
    let pool = sqlx::any::AnyPoolOptions::new()
        .max_connections(8)
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                // SQLite defaults foreign_keys to off per connection; turn it
                // on so ON DELETE CASCADE and REFERENCES enforcement match
                // PostgreSQL's always-on behaviour.  WAL is already enabled
                // once by the pool initialiser so it persists.
                if conn.backend_name() == "SQLite" {
                    sqlx::query("PRAGMA foreign_keys = ON")
                        .execute(&mut *conn)
                        .await?;
                    // Overwrite deleted content, so a deleted account's
                    // rows do not linger in free pages of the file.
                    sqlx::query("PRAGMA secure_delete = ON")
                        .execute(&mut *conn)
                        .await?;
                }
                Ok(())
            })
        })
        .connect(&connect_url)
        .await?;
    if backend == Backend::Sqlite {
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await?;
    }
    migrations(backend).run(&pool).await?;
    Ok((pool, backend))
}

#[cfg(test)]
mod redaction_tests {
    use super::{redact_url, scrub};

    #[test]
    fn passwords_are_redacted() {
        assert_eq!(
            redact_url("postgres://thrum:s3cret@db:5432/thrum"),
            "postgres://thrum:***@db:5432/thrum"
        );
        assert_eq!(
            redact_url("postgres://db/thrum?user=thrum&password=s3cret&sslmode=require"),
            "postgres://db/thrum?user=thrum&password=***&sslmode=require"
        );
        assert_eq!(
            redact_url("postgres://thrum@db/thrum"),
            "postgres://thrum@db/thrum"
        );
        assert_eq!(redact_url("thrum.sqlite"), "thrum.sqlite");
        assert_eq!(redact_url("p://u:p@ss@h/d"), "p://u:***@h/d");
    }

    #[test]
    fn error_text_loses_the_password() {
        let url = "postgres://thrum:s3cret@db:5432/thrum";
        let text = format!("failed to open {url}: bad password s3cret");
        let clean = scrub(&text, url);
        assert!(!clean.contains("s3cret"), "{clean}");
        assert!(clean.contains("postgres://thrum:***@db:5432/thrum"));
    }
}
