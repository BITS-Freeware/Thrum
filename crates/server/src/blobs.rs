//! Blob storage backend and database helpers.
//!
//! The server stores encrypted attachment blobs on disk or in S3-compatible
//! object storage. The `BlobStore` trait abstracts the backend; the database
//! tracks uploader, size, and expiry so the sweeper can purge expired rows.
//! The server never sees plaintext: clients encrypt before they upload.

use std::io;
use std::path::PathBuf;

use async_trait::async_trait;
use rand::RngCore;
use sqlx::AnyPool;

use crate::accounts::now;
use crate::config::Config;

/// A blob id is 128 bits of randomness, hex encoded: 32 hex characters.
const BLOB_ID_LEN: usize = 32;

/// Errors returned by blob operations.
#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    #[error("blob not found")]
    NotFound,
    #[error("storage I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("S3 error: {0}")]
    S3(String),
}

/// A short stand-in for a blob id in logs. The id is a read capability, so
/// logs carry the first eight hex digits of its SHA-256 instead.
pub fn log_ref(id: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(id.as_bytes());
    let hex: String = digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("blob#{hex}")
}

/// `error` as text for a log line about blob `id`, with the id (which a
/// storage error can carry in an object path) replaced by [`log_ref`].
pub fn log_error(error: &dyn std::fmt::Display, id: &str) -> String {
    let text = error.to_string();
    if id.is_empty() {
        text
    } else {
        text.replace(id, &log_ref(id))
    }
}

/// Trait that abstracts the blob storage backend.
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Store `data` under `id`.
    async fn put(&self, id: &str, data: &[u8]) -> Result<(), BlobError>;
    /// Retrieve the blob stored under `id`, or `None` if it does not exist.
    async fn get(&self, id: &str) -> Result<Option<Vec<u8>>, BlobError>;
    /// Delete the blob stored under `id`.
    async fn delete(&self, id: &str) -> Result<(), BlobError>;
}

/// Accept only well-formed blob ids: exactly 32 hex characters. Anything
/// else cannot be a blob this server issued, and refusing early also keeps
/// foreign characters out of storage paths.
pub fn normalize_blob_id(id: &str) -> Option<String> {
    let bytes = id.as_bytes();
    if bytes.len() != BLOB_ID_LEN || !bytes.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    Some(id.to_ascii_lowercase())
}

/// Generate a fresh 128-bit blob id, hex encoded.
pub fn generate_blob_id() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    let mut out = String::with_capacity(BLOB_ID_LEN);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Disk-backed blob storage.
pub struct DiskStore {
    dir: PathBuf,
}

impl DiskStore {
    /// Create a disk store in `dir`, creating the directory if needed.
    /// A directory it creates is owner-only, and so is every blob file.
    pub fn new(dir: &str) -> io::Result<Self> {
        let dir = PathBuf::from(dir);
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&dir)?;
        Ok(Self { dir })
    }

    fn path(&self, id: &str) -> PathBuf {
        // Defense in depth: callers validate ids, but a path separator or
        // `..` must never reach the filesystem even if one slips through.
        debug_assert!(!id.contains('/') && !id.contains('\\') && id != "..");
        self.dir.join(id)
    }
}

#[async_trait]
impl BlobStore for DiskStore {
    async fn put(&self, id: &str, data: &[u8]) -> Result<(), BlobError> {
        use tokio::io::AsyncWriteExt;
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);
        // Write beside the final name and rename into place, so a fetch
        // never reads a partly written object.
        let target = self.path(id);
        let partial = self
            .dir
            .join(format!(".{id}.{}.partial", uuid::Uuid::new_v4()));
        let written = async {
            let mut file = options.open(&partial).await?;
            file.write_all(data).await?;
            file.flush().await?;
            file.sync_all().await?;
            tokio::fs::rename(&partial, &target).await
        }
        .await;
        if written.is_err() {
            let _ = tokio::fs::remove_file(&partial).await;
        }
        Ok(written?)
    }

    async fn get(&self, id: &str) -> Result<Option<Vec<u8>>, BlobError> {
        match tokio::fs::read(self.path(id)).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    async fn delete(&self, id: &str) -> Result<(), BlobError> {
        match tokio::fs::remove_file(self.path(id)).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(BlobError::NotFound),
            Err(error) => Err(error.into()),
        }
    }
}

