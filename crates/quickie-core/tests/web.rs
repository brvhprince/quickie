//! The phone web client's API, exercised the way a browser would: HTTPS with an
//! accepted self-signed certificate, no client certificate, cookie auth.

use std::{path::PathBuf, sync::Arc};

use axum::body::Bytes;
use futures_util::StreamExt;
use http_body_util::{BodyExt, Full};
use hyper::{HeaderMap, Method, Request, StatusCode, header};
use hyper_util::rt::TokioIo;
use quickie_core::{Config, Node, WebSnapshot};
use rustls::{
    ClientConfig, DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use tokio::net::TcpStream;
use tokio_rustls::{TlsConnector, client::TlsStream};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

/// What a user does when they tap "visit this website anyway".
#[derive(Debug)]
struct AcceptAnyCert(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

async fn browser_tls(port: u16) -> TlsStream<TcpStream> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyCert(provider)))
        .with_no_client_auth();
    let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let name = ServerName::try_from("127.0.0.1").unwrap();
    TlsConnector::from(Arc::new(config)).connect(name, tcp).await.unwrap()
}

struct Browser {
    port: u16,
    cookie: Option<String>,
}

impl Browser {
    fn host(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    async fn send(&self, method: Method, path: &str, origin: Option<&str>, body: Vec<u8>) -> (StatusCode, HeaderMap, Bytes) {
        self.send_typed(method, path, origin, "application/json", body).await
    }

    async fn send_typed(
        &self,
        method: Method,
        path: &str,
        origin: Option<&str>,
        content_type: &str,
        body: Vec<u8>,
    ) -> (StatusCode, HeaderMap, Bytes) {
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake(TokioIo::new(browser_tls(self.port).await)).await.unwrap();
        tokio::spawn(conn);
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, self.host())
            .header(header::CONTENT_TYPE, content_type);
        if let Some(cookie) = &self.cookie {
            req = req.header(header::COOKIE, cookie);
        }
        if let Some(origin) = origin {
            req = req.header(header::ORIGIN, origin);
        }
        let res = sender.send_request(req.body(Full::new(Bytes::from(body))).unwrap()).await.unwrap();
        let (parts, body) = res.into_parts();
        (parts.status, parts.headers, body.collect().await.unwrap().to_bytes())
    }

    async fn get(&self, path: &str) -> (StatusCode, HeaderMap, Bytes) {
        self.send(Method::GET, path, None, vec![]).await
    }

    async fn post(&self, path: &str, body: impl Into<Vec<u8>>) -> (StatusCode, HeaderMap, Bytes) {
        let origin = format!("https://{}", self.host());
        self.send(Method::POST, path, Some(&origin), body.into()).await
    }

    async fn upload(&self, name: &str, body: Vec<u8>) -> (StatusCode, HeaderMap, Bytes) {
        let origin = format!("https://{}", self.host());
        let path = format!("/v1/web/files?name={name}");
        self.send_typed(Method::POST, &path, Some(&origin), "application/octet-stream", body).await
    }
}

async fn start() -> (Node, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "quickie-web-test-{:x}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let web_root = dir.join("web");
    std::fs::create_dir_all(&web_root).unwrap();
    std::fs::write(web_root.join("index.html"), "<!doctype html><title>Quickie</title>").unwrap();
    let config = Config { device_name: Some("host".into()), port: 0, web_root: Some(web_root), ..Config::new(&dir) };
    (Node::start(config).await.unwrap(), dir)
}

