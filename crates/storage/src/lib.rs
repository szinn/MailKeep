//! Filesystem adapter implementing `mk_core::storage` traits.
//!
//! `create_filesystem_storage` is the only public factory — it creates the
//! data directory layout, validates writability, and returns both storage
//! services with a shared cipher.

mod atomic;
mod attachment;
mod error;
mod path;
mod raw;

use std::{path::Path, sync::Arc};

pub use error::StorageInitError;
use mk_core::{
    crypto::CipherService,
    storage::{AttachmentStorageService, RawStorageService},
};
use tokio::fs;

use crate::{attachment::FilesystemAttachmentStorage, raw::FilesystemRawStorage};

/// Bundle returned by `create_filesystem_storage`.
pub struct FilesystemStorage {
    pub raw_storage_service: Arc<dyn RawStorageService>,
    pub attachment_storage_service: Arc<dyn AttachmentStorageService>,
}

/// Initialize the filesystem storage adapter against `data_dir`.
///
/// Creates `data_dir`, `data_dir/raw`, and `data_dir/attachments` if missing;
/// probe-writes a test file in each subdir to surface permission errors at
/// startup; constructs both storage services with the shared cipher.
pub async fn create_filesystem_storage(data_dir: &Path, cipher_service: Arc<dyn CipherService>) -> Result<FilesystemStorage, StorageInitError> {
    fs::create_dir_all(data_dir).await.map_err(|source| StorageInitError::CreateDir {
        path: data_dir.to_path_buf(),
        source,
    })?;

    let raw_root = data_dir.join("raw");
    let attachment_root = data_dir.join("attachments");

    fs::create_dir_all(&raw_root).await.map_err(|source| StorageInitError::CreateDir {
        path: raw_root.clone(),
        source,
    })?;
    fs::create_dir_all(&attachment_root).await.map_err(|source| StorageInitError::CreateDir {
        path: attachment_root.clone(),
        source,
    })?;

    probe_writable(&raw_root).await?;
    probe_writable(&attachment_root).await?;

    Ok(FilesystemStorage {
        raw_storage_service: Arc::new(FilesystemRawStorage::new(raw_root, cipher_service.clone())),
        attachment_storage_service: Arc::new(FilesystemAttachmentStorage::new(attachment_root, cipher_service)),
    })
}

