use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use tokio::{net::TcpStream, time::timeout};
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::{WebSocketStream, client_async, tungstenite::Message};

use crate::{DeviceId, Node, WireEvent, http};

/// The server pings every 10s, so this much silence means the peer is gone.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

type Ws = WebSocketStream<TlsStream<TcpStream>>;

/// Follows one paired device's event stream for as long as it is on the
/// network, reconnecting with backoff. `Node::reconcile` aborts it when the
/// device leaves or is unpaired.
pub(crate) async fn run(node: Node, peer_id: DeviceId, addrs: Vec<SocketAddr>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        match connect(&node, &peer_id, &addrs).await {
            Ok((ws, addr)) => {
                backoff = Duration::from_secs(1);
                if let Err(err) = follow(&node, &peer_id, addr, ws).await {
                    tracing::debug!("peer {peer_id} disconnected: {err:#}");
                }
            }
            Err(err) => tracing::debug!("peer {peer_id} unreachable: {err:#}"),
        }
        node.peer_gone(&peer_id);
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

async fn connect(node: &Node, peer_id: &DeviceId, addrs: &[SocketAddr]) -> Result<(Ws, SocketAddr)> {
    let (stream, addr) = http::connect_any(&node.inner.tls, peer_id, addrs).await?;
    let (ws, _) = client_async(format!("wss://{addr}/v1/events"), stream).await?;
    Ok((ws, addr))
}

async fn follow(node: &Node, peer_id: &DeviceId, addr: SocketAddr, mut ws: Ws) -> Result<()> {
    loop {
        let Some(msg) = timeout(READ_TIMEOUT, ws.next())
            .await
            .context("peer went quiet")?
        else {
            return Ok(());
        };
        match msg? {
            Message::Text(text) => match serde_json::from_str(&text)? {
                WireEvent::Snapshot { device, items } => {
                    ensure!(device.id == *peer_id, "peer changed identity");
                    node.peer_snapshot(device, items, addr);
                }
                WireEvent::Added { item } => node.peer_added(peer_id, item),
                WireEvent::Removed { id } => node.peer_removed(peer_id, &id),
            },
            Message::Close(_) => return Ok(()),
            // tungstenite answers pings on its own.
            _ => {}
        }
    }
}
