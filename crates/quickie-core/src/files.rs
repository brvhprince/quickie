//! File and folder items: mount by reference, fetch on demand into a throwaway cache.
//!
//! Folders travel as an uncompressed zip that the owner builds while sending it,
//! so nothing is copied up front; the receiver unpacks it back into a folder.

use std::{
    ffi::OsString,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use axum::body::Body;
use http_body_util::{BodyExt, Full};
use hyper::{
    Request, StatusCode,
    header::{HOST, RANGE},
};
use tokio::{fs, io::AsyncWriteExt};
use tokio_util::io::{ReaderStream, SyncIoBridge};
use walkdir::{DirEntry, WalkDir};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::{DeviceId, Item, ItemKind, Node, Transfer, http};

const PROGRESS_EVERY: Duration = Duration::from_millis(150);
const MAX_FOLDER_FILES: u64 = 50_000;
/// OS litter that nobody means to share.
const JUNK_FILES: [&str; 3] = [".DS_Store", "Thumbs.db", "desktop.ini"];

pub(crate) fn cache_root(data_dir: &Path) -> PathBuf {
    data_dir.join("cache")
}

/// The cache only holds copies of other devices' items, which are gone once
/// this device restarts, so it starts empty every time.
pub(crate) fn reset_cache(data_dir: &Path) {
    let _ = std::fs::remove_dir_all(cache_root(data_dir));
}

/// Everything under a shared folder, in a stable order. Symlinks are skipped
/// rather than followed, so a share can't reach outside its folder.
fn folder_entries(root: &Path) -> impl Iterator<Item = walkdir::Result<DirEntry>> {
    WalkDir::new(root)
        .follow_links(false)
        .min_depth(1)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| !e.file_name().to_str().is_some_and(|n| JUNK_FILES.contains(&n)))
}

