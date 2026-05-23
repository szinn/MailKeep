use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use mk_core::{
    Error,
    account::AccountId,
    crypto::{CipherService, Ciphertext},
    storage::RawStorageService,
    types::ContentHash,
};
use tokio::fs;

use crate::{
    atomic::atomic_write_all,
    path::{account_root, blob_path},
};

pub(crate) struct FilesystemRawStorage {
    root: PathBuf,
    cipher: Arc<dyn CipherService>,
}

impl FilesystemRawStorage {
    pub(crate) fn new(root: PathBuf, cipher: Arc<dyn CipherService>) -> Self {
        Self { root, cipher }
    }
}

#[async_trait]
impl RawStorageService for FilesystemRawStorage {
    async fn put_if_absent(&self, account_id: AccountId, plaintext: &[u8]) -> Result<ContentHash, Error> {
        let hash = ContentHash::compute(plaintext);
        let path = blob_path(&self.root, account_id, &hash);
        if fs::try_exists(&path).await.map_err(|e| Error::Infrastructure(e.to_string()))? {
            return Ok(hash);
        }
        let ct = self.cipher.encrypt(account_id, plaintext);
        atomic_write_all(&path, ct.as_bytes()).await?;
        Ok(hash)
    }

    async fn get(&self, account_id: AccountId, key: &ContentHash) -> Result<Vec<u8>, Error> {
        let path = blob_path(&self.root, account_id, key);
        let bytes = match fs::read(&path).await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::BlobNotFound {
                    account_id,
                    hash: key.as_hex(),
                });
            }
            Err(e) => return Err(Error::Infrastructure(e.to_string())),
        };
        self.cipher.decrypt(account_id, &Ciphertext::from_raw(bytes))
    }

    async fn exists(&self, account_id: AccountId, key: &ContentHash) -> Result<bool, Error> {
        fs::try_exists(blob_path(&self.root, account_id, key))
            .await
            .map_err(|e| Error::Infrastructure(e.to_string()))
    }

    async fn delete_account(&self, account_id: AccountId) -> Result<(), Error> {
        let dir = account_root(&self.root, account_id);
        match fs::remove_dir_all(&dir).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Infrastructure(e.to_string())),
        }
    }
}
