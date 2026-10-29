//! The phone web client. Browsers can't present a device certificate or serve
//! files, so a phone pairs by scanning a QR code (which trades a one-time secret
//! for a cookie session) and the desktop hosts whatever the phone shares.

use std::{
    collections::HashMap,
    fs,
    net::Ipv4Addr,
    path::{Path as FsPath, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{
        FromRequestParts, Path, Query, Request, State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::{
        HeaderValue, StatusCode,
        header::{
            CONTENT_DISPOSITION, CONTENT_SECURITY_POLICY, CONTENT_TYPE, COOKIE, HOST, ORIGIN,
            REFERRER_POLICY, SET_COOKIE, X_CONTENT_TYPE_OPTIONS,
        },
        request::Parts,
    },
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use futures_util::StreamExt;
use ring::rand::SecureRandom;
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncWriteExt, sync::broadcast::error::RecvError};
use tower::{ServiceBuilder, ServiceExt};
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
};

use crate::{
    DeviceStatus, Item, ItemKind, Node, WebInvite, WebSender, WebSnapshot, files, now_ms,
};

/// `DeviceInfo::platform` for paired browsers in the desktop's device list.
pub const WEB_PLATFORM: &str = "web";
const SESSION_COOKIE: &str = "quickie_session";
const INVITE_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_UPLOAD_BYTES: u64 = 8 << 30;
const MAX_NAME_CHARS: usize = 40;
const SESSIONS_FILE: &str = "web_sessions.json";
const PING_EVERY: Duration = Duration::from_secs(15);
/// Coalesces bursts (e.g. download progress) into one push.
const PUSH_DEBOUNCE: Duration = Duration::from_millis(100);

const APP_CSP: &str = "default-src 'self'; connect-src 'self'; img-src 'self' data: blob:; \
                       style-src 'self' 'unsafe-inline'; frame-ancestors 'none'";
/// Shared files are someone else's content: never let them run as our origin.
const CONTENT_CSP: &str = "sandbox; default-src 'none'; img-src 'self'; media-src 'self'; \
                           style-src 'unsafe-inline'";

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Session {
    pub id: String,
    pub name: String,
    /// SHA-256 of the cookie token; the token itself is never stored.
    token_hash: String,
    created_at: u64,
}

impl Session {
    pub fn sender(&self) -> WebSender {
        WebSender { id: self.id.clone(), name: self.name.clone() }
    }
}

pub(crate) struct Invite {
    secret: String,
    urls: Vec<String>,
    expires_at: u64,
}

impl Invite {
    pub fn view(&self) -> Option<WebInvite> {
        (now_ms() < self.expires_at)
            .then(|| WebInvite { urls: self.urls.clone(), expires_at: self.expires_at })
    }
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    ring::rand::SystemRandom::new().fill(&mut buf).expect("system RNG failed");
    hex::encode(buf)
}

fn hash_token(token: &str) -> String {
    hex::encode(ring::digest::digest(&ring::digest::SHA256, token.as_bytes()))
}

pub(crate) fn load_sessions(data_dir: &FsPath) -> Result<HashMap<String, Session>> {
    let path = data_dir.join(SESSIONS_FILE);
    let Ok(raw) = fs::read_to_string(&path) else { return Ok(HashMap::new()) };
    let list: Vec<Session> =
        serde_json::from_str(&raw).with_context(|| format!("corrupt {}", path.display()))?;
    Ok(list.into_iter().map(|s| (s.id.clone(), s)).collect())
}

fn save_sessions(data_dir: &FsPath, sessions: &HashMap<String, Session>) {
    let list: Vec<&Session> = sessions.values().collect();
    let result = serde_json::to_string_pretty(&list)
        .map_err(anyhow::Error::from)
        .and_then(|json| Ok(fs::write(data_dir.join(SESSIONS_FILE), json)?));
    if let Err(err) = result {
        tracing::error!("could not save web sessions: {err:#}");
    }
}

pub(crate) fn uploads_root(data_dir: &FsPath) -> PathBuf {
    data_dir.join("uploads")
}

/// Uploads only live as long as the items they back, which don't survive a restart.
pub(crate) fn reset_uploads(data_dir: &FsPath) {
    let _ = fs::remove_dir_all(uploads_root(data_dir));
}

/// This machine's IPv4 addresses a phone could reach, private LAN ranges first.
pub fn lan_addresses() -> Vec<Ipv4Addr> {
    let mut addrs: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|iface| iface.is_oper_up() && !iface.is_p2p)
        .filter_map(|iface| match iface.addr {
            if_addrs::IfAddr::V4(v4) => Some(v4.ip),
            _ => None,
        })
        .filter(|ip| !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified())
        .collect();
    addrs.sort_by_key(|ip| (!ip.is_private(), *ip));
    addrs.dedup();
    addrs
}

