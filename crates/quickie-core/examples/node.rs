//! Headless Quickie node for testing without the desktop UI.
//!
//!   cargo run -p quickie-core --example node -- <name> [data-dir]
//!
//! Commands (device and item ids can be abbreviated to a unique prefix):
//!   <text>            share a line of text
//!   /file <path>      mount a file
//!   /get <item>       fetch an item's bytes and print the local path
//!   /rm <item>        unmount one of your items
//!   /ask <device>     ask a nearby device to show a pairing code
//!   /pair <device> <code>
//!   /unpair <device>
//!   /ls               print the shelf

use anyhow::{Context, Result, bail};
use quickie_core::{Config, ItemKind, Node, Snapshot};
use tokio::io::{AsyncBufReadExt, BufReader};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("quickie_core=info,warn").init();
    let mut args = std::env::args().skip(1);
    let name = args.next().unwrap_or_else(|| "headless".into());
    let data_dir = args.next().unwrap_or_else(|| format!("target/quickie-{name}"));

    let node = Node::start(Config { device_name: Some(name), ..Config::new(data_dir) }).await?;
    let mut changes = node.subscribe();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();

    loop {
        tokio::select! {
            Ok(()) = changes.recv() => print_shelf(&node.snapshot()),
            line = lines.next_line() => {
                let Some(line) = line? else { break };
                if let Err(err) = run(&node, &line).await {
                    println!("error: {err:#}");
                }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    node.shutdown();
    Ok(())
}

async fn run(node: &Node, line: &str) -> Result<()> {
    let mut words = line.split_whitespace();
    let snap = node.snapshot();
    match words.next() {
        Some("/ls") => print_shelf(&snap),
        Some("/file") => {
            node.mount_file(line["/file".len()..].trim())?;
        }
        Some("/get") => {
            let id = item_id(&snap, words.next())?;
            println!("fetched: {}", node.fetch(&id).await?.display());
        }
        Some("/rm") => node.unmount(&item_id(&snap, words.next())?)?,
        Some("/ask") => node.request_pairing(&device_id(&snap, words.next())?).await?,
        Some("/pair") => {
            let device = device_id(&snap, words.next())?;
            node.pair(&device, words.next().context("missing code")?).await?;
            println!("paired");
        }
        Some("/unpair") => node.unpair(&device_id(&snap, words.next())?)?,
        Some(cmd) if cmd.starts_with('/') => bail!("unknown command {cmd}"),
        _ => {
            node.share_text(line.to_string())?;
        }
    }
    Ok(())
}

fn item_id(snap: &Snapshot, prefix: Option<&str>) -> Result<String> {
    let prefix = prefix.context("missing item id")?;
    unique(snap.items.iter().map(|i| &i.id), prefix)
}

fn device_id(snap: &Snapshot, prefix: Option<&str>) -> Result<String> {
    let prefix = prefix.context("missing device")?;
    let by_name: Vec<_> = snap.devices.iter().filter(|d| d.info.name == prefix).collect();
    if let [device] = by_name.as_slice() {
        return Ok(device.info.id.clone());
    }
    unique(snap.devices.iter().map(|d| &d.info.id), prefix)
}

fn unique<'a>(ids: impl Iterator<Item = &'a String>, prefix: &str) -> Result<String> {
    let matches: Vec<_> = ids.filter(|id| id.starts_with(prefix)).collect();
    match matches.as_slice() {
        [id] => Ok((*id).clone()),
        [] => bail!("nothing matches {prefix}"),
        _ => bail!("{prefix} is ambiguous"),
    }
}

fn print_shelf(snap: &Snapshot) {
    println!("--- {} | devices:", snap.me.name);
    for d in &snap.devices {
        let state = match (d.paired, d.online) {
            (true, true) => "paired, online",
            (true, false) => "paired, offline",
            (false, _) => "nearby",
        };
        println!("    {} [{}] ({state})", d.info.name, &d.info.id[..8]);
    }
    if let Some(p) = &snap.pairing {
        println!("  PAIRING CODE for {}: {}", p.device.name, p.code);
    }
    for item in &snap.items {
        let owner = if item.owner == snap.me.id { "me" } else { &item.owner[..8] };
        let what = match &item.kind {
            ItemKind::Text { text } => text.clone(),
            ItemKind::File { name, size, .. } => format!("file {name} ({size} bytes)"),
        };
        let progress = snap.transfers.get(&item.id).map(|t| format!(" {}/{}", t.received, t.total));
        println!("  [{}] ({owner}) {what}{}", &item.id[..8], progress.unwrap_or_default());
    }
}
