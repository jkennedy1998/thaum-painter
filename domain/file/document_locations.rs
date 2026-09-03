//! Painter document location defaults and root resolution: where documents and
//! session artifacts live, legacy root migration, and per-root document loading.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::storage::{
    load_or_create_shared_document, SharedDocumentFile, SharedDocumentPaths, SharedDocumentRuntime,
};
use crate::session_document::action_timestamp_string;

/// The default first layer every new document starts with.
pub const INITIAL_LAYER_ID: &str = "layer-1";
pub const INITIAL_LAYER_NAME: &str = "Layer 1";

pub fn shared_document_id() -> String {
    env::var("THAUM_SHARED_DOCUMENT_ID").unwrap_or_else(|_| "local-document".to_string())
}

pub fn painter_shared_document_paths(artifacts_root: &Path, document_id: &str) -> SharedDocumentPaths {
    SharedDocumentPaths::new(
        artifacts_root.join("shared-documents").join(document_id),
    )
}

pub fn default_shared_document(document_id: &str) -> SharedDocumentFile {
    SharedDocumentFile::single_layer(
        document_id,
        "Untitled Document",
        INITIAL_LAYER_ID,
        INITIAL_LAYER_NAME,
    )
}

fn legacy_painter_file_root(repo_root: &Path) -> PathBuf {
    repo_root.join("orchestration/context/painter/painter-files")
}

fn migrate_legacy_painter_file_root(legacy_root: &Path, target_root: &Path) {
    if target_root.exists() || !legacy_root.exists() {
        return;
    }
    if let Some(parent) = target_root.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::rename(legacy_root, target_root);
}

/// Resolves the painter saved-file root: the `THAUM_PAINTER_FILE_ROOT` env
/// override, else the repo's `context/painter/painter-files` folder (migrating
/// the legacy orchestration-local root on first run).
pub fn resolve_painter_file_root(repo_root: &Path) -> PathBuf {
    if let Ok(path) = env::var("THAUM_PAINTER_FILE_ROOT") {
        return PathBuf::from(path);
    }

    let target_root = repo_root.join("context/painter/painter-files");
    migrate_legacy_painter_file_root(&legacy_painter_file_root(repo_root), &target_root);
    target_root
}

pub fn load_document_from_root(root: &Path) -> Result<(SharedDocumentPaths, SharedDocumentRuntime)> {
    let document_id = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("untitled-document")
        .to_string();
    let paths = SharedDocumentPaths::new(root.to_path_buf());
    let runtime = load_or_create_shared_document(&paths, default_shared_document(&document_id))?;
    Ok((paths, runtime))
}

pub fn new_unsaved_document() -> SharedDocumentRuntime {
    let document_id = format!("document-{}", action_timestamp_string());
    SharedDocumentRuntime::new(default_shared_document(&document_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn painter_file_root_defaults_to_repo_context_folder() {
        let root = resolve_painter_file_root(Path::new("/tmp/fake-repo"));
        assert_eq!(
            root,
            PathBuf::from("/tmp/fake-repo/context/painter/painter-files")
        );
    }
}
