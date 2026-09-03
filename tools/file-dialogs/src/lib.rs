//! Native file-dialog and terminal-prompt plumbing for resolving painter
//! document paths. No painter-domain truth lives here; callers consume the
//! resolved document root folders only.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use rfd::FileDialog;

pub fn slugify_file_stem(text: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;
    for ch in text.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            slug.push(lower);
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

pub fn native_file_dialog_backend_name() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "XDG desktop portal"
    }
    #[cfg(target_os = "windows")]
    {
        "Windows native dialog"
    }
    #[cfg(target_os = "macos")]
    {
        "macOS native dialog"
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        "native dialog"
    }
}

pub fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

pub fn log_missing_dialog_selection(action: &str, file_root: &Path) {
    eprintln!(
        "thaum-painter: {action} dialog returned no selection. backend={}. root={}",
        native_file_dialog_backend_name(),
        display_path(file_root)
    );
    #[cfg(target_os = "linux")]
    eprintln!(
        "thaum-painter: on Linux this backend depends on a live desktop-portal session; if no dialog appears, check xdg-desktop-portal / DBus availability in the current desktop session."
    );
}

pub fn prompt_path_in_terminal(prompt: &str, file_root: &Path) -> Option<PathBuf> {
    eprintln!("thaum-painter: {prompt}");
    eprintln!("thaum-painter: press Enter to cancel");
    eprint!("thaum-painter path [{}]: ", display_path(file_root));
    let _ = io::stderr().flush();
    let _ = io::stdout().flush();
    let mut line = String::new();
    if io::stdin().read_line(&mut line).ok()? == 0 {
        return None;
    }
    let trimmed = line.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

pub fn normalize_open_document_root(path: &Path) -> PathBuf {
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        return path.parent().unwrap_or(path).to_path_buf();
    }
    path.to_path_buf()
}

pub fn prompt_open_document_root(file_root: &Path) -> Option<PathBuf> {
    let selected = FileDialog::new()
        .set_directory(file_root)
        .set_title("Open thaum-painter document")
        .add_filter("Thaum painter document", &["json"])
        .pick_file()
        .map(|path| normalize_open_document_root(&path));
    if let Some(path) = selected {
        return Some(path);
    }
    log_missing_dialog_selection("open", file_root);
    prompt_path_in_terminal(
        "native open dialog unavailable; type a document.json path or a document folder path",
        file_root,
    )
    .map(|path| normalize_open_document_root(&path))
}

pub fn save_as_root_from_dialog_path(path: &Path) -> PathBuf {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("document.json"))
    {
        return path.parent().unwrap_or(path).to_path_buf();
    }

    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(slugify_file_stem)
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "untitled-document".to_string());
    path.parent()
        .unwrap_or(path)
        .join(stem)
}

pub fn prompt_save_document_root(file_root: &Path, title: &str) -> Option<PathBuf> {
    let suggested = slugify_file_stem(title);
    let suggested = if suggested.is_empty() {
        "untitled-document".to_string()
    } else {
        suggested
    };
    let selected = FileDialog::new()
        .set_directory(file_root)
        .set_title("Save thaum-painter document as")
        .set_file_name(&format!("{suggested}.json"))
        .save_file()
        .map(|path| save_as_root_from_dialog_path(&path));
    if let Some(path) = selected {
        return Some(path);
    }
    log_missing_dialog_selection("save", file_root);
    prompt_path_in_terminal(
        "native save dialog unavailable; type a target document.json path or a folder/name path",
        file_root,
    )
    .map(|path| save_as_root_from_dialog_path(&path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_as_root_from_dialog_path_uses_document_parent_for_document_json() {
        let root = save_as_root_from_dialog_path(Path::new("/tmp/example/document.json"));
        assert_eq!(root, PathBuf::from("/tmp/example"));
    }

    #[test]
    fn save_as_root_from_dialog_path_uses_slugged_file_stem_for_other_names() {
        let root = save_as_root_from_dialog_path(Path::new("/tmp/example/My Sketch.json"));
        assert_eq!(root, PathBuf::from("/tmp/example/my-sketch"));
    }

    #[test]
    fn normalize_open_document_root_uses_parent_for_file_input() {
        let root = normalize_open_document_root(Path::new("/tmp/example/document.json"));
        assert_eq!(root, PathBuf::from("/tmp/example"));
    }

    #[test]
    fn normalize_open_document_root_keeps_directory_input() {
        let root = normalize_open_document_root(Path::new("/tmp/example-folder"));
        assert_eq!(root, PathBuf::from("/tmp/example-folder"));
    }
}
