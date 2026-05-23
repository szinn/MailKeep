use std::path::PathBuf;

/// Init-time errors from `create_filesystem_storage`. Never crosses a core
/// trait — only returned from the factory at startup, where the binary
/// converts to anyhow via `?`.
#[derive(thiserror::Error, Debug)]
pub enum StorageInitError {
    #[error("creating data directory {path}: {source}")]
    CreateDir { path: PathBuf, source: std::io::Error },

    #[error("data directory {path} is not writable: {source}")]
    NotWritable { path: PathBuf, source: std::io::Error },
}
