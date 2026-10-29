//! Pairing with a 6-digit code, Bluetooth style.
//!
//! 1. The initiator asks to pair (`/v1/pair/request`); the responder shows a code.
//! 2. The initiator's user types the code; both sides run SPAKE2 with it
//!    (`/v1/pair/start`). An eavesdropper learns nothing, and an active attacker
//!    gets one guess per run, so the responder caps runs at [`MAX_ATTEMPTS`].
//! 3. Both sides prove they derived the same key with an HMAC that also covers
//!    both device ids (`/v1/pair/confirm`). The ids are the TLS-verified ones, so a
//!    relay in the middle can't splice two pairings together.

use std::{collections::HashMap, fs, net::SocketAddr, path::Path, time::Duration};

use anyhow::{Context, Result, anyhow, ensure};
use axum::http::StatusCode;
use ring::{hmac, rand::SecureRandom};
use serde::{Deserialize, Serialize};
use spake2::{Ed25519Group, Identity, Password, Spake2};

use crate::{DeviceId, DeviceInfo, Node, http, now_ms};

pub(crate) const MAX_ATTEMPTS: u8 = 3;
const CODE_TTL: Duration = Duration::from_secs(120);
const SPAKE_ID: &[u8] = b"quickie-pair-v1";
const PAIRED_FILE: &str = "paired.json";

/// Responder-side state while a code is on screen.
pub(crate) struct Pending {
    pub peer: DeviceInfo,
    pub code: String,
    pub expires_at: u64,
    pub attempts: u8,
    pub key: Option<Vec<u8>>,
}

impl Pending {
    fn new(peer: DeviceInfo) -> Self {
        Self {
            peer,
            code: new_code(),
            expires_at: now_ms() + CODE_TTL.as_millis() as u64,
            attempts: 0,
            key: None,
        }
    }