/// Desktop-side API.
impl Node {
    /// Opens a 5-minute, single-use invite for a phone to scan.
    pub fn create_web_invite(&self) -> Result<WebInvite> {
        let secret = random_hex(16);
        let port = self.inner.port;
        let urls: Vec<String> =
            lan_addresses().iter().map(|ip| format!("https://{ip}:{port}/#pair={secret}")).collect();
        ensure!(!urls.is_empty(), "This computer has no network address a phone could reach");
        let invite = Invite { secret, urls, expires_at: now_ms() + INVITE_TTL.as_millis() as u64 };
        let view = invite.view().expect("fresh invite is live");
        self.state().invite = Some(invite);
        self.changed();
        Ok(view)
    }

    pub fn cancel_web_invite(&self) {
        self.state().invite = None;
        self.changed();
    }

    /// Forgets a paired browser. Returns false if there was no such session.
    pub(crate) fn revoke_web_session(&self, id: &str) -> bool {
        {
            let mut state = self.state();
            if state.web_sessions.remove(id).is_none() {
                return false;
            }
            save_sessions(&self.inner.data_dir, &state.web_sessions);
        }
        self.changed();
        true
    }
}

/// Browser-side operations, driven by the routes below.
impl Node {
    fn redeem_invite(&self, secret: &str, name: &str) -> Option<(Session, String)> {
        let token = random_hex(32);
        let session = {
            let mut state = self.state();
            let invite = state.invite.as_ref().filter(|i| now_ms() < i.expires_at)?;
            // Comparing digests keeps the comparison's timing independent of the secret.
            if hash_token(&invite.secret) != hash_token(secret) {
                return None;
            }
            state.invite = None;
            let session = Session {
                id: random_hex(16),
                name: name.to_string(),
                token_hash: hash_token(&token),
                created_at: now_ms(),
            };
            state.web_sessions.insert(session.id.clone(), session.clone());
            save_sessions(&self.inner.data_dir, &state.web_sessions);
            session
        };
        self.changed();
        Some((session, token))
    }

    fn web_session(&self, token: &str) -> Option<WebSender> {
        let hash = hash_token(token);
        self.state().web_sessions.values().find(|s| s.token_hash == hash).map(Session::sender)
    }

    fn web_session_exists(&self, id: &str) -> bool {
        self.state().web_sessions.contains_key(id)
    }

    fn web_connected(&self, id: &str, connected: bool) {
        {
            let mut state = self.state();
            let count = state.web_online.entry(id.to_string()).or_default();
            *count = if connected { *count + 1 } else { count.saturating_sub(1) };
        }
        self.changed();
    }

    fn web_snapshot(&self, you: &WebSender) -> WebSnapshot {
        let full = self.snapshot();
        let host = DeviceStatus { info: full.me.clone(), paired: true, online: true };
        let devices = std::iter::once(host)
            .chain(full.devices.into_iter().filter(|d| d.paired && d.info.platform != WEB_PLATFORM))
            .collect();
        WebSnapshot { host: full.me, you: you.clone(), devices, items: full.items, transfers: full.transfers }
    }

    fn find_item(&self, id: &str) -> Option<Item> {
        let state = self.state();
        state.local.iter().chain(state.peers.values().flat_map(|p| &p.items)).find(|i| i.id == id).cloned()
    }

    /// Streams an upload to disk and mounts it on the phone's behalf.
    async fn upload_via<S>(&self, name: &str, via: WebSender, mut body: S) -> Result<Item>
    where
        S: futures_util::Stream<Item = Result<Bytes, axum::Error>> + Unpin,
    {
        let dir = uploads_root(&self.inner.data_dir).join(random_hex(8));
        tokio::fs::create_dir_all(&dir).await?;
        let path = dir.join(files::safe_file_name(name));
        let written: Result<()> = async {
            let mut file = tokio::fs::File::create(&path).await?;
            let mut size = 0u64;
            while let Some(chunk) = body.next().await {
                let chunk = chunk.context("upload interrupted")?;
                size += chunk.len() as u64;
                ensure!(size <= MAX_UPLOAD_BYTES, "Files over 8 GB aren't supported");
                file.write_all(&chunk).await?;
            }
            file.flush().await?;
            Ok(())
        }
        .await;
        if let Err(err) = written {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            return Err(err);
        }
        let kind = files::describe(&path)?;
        Ok(self.add_local(kind, Some(path), Some(via)))
    }

