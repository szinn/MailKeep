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
    use mk_core::crypto::{MasterKey, create_cipher_service};
    use tempfile::TempDir;

    use super::*;

    fn cipher() -> Arc<dyn CipherService> {
        let key = MasterKey::derive("test-secret");
        create_cipher_service(&key)
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
}
