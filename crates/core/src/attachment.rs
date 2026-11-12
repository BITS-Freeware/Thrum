//! Client-side encryption of attachment files.
//!
//! Every file is encrypted with a fresh AES-256-GCM key before it is
//! uploaded; the key and nonce travel to the recipients inside the encrypted
//! MLS message, never through the server. The blob id is bound as
//! associated data so ciphertext cannot be re-registered under another id.

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};
use rand::RngCore;

/// Length of a per-file key: 32 bytes (AES-256).
pub const KEY_LEN: usize = 32;
/// Length of a per-file nonce: 12 bytes (GCM standard).
pub const NONCE_LEN: usize = 12;

/// A freshly generated per-file key and nonce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileKey {
    pub key: [u8; KEY_LEN],
    pub nonce: [u8; NONCE_LEN],
}

impl FileKey {
    /// Generate a new random key and nonce.
    pub fn generate() -> Self {
        let mut key = [0u8; KEY_LEN];
        let mut nonce = [0u8; NONCE_LEN];
        rand::rng().fill_bytes(&mut key);
        rand::rng().fill_bytes(&mut nonce);
        Self { key, nonce }
    }
}

/// Encrypt `plaintext` for the blob with id `blob_id`.
///
/// Returns the ciphertext with the 16-byte GCM tag appended.
pub fn encrypt_file(key: &FileKey, blob_id: &str, plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key.key));
    let nonce = Nonce::from_slice(&key.nonce);
    cipher
        .encrypt(
            nonce,
            Payload {
                msg: plaintext,
                aad: blob_id.as_bytes(),
            },
        )
        .expect("AES-GCM encryption cannot fail with valid key and nonce")
}

/// Decrypt a blob; `None` when the key, nonce, blob id or bytes do not
/// match (wrong key, tampering, or ciphertext from another blob).
pub fn decrypt_file(key: &FileKey, blob_id: &str, ciphertext: &[u8]) -> Option<Vec<u8>> {
    if ciphertext.len() < 16 {
        return None;
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key.key));
    let nonce = Nonce::from_slice(&key.nonce);
    cipher
        .decrypt(
            nonce,
            Payload {
                msg: ciphertext,
                aad: blob_id.as_bytes(),
            },
        )
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = FileKey::generate();
        let plaintext = b"file contents \xf0\x9f\x91\xbb with bytes";
        let ciphertext = encrypt_file(&key, "blob-1", plaintext);
        assert_ne!(ciphertext, plaintext);
        assert_eq!(ciphertext.len(), plaintext.len() + 16);
        assert_eq!(
            decrypt_file(&key, "blob-1", &ciphertext).as_deref(),
            Some(&plaintext[..])
        );
    }

    #[test]
    fn wrong_key_fails() {
        let key = FileKey::generate();
        let other = FileKey::generate();
        let ciphertext = encrypt_file(&key, "blob-1", b"secret");
        assert!(decrypt_file(&other, "blob-1", &ciphertext).is_none());
    }

    #[test]
    fn wrong_blob_id_fails() {
        let key = FileKey::generate();
        let ciphertext = encrypt_file(&key, "blob-1", b"secret");
        assert!(decrypt_file(&key, "blob-2", &ciphertext).is_none());
    }

    #[test]
    fn tampered_bytes_fail() {
        let key = FileKey::generate();
        let mut ciphertext = encrypt_file(&key, "blob-1", b"secret");
        ciphertext[0] ^= 1;
        assert!(decrypt_file(&key, "blob-1", &ciphertext).is_none());
    }

    #[test]
    fn truncated_ciphertext_fails() {
        let key = FileKey::generate();
        let ciphertext = encrypt_file(&key, "blob-1", b"secret");
        assert!(decrypt_file(&key, "blob-1", &ciphertext[..15]).is_none());
    }

    #[test]
    fn keys_are_fresh() {
        let a = FileKey::generate();
        let b = FileKey::generate();
        assert_ne!(a.key, b.key);
        assert_ne!(a.nonce, b.nonce);
    }
}