    /// Phones may unmount only what they shared themselves.
    fn unmount_via(&self, id: &str, session: &str) -> Result<()> {
        let theirs = self
            .state()
            .local
            .iter()
            .any(|i| i.id == id && i.via.as_ref().is_some_and(|v| v.id == session));
        ensure!(theirs, "You can only remove things you shared");
        self.unmount(id)
    }
}

pub(crate) fn routes(web_root: Option<PathBuf>) -> Router<Node> {
    let api = Router::new()
        .route("/v1/web/pair", post(pair))
        .route("/v1/web/logout", post(logout))
        .route("/v1/web/shelf", get(shelf))
        .route("/v1/web/events", get(events))
        .route("/v1/web/text", post(text))
        .route("/v1/web/files", post(upload))
        .route("/v1/web/items/{id}", delete(remove))
        .route("/v1/web/items/{id}/content", get(content));
    match web_root {
        Some(root) => api.fallback_service(
            ServiceBuilder::new()
                .layer(SetResponseHeaderLayer::overriding(CONTENT_SECURITY_POLICY, HeaderValue::from_static(APP_CSP)))
                .layer(SetResponseHeaderLayer::overriding(REFERRER_POLICY, HeaderValue::from_static("no-referrer")))
                .service(ServeDir::new(root)),
        ),
        None => api.fallback(|| async {
            (StatusCode::NOT_FOUND, "This build of Quickie doesn't include the phone web client.")
        }),
    }
}

type Rejection = (StatusCode, &'static str);

/// Refuses requests another site made a browser send (CSRF, cross-site WebSocket).
/// Same-site requests either carry a matching Origin or none at all.
struct SameOrigin;

impl<S: Send + Sync> FromRequestParts<S> for SameOrigin {
    type Rejection = Rejection;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let Some(origin) = parts.headers.get(ORIGIN) else { return Ok(SameOrigin) };
        let origin = origin.to_str().ok().and_then(|o| o.strip_prefix("https://"));
        let host = parts.headers.get(HOST).and_then(|h| h.to_str().ok());
        match (origin, host) {
            (Some(origin), Some(host)) if origin == host => Ok(SameOrigin),
            _ => Err((StatusCode::FORBIDDEN, "Cross-origin request refused")),
        }
    }
}

/// A paired browser, identified by its session cookie.
struct Browser(WebSender);

impl FromRequestParts<Node> for Browser {
    type Rejection = Rejection;

    async fn from_request_parts(parts: &mut Parts, node: &Node) -> Result<Self, Self::Rejection> {
        SameOrigin::from_request_parts(parts, node).await?;
        let token = parts
            .headers
            .get_all(COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(';'))
            .find_map(|c| c.trim().strip_prefix(SESSION_COOKIE)?.strip_prefix('='))
            .ok_or((StatusCode::UNAUTHORIZED, "Not paired"))?;
        node.web_session(token).map(Browser).ok_or((StatusCode::UNAUTHORIZED, "Not paired"))
    }
}

fn session_cookie(token: &str, max_age: u64) -> String {
    format!("{SESSION_COOKIE}={token}; Path=/; Max-Age={max_age}; Secure; HttpOnly; SameSite=Strict")
}

#[derive(Deserialize)]
struct PairBody {
    secret: String,
    name: String,
}

async fn pair(_: SameOrigin, State(node): State<Node>, Json(body): Json<PairBody>) -> Response {
    let name: String = body.name.trim().chars().take(MAX_NAME_CHARS).collect();
    let name = if name.is_empty() { "Phone".to_string() } else { name };
    match node.redeem_invite(&body.secret, &name) {
        Some((session, token)) => {
            let cookie = session_cookie(&token, 365 * 24 * 60 * 60);
            ([(SET_COOKIE, cookie)], Json(session.sender())).into_response()
        }
        None => (
            StatusCode::FORBIDDEN,
            "This QR code has expired or was already used. Make a new one on your computer.",
        )
            .into_response(),
    }
}

async fn logout(Browser(you): Browser, State(node): State<Node>) -> Response {
    node.revoke_web_session(&you.id);
    ([(SET_COOKIE, session_cookie("", 0))], StatusCode::NO_CONTENT).into_response()
}