async fn probe_writable(dir: &Path) -> Result<(), StorageInitError> {
    let probe = dir.join(".mailkeep-write-test");
    match fs::write(&probe, b"ok").await {
        Ok(()) => {
            let _ = fs::remove_file(&probe).await;
            Ok(())
        }
        Err(source) => Err(StorageInitError::NotWritable {
            path: dir.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use mk_core::{Error, crypto::create_cipher_service, types::ContentHash};
    use tempfile::TempDir;

    use super::*;

    const ACCOUNT_A: u64 = 100;
    const ACCOUNT_B: u64 = 200;

    fn cipher() -> Arc<dyn CipherService> {
        create_cipher_service("test-secret")
    }

    #[tokio::test]
    async fn factory_creates_subdirs() {
        let dir = TempDir::new().unwrap();
        let _storage = create_filesystem_storage(dir.path(), cipher()).await.unwrap();
        assert!(dir.path().join("raw").is_dir());
        assert!(dir.path().join("attachments").is_dir());
    }

    #[tokio::test]
    async fn factory_succeeds_when_subdirs_already_exist() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("raw")).await.unwrap();
        fs::create_dir_all(dir.path().join("attachments")).await.unwrap();
        let _storage = create_filesystem_storage(dir.path(), cipher()).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn factory_fails_when_data_dir_is_readonly() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
        perms.set_mode(0o555);
        std::fs::set_permissions(dir.path(), perms).unwrap();

        let result = create_filesystem_storage(dir.path(), cipher()).await;

        // Restore perms before TempDir teardown.
        let mut perms2 = std::fs::metadata(dir.path()).unwrap().permissions();
        perms2.set_mode(0o755);
        std::fs::set_permissions(dir.path(), perms2).unwrap();

        match result {
            Err(StorageInitError::CreateDir { .. } | StorageInitError::NotWritable { .. }) => {}
            Ok(_) => panic!("expected init error, got Ok"),
        }
    }

    #[tokio::test]
    async fn put_and_get_roundtrip_raw() {
        let dir = TempDir::new().unwrap();
        let storage = create_filesystem_storage(dir.path(), cipher()).await.unwrap();

        let plaintext = b"raw message body";
        let hash = storage.raw_storage_service.put_if_absent(ACCOUNT_A, plaintext).await.unwrap();

        let back = storage.raw_storage_service.get(ACCOUNT_A, &hash).await.unwrap();
        assert_eq!(back, plaintext);
    }

    #[tokio::test]
    async fn put_and_get_roundtrip_attachment() {
        let dir = TempDir::new().unwrap();
        let storage = create_filesystem_storage(dir.path(), cipher()).await.unwrap();

        let plaintext = b"attachment payload";
        let hash = storage.attachment_storage_service.put_if_absent(ACCOUNT_A, plaintext).await.unwrap();

        let back = storage.attachment_storage_service.get(ACCOUNT_A, &hash).await.unwrap();
        assert_eq!(back, plaintext);
    }

    #[tokio::test]
    async fn put_if_absent_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let storage = create_filesystem_storage(dir.path(), cipher()).await.unwrap();

        let plaintext = b"identical";
        let h1 = storage.raw_storage_service.put_if_absent(ACCOUNT_A, plaintext).await.unwrap();
        let h2 = storage.raw_storage_service.put_if_absent(ACCOUNT_A, plaintext).await.unwrap();
        assert_eq!(h1, h2);
    }

    #[tokio::test]
    async fn put_writes_under_expected_sharded_path() {
        let dir = TempDir::new().unwrap();
        let storage = create_filesystem_storage(dir.path(), cipher()).await.unwrap();

        let plaintext = b"hello";
        let hash = storage.raw_storage_service.put_if_absent(ACCOUNT_A, plaintext).await.unwrap();
        assert_eq!(hash, ContentHash::compute(plaintext));

        let (ab, cd) = hash.shard_dirs();
        let expected = dir
            .path()
            .join("raw")
            .join(ACCOUNT_A.to_string())
            .join(&ab)
            .join(&cd)
            .join(format!("{}.bin", hash.as_hex()));
        assert!(expected.is_file(), "expected file at {}", expected.display());
    }

    #[tokio::test]
    async fn exists_reflects_put() {
        let dir = TempDir::new().unwrap();
        let storage = create_filesystem_storage(dir.path(), cipher()).await.unwrap();

        let plaintext = b"existence-check";
        let hash = ContentHash::compute(plaintext);

        assert!(!storage.raw_storage_service.exists(ACCOUNT_A, &hash).await.unwrap());
        storage.raw_storage_service.put_if_absent(ACCOUNT_A, plaintext).await.unwrap();
        assert!(storage.raw_storage_service.exists(ACCOUNT_A, &hash).await.unwrap());
    }

    #[tokio::test]
    async fn get_missing_returns_blob_not_found() {
        let dir = TempDir::new().unwrap();
        let storage = create_filesystem_storage(dir.path(), cipher()).await.unwrap();

        let hash = ContentHash::compute(b"never put");
        let err = storage.raw_storage_service.get(ACCOUNT_A, &hash).await.unwrap_err();
        assert!(matches!(err, Error::BlobNotFound { .. }));
    }

    #[tokio::test]
    async fn get_with_other_account_returns_blob_not_found() {
        let dir = TempDir::new().unwrap();
        let storage = create_filesystem_storage(dir.path(), cipher()).await.unwrap();

        let plaintext = b"private to A";
        let hash = storage.raw_storage_service.put_if_absent(ACCOUNT_A, plaintext).await.unwrap();

        // Per-account sharding: B looks under raw/<B>/... — that subtree doesn't
        // exist, so this is a NotFound, not a decrypt attempt with wrong AAD.
        let err = storage.raw_storage_service.get(ACCOUNT_B, &hash).await.unwrap_err();
        assert!(matches!(err, Error::BlobNotFound { .. }));
    }

    #[tokio::test]
    async fn tampered_file_yields_decryption_failed() {
        let dir = TempDir::new().unwrap();
        let storage = create_filesystem_storage(dir.path(), cipher()).await.unwrap();

        let plaintext = b"unmodified content";
        let hash = storage.raw_storage_service.put_if_absent(ACCOUNT_A, plaintext).await.unwrap();

        let (ab, cd) = hash.shard_dirs();
        let path = dir
            .path()
            .join("raw")
            .join(ACCOUNT_A.to_string())
            .join(&ab)
            .join(&cd)
            .join(format!("{}.bin", hash.as_hex()));
        let mut bytes = fs::read(&path).await.unwrap();
        bytes[20] ^= 0xff;
        fs::write(&path, &bytes).await.unwrap();

        let err = storage.raw_storage_service.get(ACCOUNT_A, &hash).await.unwrap_err();
        assert!(matches!(err, Error::DecryptionFailed));
    }

    #[tokio::test]
    async fn delete_account_removes_subtree_and_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let storage = create_filesystem_storage(dir.path(), cipher()).await.unwrap();

        storage.raw_storage_service.put_if_absent(ACCOUNT_A, b"one").await.unwrap();
        storage.raw_storage_service.put_if_absent(ACCOUNT_A, b"two").await.unwrap();
        storage.attachment_storage_service.put_if_absent(ACCOUNT_A, b"att").await.unwrap();

        storage.raw_storage_service.delete_account(ACCOUNT_A).await.unwrap();
        storage.attachment_storage_service.delete_account(ACCOUNT_A).await.unwrap();

        assert!(!dir.path().join("raw").join(ACCOUNT_A.to_string()).exists());
        assert!(!dir.path().join("attachments").join(ACCOUNT_A.to_string()).exists());

        storage.raw_storage_service.delete_account(ACCOUNT_A).await.unwrap();
        storage.attachment_storage_service.delete_account(ACCOUNT_A).await.unwrap();
    }

    #[tokio::test]
    async fn two_accounts_with_identical_plaintext_have_independent_blobs() {
        let dir = TempDir::new().unwrap();
        let storage = create_filesystem_storage(dir.path(), cipher()).await.unwrap();

        let plaintext = b"shared content";
        let h_a = storage.raw_storage_service.put_if_absent(ACCOUNT_A, plaintext).await.unwrap();
        let h_b = storage.raw_storage_service.put_if_absent(ACCOUNT_B, plaintext).await.unwrap();
        assert_eq!(h_a, h_b);

        assert_eq!(storage.raw_storage_service.get(ACCOUNT_A, &h_a).await.unwrap(), plaintext);
        assert_eq!(storage.raw_storage_service.get(ACCOUNT_B, &h_b).await.unwrap(), plaintext);
    }
}
