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

/// Open (or create) the database described by `url` and apply migrations.
///
/// A plain path is a SQLite file; `postgres://` selects PostgreSQL.
pub async fn connect(url: &str) -> sqlx::Result<(AnyPool, Backend)> {
    sqlx::any::install_default_drivers();
    let scheme = url
        .split_once("://")
        .map(|(scheme, _)| scheme.to_ascii_lowercase());
    let backend = if matches!(scheme.as_deref(), Some("postgres" | "postgresql")) {
        Backend::Postgres
    } else {
        Backend::Sqlite
    };

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