async fn shelf(Browser(you): Browser, State(node): State<Node>) -> Json<WebSnapshot> {
    Json(node.web_snapshot(&you))
}

#[derive(Deserialize)]
struct TextBody {
    text: String,
}

async fn text(Browser(you): Browser, State(node): State<Node>, Json(body): Json<TextBody>) -> Response {
    match node.share_text_via(body.text, Some(you)) {
        Ok(item) => Json(item).into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, format!("{err:#}")).into_response(),
    }
}

#[derive(Deserialize)]
struct UploadQuery {
    name: String,
}

async fn upload(Browser(you): Browser, State(node): State<Node>, Query(q): Query<UploadQuery>, body: Body) -> Response {
    match node.upload_via(&q.name, you, body.into_data_stream()).await {
        Ok(item) => Json(item).into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, format!("{err:#}")).into_response(),
    }
}

async fn remove(Browser(you): Browser, State(node): State<Node>, Path(id): Path<String>) -> Response {
    match node.unmount_via(&id, &you.id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => (StatusCode::FORBIDDEN, format!("{err:#}")).into_response(),
    }
}

#[derive(Deserialize)]
struct ContentQuery {
    inline: Option<String>,
}

/// Serves any file on the shelf, fetching it from its owner first if it lives on
/// another device.
async fn content(
    _: Browser,
    State(node): State<Node>,
    Path(id): Path<String>,
    Query(q): Query<ContentQuery>,
    req: Request,
) -> Response {
    let Some(Item { kind: ItemKind::File { name, mime, .. }, .. }) = node.find_item(&id) else {
        return (StatusCode::NOT_FOUND, "That file is no longer on the shelf").into_response();
    };
    let path = match node.fetch(&id).await {
        Ok(path) => path,
        Err(err) => return (StatusCode::BAD_GATEWAY, format!("{err:#}")).into_response(),
    };
    let mut res = match ServeFile::new(path).oneshot(req).await {
        Ok(res) => res.into_response(),
        Err(err) => return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    };

    let inline = q.inline.is_some() && previewable(&mime);
    let headers = res.headers_mut();
    let disposition = if inline { "inline".to_string() } else { attachment(&name) };
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        headers.insert(CONTENT_DISPOSITION, value);
    }
    if inline && mime.starts_with("text/") {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    }
    headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(CONTENT_CSP));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    res
}

fn previewable(mime: &str) -> bool {
    ["image/", "video/", "audio/", "text/"].iter().any(|p| mime.starts_with(p)) || mime == "application/pdf"
}

/// `attachment; filename="ascii"; filename*=UTF-8''percent-encoded` (RFC 6266).
fn attachment(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' })
        .collect();
    let encoded: String = name
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

async fn events(Browser(you): Browser, ws: WebSocketUpgrade, State(node): State<Node>) -> Response {
    ws.on_upgrade(move |socket| push_shelf(socket, node, you))
}

/// Pushes a full snapshot whenever anything changes; they're small.
async fn push_shelf(mut socket: WebSocket, node: Node, you: WebSender) {
    let mut changes = node.subscribe();
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    node.web_connected(&you.id, true);
    'push: loop {
        if !node.web_session_exists(&you.id) {
            let frame = CloseFrame { code: 4001, reason: "unpaired".into() };
            let _ = socket.send(Message::Close(Some(frame))).await;
            break;
        }
        let Ok(json) = serde_json::to_string(&node.web_snapshot(&you)) else { break };
        if socket.send(Message::Text(json.into())).await.is_err() {
            break;
        }
        loop {
            tokio::select! {
                change = changes.recv() => match change {
                    Ok(()) | Err(RecvError::Lagged(_)) => {
                        tokio::time::sleep(PUSH_DEBOUNCE).await;
                        while changes.try_recv().is_ok() {}
                        continue 'push;
                    }
                    Err(RecvError::Closed) => break 'push,
                },
                _ = ping.tick() => if socket.send(Message::Ping(Bytes::new())).await.is_err() { break 'push },
                msg = socket.recv() => match msg {
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break 'push,
                    Some(Ok(_)) => {}
                },
            }
        }
    }
    node.web_connected(&you.id, false);
}

#[cfg(test)]
mod tests {
    use super::attachment;

    #[test]
    fn content_disposition_is_safe_and_unicode_aware() {
        assert_eq!(
            attachment("ré\"sumé.pdf"),
            "attachment; filename=\"r__sum_.pdf\"; filename*=UTF-8''r%C3%A9%22sum%C3%A9.pdf"
        );
    }
}
