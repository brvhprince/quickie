//! End-to-end: real nodes discovering each other over mDNS on this machine.

use std::{path::PathBuf, time::Duration};

use quickie_core::{Config, Node, Snapshot};

const WAIT: Duration = Duration::from_secs(10);

async fn start(name: &str) -> (Node, PathBuf) {
    let dir = std::env::temp_dir().join(format!("quickie-test-{name}-{}", uuid_like()));
    let config = Config { device_name: Some(name.into()), port: 0, ..Config::new(&dir) };
    (Node::start(config).await.expect("node starts"), dir)
}

fn uuid_like() -> String {
    format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())
}

/// Polls `node`'s snapshot until `check` passes.
async fn until(node: &Node, what: &str, check: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let snap = node.snapshot();
        if check(&snap) {
            return snap;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn pair_share_fetch_unmount() {
    let (a, dir_a) = start("alpha").await;
    let (b, dir_b) = start("beta").await;
    let (a_id, b_id) = (a.me().id.clone(), b.me().id.clone());

    until(&b, "beta to discover alpha", |s| s.devices.iter().any(|d| d.info.id == a_id)).await;
    until(&a, "alpha to discover beta", |s| s.devices.iter().any(|d| d.info.id == b_id)).await;

    // Unpaired devices never see each other's items.
    b.share_text("before pairing".into()).unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(a.snapshot().items.is_empty());

    b.request_pairing(&a_id).await.unwrap();
    let code = until(&a, "a pairing code", |s| s.pairing.is_some()).await.pairing.unwrap().code;
    let wrong = if code == "000000" { "111111" } else { "000000" };
    assert_eq!(b.pair(&a_id, wrong).await.unwrap_err().to_string(), "Wrong code");
    b.pair(&a_id, &code).await.unwrap();

    until(&a, "beta's item after pairing", |s| s.items.iter().any(|i| i.owner == b_id)).await;

    // Files are mounted by reference and fetched on demand, byte for byte.
    let original = dir_a.join("payload.bin");
    let bytes: Vec<u8> = (0..3_000_000u32).map(|i| (i * 7 % 251) as u8).collect();
    std::fs::write(&original, &bytes).unwrap();
    let item = a.mount_file(&original).unwrap();
    until(&b, "the file on beta's shelf", |s| s.items.iter().any(|i| i.id == item.id)).await;
    let fetched = b.fetch(&item.id).await.unwrap();
    assert_eq!(std::fs::read(&fetched).unwrap(), bytes);

    // Unmounting removes it everywhere, including beta's downloaded copy.
    a.unmount(&item.id).unwrap();
    until(&b, "the file to leave beta's shelf", |s| s.items.iter().all(|i| i.id != item.id)).await;
    assert!(!fetched.exists());

    a.shutdown();
    b.shutdown();
    let _ = std::fs::remove_dir_all(dir_a);
    let _ = std::fs::remove_dir_all(dir_b);
}

#[tokio::test(flavor = "multi_thread")]
async fn code_guessing_is_capped() {
    let (a, dir_a) = start("gamma").await;
    let (eve, dir_eve) = start("eve").await;
    let a_id = a.me().id.clone();
    until(&eve, "eve to discover gamma", |s| s.devices.iter().any(|d| d.info.id == a_id)).await;

    eve.request_pairing(&a_id).await.unwrap();
    let code = until(&a, "a pairing code", |s| s.pairing.is_some()).await.pairing.unwrap().code;
    let guesses = ["000000", "111111", "222222", "333333", "444444"].into_iter().filter(|g| *g != code);
    let errors: Vec<String> = {
        let mut out = Vec::new();
        for guess in guesses.take(4) {
            out.push(eve.pair(&a_id, guess).await.unwrap_err().to_string());
        }
        out
    };
    assert_eq!(errors[..3], ["Wrong code"; 3]);
    assert!(errors[3].contains("Too many wrong codes"), "{errors:?}");
    // Even the right code is useless now; a new request is needed.
    assert!(eve.pair(&a_id, &code).await.is_err());
    assert!(a.snapshot().pairing.is_none());

    a.shutdown();
    eve.shutdown();
    let _ = std::fs::remove_dir_all(dir_a);
    let _ = std::fs::remove_dir_all(dir_eve);
}
