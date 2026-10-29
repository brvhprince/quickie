//! Minimal HTTP/1.1 client over pinned TLS, for talking to one specific peer.

use std::net::SocketAddr;

use anyhow::{Result, bail};
use axum::body::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{
    Request, Response,
    body::Incoming,
    header::{CONTENT_TYPE, HOST},
};
use hyper_util::rt::TokioIo;
use serde::{Serialize, de::DeserializeOwned};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

use crate::{DeviceId, tls::Tls};

/// Tries each advertised address in turn.
pub(crate) async fn connect_any(
    tls: &Tls,
    peer: &DeviceId,
    addrs: &[SocketAddr],
) -> Result<(TlsStream<TcpStream>, SocketAddr)> {
    let mut last_err = None;
    for &addr in addrs {
        match tls.connect(addr, peer).await {
            Ok(stream) => return Ok((stream, addr)),
            Err(err) => last_err = Some(err.context(format!("connecting to {addr}"))),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("device has no known address")))
}

/// Sends one request on a fresh connection.
pub(crate) async fn send(
    stream: TlsStream<TcpStream>,
    req: Request<Full<Bytes>>,
) -> Result<Response<Incoming>> {
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
    tokio::spawn(async move {
        if let Err(err) = conn.await {
            tracing::debug!("peer http connection ended: {err}");
        }
    });
    Ok(sender.send_request(req).await?)
}

/// POSTs JSON and decodes a JSON reply. Error replies carry a human-readable message.
pub(crate) async fn post_json<B: Serialize, R: DeserializeOwned>(
    tls: &Tls,
    peer: &DeviceId,
    addrs: &[SocketAddr],
    path: &str,
    body: &B,
) -> Result<R> {
    let (stream, addr) = connect_any(tls, peer, addrs).await?;
    let req = Request::post(path)
        .header(HOST, addr.to_string())
        .header(CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(serde_json::to_vec(body)?)))?;
    let res = send(stream, req).await?;
    let status = res.status();
    let bytes = res.into_body().collect().await?.to_bytes();
    if !status.is_success() {
        bail!("{}", String::from_utf8_lossy(&bytes));
    }
    Ok(serde_json::from_slice(&bytes)?)
}
