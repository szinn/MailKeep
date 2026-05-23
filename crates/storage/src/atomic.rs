use std::path::Path;

use mk_core::Error;
use tokio::{fs, io::AsyncWriteExt};

/// Atomic durable write protocol:
///
/// 1. open `<path>.tmp`, write all bytes
/// 2. `sync_all()` the tmp file (contents durable)
/// 3. atomic `rename(tmp, path)`
/// 4. `sync_all()` the parent dir (dirent durable)
///
/// Returns `Error::Infrastructure(_)` on any io failure.
pub(crate) async fn atomic_write_all(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::Infrastructure(format!("path has no parent: {}", path.display())))?;
    fs::create_dir_all(parent).await.map_err(|e| Error::Infrastructure(e.to_string()))?;

    let file_name = path
        .file_name()
        .ok_or_else(|| Error::Infrastructure(format!("path has no file name: {}", path.display())))?;
    let tmp = parent.join(format!(".{}.tmp", file_name.to_string_lossy()));

    let mut f = fs::File::create(&tmp).await.map_err(|e| Error::Infrastructure(e.to_string()))?;
    f.write_all(bytes).await.map_err(|e| Error::Infrastructure(e.to_string()))?;
    f.sync_all().await.map_err(|e| Error::Infrastructure(e.to_string()))?;
    drop(f);

    fs::rename(&tmp, path).await.map_err(|e| Error::Infrastructure(e.to_string()))?;

    let parent_dir = fs::File::open(parent).await.map_err(|e| Error::Infrastructure(e.to_string()))?;
    parent_dir.sync_all().await.map_err(|e| Error::Infrastructure(e.to_string()))?;

    Ok(())
}
