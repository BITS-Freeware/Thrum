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
use sqlx::SqlitePool;

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
    pub fn new(dir: &str) -> io::Result<Self> {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir)?;
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
        tokio::fs::write(self.path(id), data).await?;
        Ok(())
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
                "GEIST3_S3_ENDPOINT, GEIST3_S3_BUCKET, and GEIST3_S3_REGION must be set"
                    .to_string(),
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
        let loc = self.key(id);
        self.store
            .put(&loc, data.to_vec().into())
            .await
            .map_err(|e| BlobError::S3(e.to_string()))?;
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<Vec<u8>>, BlobError> {
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

/// Insert a blob record; `created_at` is the current UTC time.
pub async fn insert_blob(
    db: &SqlitePool,
    id: &str,
    uploader_id: i64,
    size: usize,
    expires_at: i64,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO blobs (id, uploader_id, size, expires_at, created_at)
         VALUES (?1, ?2, ?3, ?4, datetime('now'))",
    )
    .bind(id)
    .bind(uploader_id)
    .bind(size as i64)
    .bind(expires_at)
    .execute(db)
    .await?;
    Ok(())
}

/// Sum of blob sizes uploaded by `user_id` on the current UTC calendar day.
pub async fn get_daily_bytes(db: &SqlitePool, user_id: i64) -> sqlx::Result<u64> {
    let (sum,): (Option<i64>,) = sqlx::query_as(
        "SELECT COALESCE(SUM(size), 0) FROM blobs
         WHERE uploader_id = ?1 AND date(created_at) = date('now')",
    )
    .bind(user_id)
    .fetch_one(db)
    .await?;
    Ok(sum.unwrap_or(0).max(0) as u64)
}

/// Return blob ids whose expiry has passed.
pub async fn expired_blob_ids(db: &SqlitePool) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar::<_, String>(
        "SELECT id FROM blobs
         WHERE expires_at < CAST(strftime('%s', 'now') AS INTEGER)",
    )
    .fetch_all(db)
    .await
}

/// Delete a blob row by id.
pub async fn delete_blob_row(db: &SqlitePool, id: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM blobs WHERE id = ?1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

/// Remove every expired blob: the row from the database and the object from
/// storage. Returns how many blobs were removed. Missing objects count as
/// removed; storage backends may race with the sweep.
pub async fn sweep_expired(db: &SqlitePool, store: &dyn BlobStore) -> usize {
    let Ok(ids) = expired_blob_ids(db).await else {
        return 0;
    };
    let mut removed = 0;
    for id in ids {
        let _ = store.delete(&id).await; // already gone is fine
        if delete_blob_row(db, &id).await.is_ok() {
            removed += 1;
        }
    }
    removed
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