fn secret_of(url: &str) -> String {
    url.split("#pair=").nth(1).unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn phone_pairs_shares_and_downloads() {
    let (node, dir) = start().await;
    let mut phone = Browser { port: node.port(), cookie: None };

    // The app page is public, with a strict CSP; the API is not.
    let (status, headers, body) = phone.get("/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&body).contains("Quickie"));
    assert!(headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap().contains("default-src 'self'"));
    assert_eq!(phone.get("/v1/web/shelf").await.0, StatusCode::UNAUTHORIZED);

    // Wrong secret, then the QR code's secret.
    let invite = node.create_web_invite().unwrap();
    assert!(invite.urls[0].starts_with("https://"));
    let bad = phone.post("/v1/web/pair", r#"{"secret":"nope","name":"iPhone"}"#).await;
    assert_eq!(bad.0, StatusCode::FORBIDDEN);
    let pair = format!(r#"{{"secret":"{}","name":"iPhone"}}"#, secret_of(&invite.urls[0]));
    let (status, headers, _) = phone.post("/v1/web/pair", pair.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let set_cookie = headers[header::SET_COOKIE].to_str().unwrap().to_string();
    for flag in ["Secure", "HttpOnly", "SameSite=Strict"] {
        assert!(set_cookie.contains(flag), "{set_cookie}");
    }
    phone.cookie = Some(set_cookie.split(';').next().unwrap().to_string());
    assert!(node.snapshot().web_invite.is_none(), "invites are single-use");
    assert_eq!(Browser { port: node.port(), cookie: None }.post("/v1/web/pair", pair).await.0, StatusCode::FORBIDDEN);

    // The desktop lists the phone as a paired device.
    let phone_id = node.snapshot().devices.iter().find(|d| d.info.name == "iPhone").unwrap().info.id.clone();

    // Cross-site requests are refused even with the cookie.
    let evil = phone.send(Method::POST, "/v1/web/text", Some("https://evil.example"), br#"{"text":"x"}"#.to_vec()).await;
    assert_eq!(evil.0, StatusCode::FORBIDDEN);

    // Live updates over the WebSocket.
    let mut ws_req = format!("wss://{}/v1/web/events", phone.host()).into_client_request().unwrap();
    ws_req.headers_mut().insert(header::COOKIE, phone.cookie.clone().unwrap().parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::client_async(ws_req, browser_tls(node.port()).await).await.unwrap();
    let first: WebSnapshot = next_snapshot(&mut ws).await;
    assert_eq!(first.you.name, "iPhone");
    assert_eq!(first.host.name, "host");

    // Text and an upload from the phone land on the desktop's shelf, tagged with the phone.
    assert_eq!(phone.post("/v1/web/text", r#"{"text":"hi from phone"}"#).await.0, StatusCode::OK);
    let upload: Vec<u8> = (0..200_000u32).map(|i| (i % 253) as u8).collect();
    let (status, _, body) = phone.upload("..%2Fphoto.jpg", upload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let item: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(item["name"], "photo.jpg", "path components are stripped");
    assert_eq!(item["via"]["name"], "iPhone");
    let id = item["id"].as_str().unwrap().to_string();
    let snap = loop {
        let snap = next_snapshot(&mut ws).await;
        if snap.items.len() == 2 {
            break snap;
        }
    };
    assert!(snap.items.iter().all(|i| i.via.as_ref().is_some_and(|v| v.name == "iPhone")));

    // Downloads are attachments with a sandboxing CSP; previews only for safe types.
    let (status, headers, body) = phone.get(&format!("/v1/web/items/{id}/content")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_ref(), upload.as_slice());
    assert!(headers[header::CONTENT_DISPOSITION].to_str().unwrap().starts_with("attachment"));
    assert!(headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap().starts_with("sandbox"));
    let (_, headers, _) = phone.get(&format!("/v1/web/items/{id}/content?inline=1")).await;
    assert_eq!(headers[header::CONTENT_DISPOSITION], "inline");

    // A desktop-owned item can't be removed from the phone; its own upload can.
    let desktop_item = node.share_text("desktop note".into()).unwrap();
    let mine = format!("/v1/web/items/{id}");
    let not_mine = format!("/v1/web/items/{}", desktop_item.id);
    assert_eq!(phone.send(Method::DELETE, &not_mine, None, vec![]).await.0, StatusCode::FORBIDDEN);
    assert_eq!(phone.send(Method::DELETE, &mine, None, vec![]).await.0, StatusCode::NO_CONTENT);
    assert!(node.snapshot().items.iter().all(|i| i.id != id));

    // Unpairing from the desktop kicks the phone out.
    node.unpair(&phone_id).unwrap();
    assert_eq!(phone.get("/v1/web/shelf").await.0, StatusCode::UNAUTHORIZED);
    let closed = loop {
        match ws.next().await {
            Some(Ok(Message::Close(frame))) => break frame,
            Some(Ok(_)) => continue,
            other => panic!("expected a close frame, got {other:?}"),
        }
    };
    assert_eq!(u16::from(closed.unwrap().code), 4001);

    node.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

async fn next_snapshot<S>(ws: &mut tokio_tungstenite::WebSocketStream<S>) -> WebSnapshot
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => return serde_json::from_str(&text).unwrap(),
            Ok(Some(Ok(_))) => continue,
            other => panic!("expected a snapshot, got {other:?}"),
        }
    }
}
