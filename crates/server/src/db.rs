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

/// Open (or create) the database described by `url` and apply migrations.
///
/// A plain path is a SQLite file; `postgres://` selects PostgreSQL.
pub async fn connect(url: &str) -> sqlx::Result<(AnyPool, Backend)> {
    sqlx::any::install_default_drivers();
    let backend = if url.starts_with("postgres://") || url.starts_with("postgresql://") {
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
            let path_text = absolute.to_string_lossy().into_owned();
            let encoded = percent_encoding::utf8_percent_encode(&path_text, PATH);
            match query {
                Some(query) => format!("sqlite:///{encoded}?mode=rwc&{query}"),
                None => format!("sqlite:///{encoded}?mode=rwc"),
            }
        }
        Backend::Postgres => url.to_string(),
    };

    let pool = sqlx::any::AnyPoolOptions::new()
        .max_connections(8)
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