    pub fn is_live(&self) -> bool {
        now_ms() < self.expires_at
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct RequestMsg {
    pub device: DeviceInfo,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct StartMsg {
    pub spake: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct StartReply {
    pub spake: String,
    pub confirm: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct ConfirmMsg {
    pub confirm: String,
}

#[derive(Debug)]
pub(crate) enum PairError {
    BadRequest,
    Busy,
    NotRequested,
    TooManyAttempts,
    WrongCode,
}

impl PairError {
    pub fn status(&self) -> (StatusCode, &'static str) {
        match self {
            Self::BadRequest => (StatusCode::BAD_REQUEST, "Malformed pairing message"),
            Self::Busy => (StatusCode::CONFLICT, "That device is pairing with someone else"),
            Self::NotRequested => (StatusCode::FORBIDDEN, "No pairing in progress; ask again"),
            Self::TooManyAttempts => (StatusCode::FORBIDDEN, "Too many wrong codes; ask again"),
            Self::WrongCode => (StatusCode::FORBIDDEN, "Wrong code"),
        }
    }
}

#[derive(Clone, Copy)]
enum Role {
    Responder,
    Initiator,
}

fn new_code() -> String {
    let mut bytes = [0u8; 4];
    ring::rand::SystemRandom::new().fill(&mut bytes).expect("system RNG failed");
    format!("{:06}", u32::from_le_bytes(bytes) % 1_000_000)
}

fn spake(code: &str) -> (Spake2<Ed25519Group>, Vec<u8>) {
    Spake2::<Ed25519Group>::start_symmetric(&Password::new(code.as_bytes()), &Identity::new(SPAKE_ID))
}

fn transcript(role: Role, responder: &str, initiator: &str) -> Vec<u8> {
    let role: &[u8] = match role {
        Role::Responder => b"responder",
        Role::Initiator => b"initiator",
    };
    [SPAKE_ID, b"|", role, b"|", responder.as_bytes(), b"|", initiator.as_bytes()].concat()
}

fn confirmation(key: &[u8], role: Role, responder: &str, initiator: &str) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    hex::encode(hmac::sign(&key, &transcript(role, responder, initiator)))
}

fn verify(key: &[u8], role: Role, responder: &str, initiator: &str, tag: &str) -> bool {
    let Ok(tag) = hex::decode(tag) else { return false };
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    hmac::verify(&key, &transcript(role, responder, initiator), &tag).is_ok()
}

pub(crate) fn load_paired(data_dir: &Path) -> Result<HashMap<DeviceId, DeviceInfo>> {
    let path = data_dir.join(PAIRED_FILE);
    let Ok(raw) = fs::read_to_string(&path) else { return Ok(HashMap::new()) };
    let list: Vec<DeviceInfo> =
        serde_json::from_str(&raw).with_context(|| format!("corrupt {}", path.display()))?;
    Ok(list.into_iter().map(|d| (d.id.clone(), d)).collect())
}

pub(crate) fn save_paired(data_dir: &Path, paired: &HashMap<DeviceId, DeviceInfo>) {
    let list: Vec<&DeviceInfo> = paired.values().collect();
    let result = serde_json::to_string_pretty(&list)
        .map_err(anyhow::Error::from)
        .and_then(|json| Ok(fs::write(data_dir.join(PAIRED_FILE), json)?));
    if let Err(err) = result {
        tracing::error!("could not save paired devices: {err:#}");
    }
}

/// Initiator side.
impl Node {
    /// Asks a nearby device to show a pairing code.
    pub async fn request_pairing(&self, device: &str) -> Result<()> {
        let (peer, addrs) = self.nearby(device)?;
        let msg = RequestMsg { device: self.me().clone() };
        http::post_json::<_, ()>(&self.inner.tls, &peer.id, &addrs, "/v1/pair/request", &msg).await
    }

    /// Completes pairing with the code shown on the other device.
    pub async fn pair(&self, device: &str, code: &str) -> Result<()> {
        let (peer, addrs) = self.nearby(device)?;
        let code: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
        ensure!(code.len() == 6, "The code is 6 digits");
        let me = &self.me().id;
        let tls = &self.inner.tls;

        let (spake, outbound) = spake(&code);
        let start = StartMsg { spake: hex::encode(outbound) };
        let reply: StartReply = http::post_json(tls, &peer.id, &addrs, "/v1/pair/start", &start).await?;
        let inbound = hex::decode(&reply.spake).context("malformed reply")?;
        let key = spake.finish(&inbound).map_err(|_| anyhow!("malformed reply"))?;
        ensure!(verify(&key, Role::Responder, &peer.id, me, &reply.confirm), "Wrong code");

        let confirm = ConfirmMsg { confirm: confirmation(&key, Role::Initiator, &peer.id, me) };
        http::post_json::<_, ()>(tls, &peer.id, &addrs, "/v1/pair/confirm", &confirm).await?;
        self.trust(peer);
        Ok(())
    }

    fn nearby(&self, device: &str) -> Result<(DeviceInfo, Vec<SocketAddr>)> {
        let state = self.state();
        let found = state.discovered.get(device).context("That device is no longer on the network")?;
        Ok((found.info.clone(), found.addrs.clone()))
    }
}

/// Responder side, driven by the server routes.
impl Node {
    pub(crate) fn pairing_requested(&self, caller: &DeviceId, device: DeviceInfo) -> Result<(), PairError> {
        if device.id != *caller {
            return Err(PairError::BadRequest);
        }
        {
            let mut state = self.state();
            if state.pairing.as_ref().is_some_and(|p| p.peer.id != *caller && p.is_live()) {
                return Err(PairError::Busy);
            }
            state.pairing = Some(Pending::new(device));
        }
        self.changed();
        Ok(())
    }

    pub(crate) fn pairing_start(&self, caller: &DeviceId, msg: StartMsg) -> Result<StartReply, PairError> {
        let me = self.me().id.clone();
        let mut state = self.state();
        let pending = state
            .pairing
            .as_mut()
            .filter(|p| p.peer.id == *caller && p.is_live())
            .ok_or(PairError::NotRequested)?;
        // Every run is one guess for an active attacker, so count runs, not confirmations.
        pending.attempts += 1;
        if pending.attempts > MAX_ATTEMPTS {
            state.pairing = None;
            drop(state);
            self.changed();
            return Err(PairError::TooManyAttempts);
        }
        let inbound = hex::decode(&msg.spake).map_err(|_| PairError::BadRequest)?;
        let (spake, outbound) = spake(&pending.code);
        let key = spake.finish(&inbound).map_err(|_| PairError::BadRequest)?;
        let confirm = confirmation(&key, Role::Responder, &me, caller);
        pending.key = Some(key);
        Ok(StartReply { spake: hex::encode(outbound), confirm })
    }

    pub(crate) fn pairing_confirm(&self, caller: &DeviceId, msg: ConfirmMsg) -> Result<(), PairError> {
        let me = self.me().id.clone();
        let peer = {
            let mut state = self.state();
            let pending = state
                .pairing
                .as_mut()
                .filter(|p| p.peer.id == *caller && p.is_live())
                .ok_or(PairError::NotRequested)?;
            let key = pending.key.take().ok_or(PairError::NotRequested)?;
            if !verify(&key, Role::Initiator, &me, caller, &msg.confirm) {
                return Err(PairError::WrongCode);
            }
            state.pairing.take().map(|p| p.peer).ok_or(PairError::NotRequested)?
        };
        self.trust(peer);
        Ok(())
    }
}