/// S3-compatible blob storage via the `object_store` crate. Covers AWS S3,
/// Cloudflare R2 and MinIO; configuration comes from the operator's
/// environment through `Config`.
pub struct S3Store {
    store: std::sync::Arc<dyn object_store::ObjectStore>,
}

impl S3Store {
    /// Build an S3 store from configuration. Fails when the required S3
    /// settings are missing.
    pub fn new(cfg: &Config) -> Result<Self, BlobError> {
        if cfg.s3_endpoint.is_empty() || cfg.s3_bucket.is_empty() || cfg.s3_region.is_empty() {
            return Err(BlobError::S3(
                "THRUM_S3_ENDPOINT, THRUM_S3_BUCKET, and THRUM_S3_REGION must be set".to_string(),
            ));
        }
        let mut builder = object_store::aws::AmazonS3Builder::new()
            .with_region(&cfg.s3_region)
            .with_bucket_name(&cfg.s3_bucket)
            .with_endpoint(&cfg.s3_endpoint);
        if !cfg.s3_access_key.is_empty() && !cfg.s3_secret_key.is_empty() {
            builder = builder
                .with_access_key_id(&cfg.s3_access_key)
                .with_secret_access_key(&cfg.s3_secret_key);
        }
        // Self-hosted endpoints are often plain HTTP (MinIO on a LAN).
        if cfg.s3_endpoint.starts_with("http://") {
            builder = builder.with_allow_http(true);
        }
        let store: std::sync::Arc<dyn object_store::ObjectStore> =
            std::sync::Arc::new(builder.build().map_err(|e| BlobError::S3(e.to_string()))?);
        Ok(Self { store })
    }

    fn key(&self, id: &str) -> object_store::path::Path {
        object_store::path::Path::from(id)
    }
}

