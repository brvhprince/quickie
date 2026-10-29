use std::net::SocketAddr;

use anyhow::Result;
use mdns_sd::{Receiver, ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::{DeviceInfo, Node};

pub(crate) const SERVICE: &str = "_quickie._tcp.local.";
/// TXT values are capped at 255 bytes; keep names comfortably short.
const MAX_NAME_CHARS: usize = 63;

/// Advertises this device on the LAN. Returns the mDNS fullname for unregistering.
///
/// The advertised id is only a hint: connections pin it against the TLS
/// certificate, so a device can't usefully claim someone else's id.
pub(crate) fn advertise(mdns: &ServiceDaemon, me: &DeviceInfo, port: u16) -> Result<String> {
    let name: String = me.name.chars().take(MAX_NAME_CHARS).collect();
    let host = format!("quickie-{}.local.", &me.id[..12]);
    let props = [
        ("v", "1"),
        ("id", me.id.as_str()),
        ("name", name.as_str()),
        ("platform", me.platform.as_str()),
    ];
    let info = ServiceInfo::new(SERVICE, &me.id, &host, "", port, &props[..])?.enable_addr_auto();
    let fullname = info.get_fullname().to_string();
    mdns.register(info)?;
    Ok(fullname)
}

/// Feeds the node every Quickie device that appears on or leaves the network.
pub(crate) async fn browse(node: Node, events: Receiver<ServiceEvent>) {
    while let Ok(event) = events.recv_async().await {
        match event {
            ServiceEvent::ServiceResolved(svc) => {
                let txt = &svc.txt_properties;
                let (Some(id), Some(name)) = (txt.get_property_val_str("id"), txt.get_property_val_str("name")) else {
                    continue;
                };
                if id == node.me().id {
                    continue;
                }
                let info = DeviceInfo {
                    id: id.to_string(),
                    name: name.to_string(),
                    platform: txt.get_property_val_str("platform").unwrap_or("unknown").to_string(),
                };
                // IPv4 only for now: IPv6 link-local addresses need a scope id in the URL.
                let mut addrs: Vec<SocketAddr> = svc
                    .get_addresses_v4()
                    .into_iter()
                    .filter(|ip| !ip.is_loopback())
                    .map(|ip| SocketAddr::from((ip, svc.get_port())))
                    .collect();
                addrs.sort();
                if !addrs.is_empty() {
                    node.device_discovered(svc.fullname.clone(), info, addrs);
                }
            }
            ServiceEvent::ServiceRemoved(_, fullname) => node.device_lost(&fullname),
            _ => {}
        }
    }
}
