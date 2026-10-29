use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub type DeviceId = String;
pub type ItemId = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub platform: String,
}

/// Something on the shelf. The owner is the only device that can unmount it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub id: ItemId,
    pub owner: DeviceId,
    /// Unix epoch milliseconds.
    pub created_at: u64,
    /// When the owner will unmount it automatically, if ever.
    pub expires_at: Option<u64>,
    #[serde(flatten)]
    pub kind: ItemKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ItemKind {
    Text { text: String },
    /// Mounted by reference: the bytes stay on the owner until someone fetches them.
    File { name: String, size: u64, mime: String },
}

/// Peer-to-peer messages on `/v1/events`. Each device only ever announces its
/// own items, so there is a single source of truth per item and nothing to merge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireEvent {
    Snapshot { device: DeviceInfo, items: Vec<Item> },
    Added { item: Item },
    Removed { id: ItemId },
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceStatus {
    #[serde(flatten)]
    pub info: DeviceInfo,
    pub paired: bool,
    /// Paired: connected right now. Unpaired: visible on the network.
    pub online: bool,
}

/// Another device asked to pair; show this code so its user can type it in.
#[derive(Debug, Clone, Serialize)]
pub struct PairingPrompt {
    pub device: DeviceInfo,
    pub code: String,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Transfer {
    pub received: u64,
    pub total: u64,
}

/// The whole shelf as this device sees it.
#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub me: DeviceInfo,
    pub port: u16,
    /// Paired devices (online or not) and unpaired devices nearby.
    pub devices: Vec<DeviceStatus>,
    /// Newest first: our items plus every online paired device's.
    pub items: Vec<Item>,
    /// File items whose bytes are on this device: our own, or already fetched.
    pub local_files: HashMap<ItemId, String>,
    pub transfers: HashMap<ItemId, Transfer>,
    pub pairing: Option<PairingPrompt>,
}
