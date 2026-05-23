use std::path::{Path, PathBuf};

use mk_core::{account::AccountId, types::ContentHash};

/// Build the per-account shard path for a given content hash:
/// `<root>/<account_id>/<ab>/<cd>/<hex>.bin`.
pub(crate) fn blob_path(root: &Path, account_id: AccountId, hash: &ContentHash) -> PathBuf {
    let (ab, cd) = hash.shard_dirs();
    root.join(account_id.to_string()).join(ab).join(cd).join(format!("{}.bin", hash.as_hex()))
}

/// Build the per-account root directory: `<root>/<account_id>`.
pub(crate) fn account_root(root: &Path, account_id: AccountId) -> PathBuf {
    root.join(account_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_path_uses_two_level_shard_lowercase() {
        let hash = ContentHash::compute(b"hello");
        let path = blob_path(Path::new("/data/raw"), 42, &hash);
        let s = path.to_string_lossy();
        assert!(s.starts_with("/data/raw/42/2c/f2/"));
        assert!(s.ends_with(".bin"));
        assert!(s.contains(&hash.as_hex()));
    }

    #[test]
    fn account_root_is_root_slash_id() {
        let p = account_root(Path::new("/data/raw"), 7);
        assert_eq!(p, Path::new("/data/raw/7"));
    }
}