pub(crate) fn describe(path: &Path) -> Result<ItemKind> {
    let meta = std::fs::metadata(path).with_context(|| format!("can't read {}", path.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .context("it has no name")?;
    if meta.is_dir() {
        let (mut files, mut size) = (0u64, 0u64);
        for entry in folder_entries(path) {
            let entry = entry?;
            if entry.file_type().is_file() {
                files += 1;
                size += entry.metadata()?.len();
                ensure!(files <= MAX_FOLDER_FILES, "Folders with over {MAX_FOLDER_FILES} files aren't supported");
            }
        }
        return Ok(ItemKind::Folder { name, size, files });
    }
    ensure!(meta.is_file(), "Only files and folders can be shared");
    let mime = mime_guess::from_path(path).first_or_octet_stream().to_string();
    Ok(ItemKind::File { name, size: meta.len(), mime })
}

/// A peer chooses the name, so strip anything that could escape the cache dir
/// or upset Windows.
pub(crate) fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_control() || r#"<>:"|?*"#.contains(c) { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim_matches([' ', '.']);
    if cleaned.is_empty() { "file".into() } else { cleaned.into() }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

/// Streams a folder as a zip, built while it's being sent. Must be called
/// from within the Tokio runtime.
pub(crate) fn zip_body(dir: PathBuf) -> Body {
    let (reader, writer) = tokio::io::duplex(256 * 1024);
    let writer = SyncIoBridge::new(writer);
    tokio::task::spawn_blocking(move || {
        // On failure the stream just ends early; the receiver's unzip then fails cleanly.
        if let Err(err) = write_zip(&dir, writer) {
            tracing::warn!("sending {} failed: {err:#}", dir.display());
        }
    });
    Body::from_stream(ReaderStream::new(reader))
}

fn write_zip(dir: &Path, out: impl Write) -> Result<()> {
    let mut zip = ZipWriter::new_stream(out);
    // Stored, not deflated: most big files are already compressed, and this is a LAN.
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for entry in folder_entries(dir) {
        let entry = entry?;
        let relative = entry.path().strip_prefix(dir)?;
        let name: Vec<String> =
            relative.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        let name = name.join("/");
        if entry.file_type().is_dir() {
            zip.add_directory(name, options)?;
        } else if entry.file_type().is_file() {
            let large = entry.metadata()?.len() >= u64::from(u32::MAX);
            zip.start_file(name, options.large_file(large))?;
            std::io::copy(&mut std::fs::File::open(entry.path())?, &mut zip)?;
        }
    }
    zip.finish()?;
    Ok(())
}

/// Unpacks a received folder. Entry paths come from the network, so any that
/// would land outside `dest` are refused.
fn unzip(archive: &Path, dest: &Path) -> Result<()> {
    let mut archive = ZipArchive::new(std::fs::File::open(archive)?)?;
    std::fs::create_dir_all(dest)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let relative = entry.enclosed_name().context("the folder contained an unsafe path")?;
        let out = dest.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::io::copy(&mut entry, &mut std::fs::File::create(&out)?)?;
    }
    Ok(())
}

impl Node {
    /// Puts a file or folder on the shelf without copying it anywhere.
    pub fn mount_path(&self, path: impl Into<PathBuf>) -> Result<Item> {
        let path = path.into();
        let kind = describe(&path)?;
        Ok(self.add_local(kind, Some(path), None))
    }

    /// Path to an item's bytes on this device, downloading from the owner first
    /// if needed. Interrupted file downloads resume where they stopped.
    pub async fn fetch(&self, id: &str) -> Result<PathBuf> {
        let (owner, addr, dest, size, folder) = {
            let state = self.state();
            if let Some(path) = state.local_paths.get(id) {
                return Ok(path.clone());
            }
            if let Some(path) = state.cached.get(id).filter(|p| p.exists()) {
                return Ok(path.clone());
            }
            ensure!(!state.transfers.contains_key(id), "Already downloading");
            let (peer, item) = state
                .peers
                .values()
                .find_map(|p| p.items.iter().find(|i| i.id == id).map(|i| (p, i)))
                .context("That item is no longer on the shelf")?;
            let (name, size, folder) = match &item.kind {
                ItemKind::File { name, size, .. } => (name, *size, false),
                ItemKind::Folder { name, size, .. } => (name, *size, true),
                ItemKind::Text { .. } => bail!("Not a file"),
            };
            let dest = cache_root(&self.inner.data_dir).join(id).join(safe_file_name(name));
            (peer.info.id.clone(), peer.addr, dest, size, folder)
        };

        self.set_progress(id, Some(Transfer { received: 0, total: size }));
        let result = self.download(&owner, addr, id, size, folder, &dest).await;
        {
            let mut state = self.state();
            state.transfers.remove(id);
            let still_shared = state.peers.values().any(|p| p.items.iter().any(|i| i.id == id));
            if result.is_ok() && still_shared {
                state.cached.insert(id.to_string(), dest.clone());
            }
        }
        self.changed();
        result.map(|()| dest)
    }

    async fn download(
        &self,
        owner: &DeviceId,
        addr: SocketAddr,
        id: &str,
        total: u64,
        folder: bool,
        dest: &Path,
    ) -> Result<()> {
        // A zip built on the fly has no stable length, so only files can resume.
        let part = with_suffix(dest, if folder { ".zip.part" } else { ".part" });
        fs::create_dir_all(dest.parent().context("bad cache path")?).await?;
        let mut offset = if folder { 0 } else { fs::metadata(&part).await.map_or(0, |m| m.len()) };
        if offset >= total {
            offset = 0;
        }

        let (stream, addr) = http::connect_any(&self.inner.tls, owner, &[addr]).await?;
        let mut req = Request::get(format!("/v1/items/{id}/content")).header(HOST, addr.to_string());
        if offset > 0 {
            req = req.header(RANGE, format!("bytes={offset}-"));
        }
        let res = http::send(stream, req.body(Full::default())?).await?;
        let mut file = match res.status() {
            StatusCode::PARTIAL_CONTENT => fs::OpenOptions::new().append(true).open(&part).await?,
            StatusCode::OK => {
                offset = 0;
                fs::File::create(&part).await?
            }
            status => {
                let _ = fs::remove_file(&part).await;
                bail!("The owner couldn't send it ({status})");
            }
        };

        let mut received = offset;
        let mut last_report = Instant::now();
        let mut body = res.into_body();
        while let Some(frame) = body.frame().await {
            if let Ok(data) = frame?.into_data() {
                file.write_all(&data).await?;
                received += data.len() as u64;
                if last_report.elapsed() >= PROGRESS_EVERY {
                    last_report = Instant::now();
                    // Zip headers make a folder's stream a little bigger than its contents.
                    self.set_progress(id, Some(Transfer { received: received.min(total), total }));
                }
            }
        }
        file.flush().await?;
        drop(file);

        if folder {
            let staging = with_suffix(dest, ".unpacking");
            let (archive, unpack_to) = (part.clone(), staging.clone());
            let unpacked = tokio::task::spawn_blocking(move || unzip(&archive, &unpack_to)).await?;
            let _ = fs::remove_file(&part).await;
            if let Err(err) = unpacked {
                let _ = fs::remove_dir_all(&staging).await;
                return Err(err.context("The folder arrived damaged"));
            }
            let _ = fs::remove_dir_all(dest).await;
            fs::rename(&staging, dest).await?;
        } else {
            ensure!(received == total, "Download incomplete ({received} of {total} bytes)");
            fs::rename(&part, dest).await?;
        }
        Ok(())
    }

    fn set_progress(&self, id: &str, transfer: Option<Transfer>) {
        {
            let mut state = self.state();
            match transfer {
                Some(t) => state.transfers.insert(id.to_string(), t),
                None => state.transfers.remove(id),
            };
        }
        self.changed();
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{describe, safe_file_name, unzip, write_zip};
    use crate::ItemKind;

    #[test]
    fn file_names_cannot_escape_the_cache() {
        assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_file_name(r"..\..\boot.ini"), "boot.ini");
        assert_eq!(safe_file_name(".."), "file");
        assert_eq!(safe_file_name("a:b?.txt"), "a_b_.txt");
        assert_eq!(safe_file_name("photo.jpg"), "photo.jpg");
    }

    #[test]
    fn folders_round_trip_through_zip() {
        let root = std::env::temp_dir().join(format!("quickie-zip-{}", std::process::id()));
        let src = root.join("Holiday");
        fs::create_dir_all(src.join("day 1/raw")).unwrap();
        fs::create_dir_all(src.join("empty")).unwrap();
        fs::write(src.join("notes.txt"), "hello").unwrap();
        fs::write(src.join("day 1/raw/a.bin"), vec![7u8; 100_000]).unwrap();
        fs::write(src.join(".DS_Store"), "junk").unwrap();

        let ItemKind::Folder { name, size, files } = describe(&src).unwrap() else { panic!("not a folder") };
        assert_eq!((name.as_str(), size, files), ("Holiday", 100_005, 2));

        let mut zipped = Vec::new();
        write_zip(&src, &mut zipped).unwrap();
        let archive = root.join("Holiday.zip");
        fs::write(&archive, zipped).unwrap();
        let out = root.join("out");
        unzip(&archive, &out).unwrap();

        assert_eq!(fs::read_to_string(out.join("notes.txt")).unwrap(), "hello");
        assert_eq!(fs::read(out.join("day 1/raw/a.bin")).unwrap(), vec![7u8; 100_000]);
        assert!(out.join("empty").is_dir());
        assert!(!out.join(".DS_Store").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hostile_archives_cannot_write_outside_the_folder() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("quickie-zipslip-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut zipped = Vec::new();
        let mut zip = zip::ZipWriter::new_stream(&mut zipped);
        zip.start_file("../escaped.txt", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"gotcha").unwrap();
        zip.finish().unwrap();
        let archive = root.join("evil.zip");
        fs::write(&archive, zipped).unwrap();

        assert!(unzip(&archive, &root.join("out")).is_err());
        assert!(!root.join("escaped.txt").exists());
        let _ = fs::remove_dir_all(root);
    }
}
