use std::{fs, io::Write, path::Path};

use anyhow::{Context, Result};
use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

use crate::DeviceId;

/// This device's long-lived TLS identity. The device id is derived from the
/// certificate, so a peer that completes a handshake has proven it owns the id.
pub(crate) struct Identity {
    pub id: DeviceId,
    pub cert: CertificateDer<'static>,
    pub key: PrivatePkcs8KeyDer<'static>,
}

const CERT_FILE: &str = "device.crt.der";
const KEY_FILE: &str = "device.key.der";

pub(crate) fn load_or_create(data_dir: &Path) -> Result<Identity> {
    let (cert_path, key_path) = (data_dir.join(CERT_FILE), data_dir.join(KEY_FILE));
    if let (Ok(cert), Ok(key)) = (fs::read(&cert_path), fs::read(&key_path)) {
        let cert = CertificateDer::from(cert);
        return Ok(Identity { id: device_id(&cert), cert, key: PrivatePkcs8KeyDer::from(key) });
    }

    // P-256 rather than Ed25519: browsers (the phone web client) don't accept Ed25519 in TLS.
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
    // Validity defaults to effectively forever; trust is pinned to the key, not a CA.
    let cert = CertificateParams::new(vec!["quickie.local".to_string()])?.self_signed(&key)?;
    let (cert, key) = (cert.der().clone(), key.serialize_der());
    write_private(&key_path, &key).with_context(|| format!("writing {}", key_path.display()))?;
    fs::write(&cert_path, &cert).with_context(|| format!("writing {}", cert_path.display()))?;
    Ok(Identity { id: device_id(&cert), cert, key: PrivatePkcs8KeyDer::from(key) })
}

/// First 128 bits of SHA-256 over the certificate, hex encoded.
pub(crate) fn device_id(cert: &CertificateDer<'_>) -> DeviceId {
    let digest = ring::digest::digest(&ring::digest::SHA256, cert);
    hex::encode(&digest.as_ref()[..16])
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(bytes)
}

pub fn default_device_name() -> String {
    let host = gethostname::gethostname().to_string_lossy().into_owned();
    host.trim_end_matches(".local").to_string()
}
