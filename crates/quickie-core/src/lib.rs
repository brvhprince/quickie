//! Quickie's networked core: LAN discovery, pairing, the shared shelf, and the
//! peer protocol. Shared by the desktop app, the (later) mobile app, and the
//! phone web client host.

mod discovery;
mod files;
mod http;
mod identity;
mod pairing;
mod peer;
mod server;
mod tls;
mod types;

use std::{
    collections::{HashMap, hash_map::Entry},
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, bail, ensure};
use mdns_sd::ServiceDaemon;
use tokio::{sync::broadcast, task::AbortHandle};

pub use identity::default_device_name;
pub use types::{
    DeviceId, DeviceInfo, DeviceStatus, Item, ItemId, ItemKind, PairingPrompt, Snapshot, Transfer,
    WireEvent,
};

pub const DEFAULT_PORT: u16 = 47800;
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
pub const DEFAULT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

pub struct Config {
    pub data_dir: PathBuf,
    /// Defaults to the machine's hostname.
    pub device_name: Option<String>,
    pub port: u16,
    /// How long shared items stay on the shelf before unmounting themselves.
    pub ttl: Duration,
}

impl Config {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self { data_dir: data_dir.into(), device_name: None, port: DEFAULT_PORT, ttl: DEFAULT_TTL }
    }
}

/// A running Quickie device. Cheap to clone.
#[derive(Clone)]
pub struct Node {
    inner: Arc<Inner>,
}

struct Inner {
    me: DeviceInfo,
    port: u16,
    data_dir: PathBuf,
    ttl: Duration,
    tls: tls::Tls,
    runtime: tokio::runtime::Handle,
    state: Mutex<State>,
    /// Our own item changes, streamed to peers.
    wire_tx: broadcast::Sender<WireEvent>,
    /// "Something changed", for the UI.
    changed_tx: broadcast::Sender<()>,
    mdns: ServiceDaemon,
    mdns_fullname: String,
}

#[derive(Default)]
struct State {
    local: Vec<Item>,
    local_paths: HashMap<ItemId, PathBuf>,
    paired: HashMap<DeviceId, DeviceInfo>,
    /// Every Quickie device currently advertising on the network.
    discovered: HashMap<DeviceId, Discovered>,
    /// Paired devices we're connected to, with their items.
    peers: HashMap<DeviceId, Peer>,
    /// One background connection per paired device that's on the network.
    links: HashMap<DeviceId, Link>,
    pairing: Option<pairing::Pending>,
    transfers: HashMap<ItemId, Transfer>,
    cached: HashMap<ItemId, PathBuf>,
}

struct Discovered {
    info: DeviceInfo,
    addrs: Vec<SocketAddr>,
    fullname: String,
}

struct Peer {
    info: DeviceInfo,
    items: Vec<Item>,
    addr: SocketAddr,
}

struct Link {
    addrs: Vec<SocketAddr>,
    task: AbortHandle,
}

impl Node {
    /// Binds the server, advertises on the LAN and starts following paired devices.
    /// Must be called from within a Tokio runtime.
    pub async fn start(config: Config) -> Result<Node> {
        let data_dir = config.data_dir;
        std::fs::create_dir_all(&data_dir)?;
        let identity = identity::load_or_create(&data_dir)?;
        let me = DeviceInfo {
            id: identity.id.clone(),
            name: config.device_name.unwrap_or_else(identity::default_device_name),
            platform: std::env::consts::OS.to_string(),
        };
        let tls = tls::Tls::new(&identity)?;
        let paired = pairing::load_paired(&data_dir)?;
        files::reset_cache(&data_dir);

        let listener = server::bind(config.port).await?;
        let port = listener.local_addr()?.port();
        let mdns = ServiceDaemon::new()?;
        let mdns_fullname = discovery::advertise(&mdns, &me, port)?;
        let browse = mdns.browse(discovery::SERVICE)?;

        let acceptor = tls.acceptor();
        let node = Node {
            inner: Arc::new(Inner {
                me,
                port,
                data_dir,
                ttl: config.ttl,
                tls,
                runtime: tokio::runtime::Handle::current(),
                state: Mutex::new(State { paired, ..State::default() }),
                wire_tx: broadcast::channel(256).0,
                changed_tx: broadcast::channel(64).0,
                mdns,
                mdns_fullname,
            }),
        };
        tokio::spawn(server::serve(listener, acceptor, node.clone()));
        tokio::spawn(discovery::browse(node.clone(), browse));
        tokio::spawn(expire_items(node.clone()));
        tracing::info!("quickie node {} ({}) on port {port}", node.me().name, node.me().id);
        Ok(node)
    }

    pub fn me(&self) -> &DeviceInfo {
        &self.inner.me
    }

    pub fn port(&self) -> u16 {
        self.inner.port
    }