#[async_trait]
impl BlobStore for S3Store {
    async fn put(&self, id: &str, data: &[u8]) -> Result<(), BlobError> {
        use object_store::ObjectStoreExt as _;
        let loc = self.key(id);
        self.store
            .put(&loc, data.to_vec().into())
            .await
            .map_err(|e| BlobError::S3(e.to_string()))?;
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<Vec<u8>>, BlobError> {
        use object_store::ObjectStoreExt as _;
        let loc = self.key(id);
        match self.store.get(&loc).await {
            Ok(resp) => {
                let bytes = resp
                    .bytes()
                    .await
                    .map_err(|e| BlobError::S3(e.to_string()))?;
                Ok(Some(bytes.to_vec()))
            }
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(BlobError::S3(e.to_string())),
        }
    }

    async fn delete(&self, id: &str) -> Result<(), BlobError> {
        use object_store::ObjectStoreExt as _;
        let loc = self.key(id);
        match self.store.delete(&loc).await {
            Ok(()) => Ok(()),
            // Deleting twice is fine; the sweeper races with nothing.
            Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(e) => Err(BlobError::S3(e.to_string())),
        }
    }
}

// ── Database helpers ────────────────────────────────────────────────

/// Insert a blob record; `created_at` is the current Unix time in seconds.
pub async fn insert_blob(
    db: &AnyPool,
    id: &str,
    uploader_id: i64,
    size: usize,
    expires_at: i64,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO blobs (id, uploader_id, size, expires_at, created_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(uploader_id)
    .bind(size as i64)
    .bind(expires_at)
    .bind(now())
    .execute(db)
    .await?;
    Ok(())
}

/// Sum of blob sizes uploaded by `user_id` since `since_seconds`
/// (the start of the current UTC day).
pub async fn get_daily_bytes_since(
    db: &AnyPool,
    user_id: i64,
    since_seconds: i64,
) -> sqlx::Result<u64> {
    let (sum,): (Option<i64>,) = sqlx::query_as(
        "SELECT CAST(COALESCE(SUM(size), 0) AS BIGINT) FROM blobs
         WHERE uploader_id = $1 AND created_at >= $2",
    )
    .bind(user_id)
    .bind(since_seconds)
    .fetch_one(db)
    .await?;
    Ok(sum.unwrap_or(0).max(0) as u64)
}

/// Return blob ids whose expiry has passed.
pub async fn expired_blob_ids(db: &AnyPool) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar::<_, String>("SELECT id FROM blobs WHERE expires_at < $1")
        .bind(now())
        .fetch_all(db)
        .await
}

/// Delete a blob row by id.
pub async fn delete_blob_row(db: &AnyPool, id: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM blobs WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

/// Delete `id`'s object from `store`; an object that is already gone counts
/// as deleted.
pub async fn delete_object(store: &dyn BlobStore, id: &str) -> Result<(), BlobError> {
    match store.delete(id).await {
        Ok(()) | Err(BlobError::NotFound) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Remove every expired blob: first the object from storage, then the row
/// from the database. A row whose object the store fails to delete stays
/// for the next sweep, so the object is never left without a row pointing
/// at it. Also retries the objects of deleted accounts
/// ([`purge_tombstones`]). Returns how many objects were removed.
pub async fn sweep_expired(db: &AnyPool, store: &dyn BlobStore) -> usize {
    let mut removed = 0;
    if let Ok(ids) = expired_blob_ids(db).await {
        for id in ids {
            match delete_object(store, &id).await {
                Ok(()) => {
                    if delete_blob_row(db, &id).await.is_ok() {
                        removed += 1;
                    }
                }
                Err(error) => eprintln!(
                    "expired {} kept for the next sweep: {}",
                    log_ref(&id),
                    log_error(&error, &id)
                ),
            }
        }
    }
    removed + purge_tombstones(db, store).await
}

/// Delete the objects listed in `blob_tombstones` (their rows are already
/// gone), dropping each tombstone once its object is gone. Failures stay
/// listed for the next sweep. Returns how many were removed.
pub async fn purge_tombstones(db: &AnyPool, store: &dyn BlobStore) -> usize {
    let Ok(ids) = sqlx::query_scalar::<_, String>("SELECT id FROM blob_tombstones")
        .fetch_all(db)
        .await
    else {
        return 0;
    };
    let mut removed = 0;
    for id in ids {
        match delete_object(store, &id).await {
            Ok(()) => {
                if sqlx::query("DELETE FROM blob_tombstones WHERE id = $1")
                    .bind(&id)
                    .execute(db)
                    .await
                    .is_ok()
                {
                    removed += 1;
                }
            }
            Err(error) => eprintln!(
                "deleted account's {} kept for the next sweep: {}",
                log_ref(&id),
                log_error(&error, &id)
            ),
        }
    }
    removed
}

#[cfg(test)]
mod sweep_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A store whose deletes fail until `working` is set.
    struct FlakyStore {
        inner: DiskStore,
        working: AtomicBool,
    }

    #[async_trait]
    impl BlobStore for FlakyStore {
        async fn put(&self, id: &str, data: &[u8]) -> Result<(), BlobError> {
            self.inner.put(id, data).await
        }
        async fn get(&self, id: &str) -> Result<Option<Vec<u8>>, BlobError> {
            self.inner.get(id).await
        }
        async fn delete(&self, id: &str) -> Result<(), BlobError> {
            if self.working.load(Ordering::SeqCst) {
                self.inner.delete(id).await
            } else {
                Err(BlobError::S3("unavailable".into()))
            }
        }
    }

    #[tokio::test]
    async fn failed_object_deletes_are_retried_not_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let url = dir.path().join("t.sqlite").to_string_lossy().into_owned();
        let (db, _) = crate::db::connect(&url).await.unwrap();
        let store = FlakyStore {
            inner: DiskStore::new(&dir.path().join("blobs").to_string_lossy()).unwrap(),
            working: AtomicBool::new(false),
        };
        sqlx::query("INSERT INTO users (username, password_hash, created_at) VALUES ('u', 'h', 0)")
            .execute(&db)
            .await
            .unwrap();
        let expired = "a".repeat(32);
        let orphan = "b".repeat(32);
        store.put(&expired, b"x").await.unwrap();
        store.put(&orphan, b"y").await.unwrap();
        sqlx::query(
            "INSERT INTO blobs (id, uploader_id, size, expires_at, created_at)
             SELECT $1, id, 1, 1, 0 FROM users WHERE username = 'u'",
        )
        .bind(&expired)
        .execute(&db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO blob_tombstones (id, created_at) VALUES ($1, 0)")
            .bind(&orphan)
            .execute(&db)
            .await
            .unwrap();

        assert_eq!(sweep_expired(&db, &store).await, 0);
        let rows: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM blobs) + (SELECT COUNT(*) FROM blob_tombstones)",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(rows, 2, "rows must outlive a failed object delete");

        store.working.store(true, Ordering::SeqCst);
        assert_eq!(sweep_expired(&db, &store).await, 2);
        assert!(store.get(&expired).await.unwrap().is_none());
        assert!(store.get(&orphan).await.unwrap().is_none());
        // Gone already counts as deleted.
        assert!(delete_object(&store, &expired).await.is_ok());
    }
}

#[cfg(test)]
mod log_tests {
    use super::{log_error, log_ref};

    #[test]
    fn log_lines_do_not_carry_the_blob_id() {
        let id = "0123456789abcdef0123456789abcdef";
        let error = format!("Object at location {id} not found");
        let line = log_error(&error, id);
        assert!(!line.contains(id), "{line}");
        assert!(line.contains(&log_ref(id)));
        assert_eq!(log_ref(id).len(), "blob#".len() + 8);
        assert!(!log_ref(id).contains(&id[..8]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_id(byte: u8) -> String {
        format!("{byte:032x}")
    }

    #[test]
    fn generated_ids_are_valid() {
        for _ in 0..64 {
            let id = generate_blob_id();
            assert_eq!(normalize_blob_id(&id).as_deref(), Some(id.as_str()));
            assert_eq!(id.len(), BLOB_ID_LEN);
        }
        assert_ne!(generate_blob_id(), generate_blob_id());
    }

    #[test]
    fn id_validation_rejects_short_long_and_non_hex() {
        assert!(normalize_blob_id("").is_none());
        assert!(normalize_blob_id("abc").is_none());
        assert!(normalize_blob_id(&"a".repeat(31)).is_none());
        assert!(normalize_blob_id(&"a".repeat(33)).is_none());
        assert!(normalize_blob_id(&"z".repeat(32)).is_none());
        assert!(normalize_blob_id("../../etc/passwd").is_none());
        assert!(normalize_blob_id(&"A".repeat(32)).is_some());
    }

    #[tokio::test]
    async fn disk_store_roundtrip() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = DiskStore::new(&dir.path().to_string_lossy()).unwrap();
        let id = hex_id(7);
        store.put(&id, b"hello disk").await.unwrap();
        assert_eq!(
            store.get(&id).await.unwrap().as_deref(),
            Some(&b"hello disk"[..])
        );
        store.delete(&id).await.unwrap();
        assert!(store.get(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn disk_store_missing_blob_is_none() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = DiskStore::new(&dir.path().to_string_lossy()).unwrap();
        assert!(store.get(&hex_id(9)).await.unwrap().is_none());
    }

    #[test]
    fn s3_store_requires_configuration() {
        let cfg = Config::default();
        assert!(S3Store::new(&cfg).is_err());
    }

    #[test]
    fn s3_store_builds_with_configuration() {
        let cfg = Config {
            s3_endpoint: "https://blobs.example.com".to_string(),
            s3_bucket: "test".to_string(),
            s3_region: "auto".to_string(),
            ..Default::default()
        };
        // Construction validates settings; it does not contact the endpoint.
        S3Store::new(&cfg).expect("s3 store builds");
    }
}
