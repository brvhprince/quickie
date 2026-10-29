//! File items: mount by reference, fetch on demand into a throwaway cache.

use std::{
    ffi::OsString,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use http_body_util::{BodyExt, Full};
use hyper::{
    Request, StatusCode,
    header::{HOST, RANGE},
};
use tokio::{fs, io::AsyncWriteExt};

use crate::{DeviceId, Item, ItemKind, Node, Transfer, http};

const PROGRESS_EVERY: Duration = Duration::from_millis(150);

pub(crate) fn cache_root(data_dir: &Path) -> PathBuf {
    data_dir.join("cache")
}

/// The cache only holds copies of other devices' items, which are gone once
/// this device restarts, so it starts empty every time.
pub(crate) fn reset_cache(data_dir: &Path) {
    let _ = std::fs::remove_dir_all(cache_root(data_dir));
}

pub(crate) fn describe(path: &Path) -> Result<ItemKind> {
    let meta = std::fs::metadata(path).with_context(|| format!("can't read {}", path.display()))?;
    ensure!(meta.is_file(), "Folders aren't supported yet");
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .context("file has no name")?;
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

fn part_path(dest: &Path) -> PathBuf {
    let mut name = OsString::from(dest.as_os_str());
    name.push(".part");
    PathBuf::from(name)
}

impl Node {
    /// Puts a file on the shelf without copying it anywhere.
    pub fn mount_file(&self, path: impl Into<PathBuf>) -> Result<Item> {
        let path = path.into();
        let kind = describe(&path)?;
        Ok(self.add_local(kind, Some(path), None))
    }

    /// Path to an item's bytes on this device, downloading from the owner first
    /// if needed. Interrupted downloads resume where they stopped.
    pub async fn fetch(&self, id: &str) -> Result<PathBuf> {
        let (item, owner, addr, dest) = {
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
                .find_map(|p| p.items.iter().find(|i| i.id == id).map(|i| (p, i.clone())))
                .context("That item is no longer on the shelf")?;
            let ItemKind::File { name, .. } = &item.kind else { bail!("Not a file") };
            let dest = cache_root(&self.inner.data_dir).join(id).join(safe_file_name(name));
            (item, peer.info.id.clone(), peer.addr, dest)
        };
        let ItemKind::File { size, .. } = item.kind else { unreachable!() };

        self.set_progress(id, Some(Transfer { received: 0, total: size }));
        let result = self.download(&owner, addr, id, size, &dest).await;
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

    async fn download(&self, owner: &DeviceId, addr: SocketAddr, id: &str, total: u64, dest: &Path) -> Result<()> {
        let part = part_path(dest);
        fs::create_dir_all(dest.parent().context("bad cache path")?).await?;
        let mut offset = fs::metadata(&part).await.map_or(0, |m| m.len());
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
                    self.set_progress(id, Some(Transfer { received, total }));
                }
            }
        }
        file.flush().await?;
        ensure!(received == total, "Download incomplete ({received} of {total} bytes)");
        fs::rename(&part, dest).await?;
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
    use super::safe_file_name;

    #[test]
    fn file_names_cannot_escape_the_cache() {
        assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_file_name(r"..\..\boot.ini"), "boot.ini");
        assert_eq!(safe_file_name(".."), "file");
        assert_eq!(safe_file_name("a:b?.txt"), "a_b_.txt");
        assert_eq!(safe_file_name("photo.jpg"), "photo.jpg");
    }
}
