use std::path::{Path, PathBuf};

use quickie_core::{Node, Snapshot};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

type CmdResult<T = ()> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

#[tauri::command]
pub fn snapshot(node: State<Node>) -> Snapshot {
    node.snapshot()
}

#[tauri::command]
pub fn share_text(node: State<Node>, text: String) -> CmdResult {
    node.share_text(text).map(|_| ()).map_err(err)
}

/// Mounts dropped or pasted files and folders. Returns a message for each one
/// that couldn't be shared.
#[tauri::command]
pub async fn mount_paths(node: State<'_, Node>, paths: Vec<PathBuf>) -> CmdResult<Vec<String>> {
    // Sizing up a big folder walks the whole tree; keep that off the async workers.
    let node = node.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        paths
            .iter()
            .filter_map(|path| {
                let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
                node.mount_path(path).err().map(|e| format!("{name}: {e:#}"))
            })
            .collect()
    })
    .await
    .map_err(err)
}

/// Files and folders copied in Finder / Explorer, if that's what's on the clipboard.
/// Sync, so Tauri runs it on the main thread where AppKit expects pasteboard access.
#[tauri::command]
pub fn clipboard_files() -> Vec<PathBuf> {
    use clipboard_rs::{Clipboard, ClipboardContext};
    let Ok(clipboard) = ClipboardContext::new() else { return Vec::new() };
    clipboard
        .get_files()
        .unwrap_or_default()
        .into_iter()
        .map(|f| PathBuf::from(f.strip_prefix("file://").unwrap_or(&f)))
        .filter(|p| p.exists())
        .collect()
}

#[tauri::command]
pub fn unmount(node: State<Node>, id: String) -> CmdResult {
    node.unmount(&id).map_err(err)
}

/// Local path to an item's bytes, downloading it first if needed.
#[tauri::command]
pub async fn fetch(node: State<'_, Node>, id: String) -> CmdResult<PathBuf> {
    node.fetch(&id).await.map_err(err)
}

#[tauri::command]
pub async fn open_item(app: AppHandle, node: State<'_, Node>, id: String) -> CmdResult {
    let path = node.fetch(&id).await.map_err(err)?;
    app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(err)
}

/// Copies an item out of the throwaway cache into Downloads and reveals it.
#[tauri::command]
pub async fn save_item(app: AppHandle, node: State<'_, Node>, id: String) -> CmdResult<PathBuf> {
    let src = node.fetch(&id).await.map_err(err)?;
    let dir = app.path().download_dir().map_err(err)?;
    let name = src.file_name().ok_or("item has no file name")?;
    let dest = unique_path(&dir, Path::new(name));
    let (from, to) = (src.clone(), dest.clone());
    tauri::async_runtime::spawn_blocking(move || copy_recursive(&from, &to))
        .await
        .map_err(err)?
        .map_err(err)?;
    app.opener().reveal_item_in_dir(&dest).map_err(err)?;
    Ok(dest)
}

fn copy_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    if !from.is_dir() {
        return std::fs::copy(from, to).map(|_| ());
    }
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        copy_recursive(&entry.path(), &to.join(entry.file_name()))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn request_pairing(node: State<'_, Node>, device: String) -> CmdResult {
    node.request_pairing(&device).await.map_err(err)
}

#[tauri::command]
pub async fn pair(node: State<'_, Node>, device: String, code: String) -> CmdResult {
    node.pair(&device, &code).await.map_err(err)
}

#[tauri::command]
pub fn cancel_pairing(node: State<Node>) {
    node.cancel_pairing();
}

/// Shows a QR code a phone can scan to pair its browser.
#[tauri::command]
pub fn create_web_invite(node: State<Node>) -> CmdResult {
    node.create_web_invite().map(|_| ()).map_err(err)
}

#[tauri::command]
pub fn cancel_web_invite(node: State<Node>) {
    node.cancel_web_invite();
}

#[tauri::command]
pub fn unpair(node: State<Node>, device: String) -> CmdResult {
    node.unpair(&device).map_err(err)
}

/// `report.pdf`, then `report (1).pdf`, `report (2).pdf`, ...
fn unique_path(dir: &Path, name: &Path) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let stem = name.file_stem().unwrap_or_default().to_string_lossy();
    let ext = name.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("ran out of file names")
}