    /// Fires whenever anything in [`Snapshot`] changes.
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.inner.changed_tx.subscribe()
    }

    pub fn snapshot(&self) -> Snapshot {
        let state = self.state();
        let mut devices: Vec<DeviceStatus> = state
            .paired
            .values()
            .map(|info| DeviceStatus {
                info: state.peers.get(&info.id).map_or(info, |p| &p.info).clone(),
                paired: true,
                online: state.peers.contains_key(&info.id),
            })
            .chain(
                state
                    .discovered
                    .values()
                    .filter(|d| !state.paired.contains_key(&d.info.id))
                    .map(|d| DeviceStatus { info: d.info.clone(), paired: false, online: true }),
            )
            .collect();
        devices.sort_by(|a, b| a.info.name.cmp(&b.info.name));

        let mut items: Vec<Item> = state
            .local
            .iter()
            .chain(state.peers.values().flat_map(|p| &p.items))
            .cloned()
            .collect();
        items.sort_by_key(|i| std::cmp::Reverse(i.created_at));

        let local_files = state
            .local_paths
            .iter()
            .chain(&state.cached)
            .map(|(id, path)| (id.clone(), path.to_string_lossy().into_owned()))
            .collect();

        let pairing = state.pairing.as_ref().filter(|p| p.is_live()).map(|p| PairingPrompt {
            device: p.peer.clone(),
            code: p.code.clone(),
            expires_at: p.expires_at,
        });

        Snapshot {
            me: self.inner.me.clone(),
            port: self.inner.port,
            devices,
            items,
            local_files,
            transfers: state.transfers.clone(),
            pairing,
        }
    }

    pub fn share_text(&self, text: String) -> Result<Item> {
        ensure!(!text.trim().is_empty(), "Nothing to share");
        ensure!(text.len() <= MAX_TEXT_BYTES, "Text is over {MAX_TEXT_BYTES} bytes");
        Ok(self.add_local(ItemKind::Text { text }, None))
    }

    /// Takes one of our own items off everyone's shelf.
    pub fn unmount(&self, id: &str) -> Result<()> {
        {
            let mut state = self.state();
            let Some(pos) = state.local.iter().position(|i| i.id == id) else {
                bail!("Not one of this device's items");
            };
            state.local.remove(pos);
            state.local_paths.remove(id);
            let _ = self.inner.wire_tx.send(WireEvent::Removed { id: id.to_string() });
        }
        self.changed();
        Ok(())
    }

    /// Dismisses an incoming pairing request.
    pub fn cancel_pairing(&self) {
        self.state().pairing = None;
        self.changed();
    }

    /// Forgets a paired device. It can no longer see this shelf (and vice versa).
    pub fn unpair(&self, device: &str) -> Result<()> {
        {
            let mut state = self.state();
            ensure!(state.paired.remove(device).is_some(), "Not paired with that device");
            pairing::save_paired(&self.inner.data_dir, &state.paired);
        }
        self.reconcile();
        self.changed();
        Ok(())
    }

    /// Unregisters from mDNS so peers drop us immediately instead of timing out.
    pub fn shutdown(&self) {
        if let Ok(rx) = self.inner.mdns.unregister(&self.inner.mdns_fullname) {
            let _ = rx.recv_timeout(Duration::from_millis(500));
        }
        let _ = self.inner.mdns.shutdown();
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn changed(&self) {
        let _ = self.inner.changed_tx.send(());
    }

    fn add_local(&self, kind: ItemKind, path: Option<PathBuf>) -> Item {
        let now = now_ms();
        let item = Item {
            id: uuid::Uuid::new_v4().simple().to_string(),
            owner: self.inner.me.id.clone(),
            created_at: now,
            expires_at: Some(now + self.inner.ttl.as_millis() as u64),
            kind,
        };
        {
            let mut state = self.state();
            state.local.push(item.clone());
            if let Some(path) = path {
                state.local_paths.insert(item.id.clone(), path);
            }
            // Sent under the lock so subscribers never miss or reorder an event.
            let _ = self.inner.wire_tx.send(WireEvent::Added { item: item.clone() });
        }
        self.changed();
        item
    }

    fn local_items(&self) -> Vec<Item> {
        self.state().local.clone()
    }

    fn local_path(&self, id: &str) -> Option<PathBuf> {
        self.state().local_paths.get(id).cloned()
    }

    fn is_paired(&self, id: &str) -> bool {
        self.state().paired.contains_key(id)
    }

    fn trust(&self, device: DeviceInfo) {
        {
            let mut state = self.state();
            state.paired.insert(device.id.clone(), device);
            pairing::save_paired(&self.inner.data_dir, &state.paired);
        }
        self.reconcile();
        self.changed();
    }

    /// A snapshot of our own items plus a subscription that starts exactly after it.
    fn subscribe_wire(&self) -> (WireEvent, broadcast::Receiver<WireEvent>) {
        let state = self.state();
        let rx = self.inner.wire_tx.subscribe();
        let snapshot =
            WireEvent::Snapshot { device: self.inner.me.clone(), items: state.local.clone() };
        (snapshot, rx)
    }

    /// Runs exactly one link per paired device that is on the network.
    fn reconcile(&self) {
        let mut stale_cache = Vec::new();
        {
            let state = &mut *self.state();
            let wanted: HashMap<DeviceId, Vec<SocketAddr>> = state
                .discovered
                .values()
                .filter(|d| state.paired.contains_key(&d.info.id))
                .map(|d| (d.info.id.clone(), d.addrs.clone()))
                .collect();

            state.links.retain(|id, link| {
                let keep = wanted.get(id).is_some_and(|addrs| *addrs == link.addrs);
                if !keep {
                    link.task.abort();
                }
                keep
            });
            for (id, addrs) in wanted {
                if let Entry::Vacant(slot) = state.links.entry(id) {
                    let task = self.inner.runtime.spawn(peer::run(self.clone(), slot.key().clone(), addrs.clone()));
                    slot.insert(Link { addrs, task: task.abort_handle() });
                }
            }

            let gone: Vec<DeviceId> =
                state.peers.keys().filter(|id| !state.links.contains_key(*id)).cloned().collect();
            for id in gone {
                if let Some(peer) = state.peers.remove(&id) {
                    stale_cache.extend(self.forget(&mut state.cached, peer.items.iter().map(|i| &i.id)));
                }
            }
        }
        remove_dirs(stale_cache);
    }

    fn device_discovered(&self, fullname: String, info: DeviceInfo, addrs: Vec<SocketAddr>) {
        {
            let mut state = self.state();
            if let Some(known) = state.discovered.get(&info.id)
                && known.info == info
                && known.addrs == addrs
            {
                return;
            }
            tracing::info!("found {} ({}) at {addrs:?}", info.name, info.id);
            state.discovered.insert(info.id.clone(), Discovered { info, addrs, fullname });
        }
        self.reconcile();
        self.changed();
    }

    fn device_lost(&self, fullname: &str) {
        {
            let mut state = self.state();
            let before = state.discovered.len();
            state.discovered.retain(|_, d| d.fullname != fullname);
            if state.discovered.len() == before {
                return;
            }
        }
        self.reconcile();
        self.changed();
    }

    fn peer_snapshot(&self, info: DeviceInfo, items: Vec<Item>, addr: SocketAddr) {
        let stale = {
            let state = &mut *self.state();
            let items: Vec<Item> = items.into_iter().filter(|i| i.owner == info.id).collect();
            let old = state.peers.remove(&info.id).map(|p| p.items).unwrap_or_default();
            let removed = old.iter().map(|i| &i.id).filter(|id| !items.iter().any(|i| i.id == **id));
            let stale = self.forget(&mut state.cached, removed);
            if let Some(paired) = state.paired.get_mut(&info.id)
                && *paired != info
            {
                *paired = info.clone();
                pairing::save_paired(&self.inner.data_dir, &state.paired);
            }
            state.peers.insert(info.id.clone(), Peer { info, items, addr });
            stale
        };
        remove_dirs(stale);
        self.changed();
    }

    fn peer_added(&self, peer_id: &str, item: Item) {
        if item.owner != peer_id {
            return;
        }
        if let Some(peer) = self.state().peers.get_mut(peer_id) {
            peer.items.retain(|i| i.id != item.id);
            peer.items.push(item);
        }
        self.changed();
    }

    fn peer_removed(&self, peer_id: &str, id: &str) {
        let stale = {
            let state = &mut *self.state();
            let Some(peer) = state.peers.get_mut(peer_id) else { return };
            peer.items.retain(|i| i.id != id);
            self.forget(&mut state.cached, [&id.to_string()])
        };
        remove_dirs(stale);
        self.changed();
    }

    fn peer_gone(&self, peer_id: &str) {
        let stale = {
            let state = &mut *self.state();
            let Some(peer) = state.peers.remove(peer_id) else { return };
            self.forget(&mut state.cached, peer.items.iter().map(|i| &i.id))
        };
        remove_dirs(stale);
        self.changed();
    }

    /// Drops downloaded copies of items that left the shelf; returns the dirs to
    /// delete once the state lock is released.
    fn forget<'a>(
        &self,
        cached: &mut HashMap<ItemId, PathBuf>,
        ids: impl IntoIterator<Item = &'a ItemId>,
    ) -> Vec<PathBuf> {
        let root = files::cache_root(&self.inner.data_dir);
        ids.into_iter()
            .map(|id| {
                cached.remove(id);
                root.join(id)
            })
            .collect()
    }
}

fn remove_dirs(dirs: Vec<PathBuf>) {
    for dir in dirs {
        let _ = std::fs::remove_dir_all(dir);
    }
}

async fn expire_items(node: Node) {
    let mut tick = tokio::time::interval(Duration::from_secs(15));
    loop {
        tick.tick().await;
        let now = now_ms();
        let expired: Vec<ItemId> = node
            .state()
            .local
            .iter()
            .filter(|i| i.expires_at.is_some_and(|t| t <= now))
            .map(|i| i.id.clone())
            .collect();
        for id in expired {
            let _ = node.unmount(&id);
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}
