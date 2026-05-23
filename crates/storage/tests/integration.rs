use std::sync::Arc;

use mk_core::{
    Error,
    crypto::{CipherService, MasterKey, create_cipher_service},
    types::ContentHash,
};
use mk_storage::create_filesystem_storage;
use tempfile::TempDir;
use tokio::fs;

const ACCOUNT_A: u64 = 100;
const ACCOUNT_B: u64 = 200;

fn cipher() -> Arc<dyn CipherService> {
    let key = MasterKey::derive("integration-test-secret");
    create_cipher_service(&key)
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
