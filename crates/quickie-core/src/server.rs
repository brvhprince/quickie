use std::{net::SocketAddr, time::Duration};

use anyhow::Result;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{
        ConnectInfo, FromRequestParts, Path, Request, State,
        connect_info::Connected,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
    routing::{get, post},
    serve::{IncomingStream, Listener},
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{broadcast::error::RecvError, mpsc},
};
use tokio_rustls::{TlsAcceptor, server::TlsStream};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::{
    DeviceId, DeviceInfo, Item, Node, WireEvent,
    identity::device_id,
    pairing::{ConfirmMsg, PairError, RequestMsg, StartMsg, StartReply},
};

const PING_EVERY: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Binds the preferred port so phone bookmarks stay valid, or any free port if taken.
pub(crate) async fn bind(port: u16) -> Result<TcpListener> {
    match TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))).await {
        Ok(listener) => Ok(listener),
        Err(err) => {
            tracing::warn!("port {port} unavailable ({err}), using a random port");
            Ok(TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], 0))).await?)
        }
    }
}

pub(crate) async fn serve(tcp: TcpListener, acceptor: TlsAcceptor, node: Node) {
    let app = Router::new()
        .route("/v1/hello", get(hello))
        .route("/v1/pair/request", post(pair_request))
        .route("/v1/pair/start", post(pair_start))
        .route("/v1/pair/confirm", post(pair_confirm))
        .route("/v1/items", get(items))
        .route("/v1/items/{id}/content", get(content))
        .route("/v1/events", get(events))
        .with_state(node);
    let listener = TlsListener::new(tcp, acceptor);
    let service = app.into_make_service_with_connect_info::<Remote>();
    if let Err(err) = axum::serve(listener, service).await {
        tracing::error!("server stopped: {err}");
    }
}

/// Who is on the other end of a connection, per its TLS client certificate.
#[derive(Clone, Debug)]
pub(crate) struct Remote {
    device: Option<DeviceId>,
}

impl Connected<IncomingStream<'_, TlsListener>> for Remote {
    fn connect_info(stream: IncomingStream<'_, TlsListener>) -> Self {
        stream.remote_addr().clone()
    }
}

/// Accepts TCP and runs TLS handshakes concurrently, so one slow client can't
/// stall everyone else.
pub(crate) struct TlsListener {
    ready: mpsc::Receiver<(TlsStream<TcpStream>, Remote)>,
}

impl TlsListener {
    fn new(tcp: TcpListener, acceptor: TlsAcceptor) -> Self {
        let (tx, ready) = mpsc::channel(32);
        tokio::spawn(async move {
            loop {
                let (stream, addr) = match tcp.accept().await {
                    Ok(conn) => conn,
                    Err(err) => {
                        tracing::warn!("accept failed: {err}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let (acceptor, tx) = (acceptor.clone(), tx.clone());
                tokio::spawn(async move {
                    match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await {
                        Ok(Ok(tls)) => {
                            let device = tls.get_ref().1.peer_certificates().and_then(|c| c.first()).map(device_id);
                            let _ = tx.send((tls, Remote { device })).await;
                        }
                        Ok(Err(err)) => tracing::debug!("tls handshake with {addr} failed: {err}"),
                        Err(_) => tracing::debug!("tls handshake with {addr} timed out"),
                    }
                });
            }
        });
        Self { ready }
    }
}

impl Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = Remote;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.ready.recv().await {
            Some(conn) => conn,
            // The accept loop never exits, so this is unreachable in practice.
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        Ok(Remote { device: None })
    }
}

/// Any device that presented a certificate.
struct Caller(DeviceId);

/// A device we have paired with.
struct Paired;

fn remote_device(parts: &Parts) -> Option<DeviceId> {
    parts.extensions.get::<ConnectInfo<Remote>>().and_then(|c| c.0.device.clone())
}

impl FromRequestParts<Node> for Caller {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _: &Node) -> Result<Self, Self::Rejection> {
        remote_device(parts)
            .map(Caller)
            .ok_or((StatusCode::UNAUTHORIZED, "A device certificate is required"))
    }
}

impl FromRequestParts<Node> for Paired {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, node: &Node) -> Result<Self, Self::Rejection> {
        match remote_device(parts) {
            Some(id) if node.is_paired(&id) => Ok(Paired),
            Some(_) => Err((StatusCode::FORBIDDEN, "Not paired with this device")),
            None => Err((StatusCode::UNAUTHORIZED, "A device certificate is required")),
        }
    }
}

impl IntoResponse for PairError {
    fn into_response(self) -> Response {
        self.status().into_response()
    }
}

async fn hello(State(node): State<Node>) -> Json<DeviceInfo> {
    Json(node.me().clone())
}

async fn pair_request(
    Caller(caller): Caller,
    State(node): State<Node>,
    Json(msg): Json<RequestMsg>,
) -> Result<Json<()>, PairError> {
    node.pairing_requested(&caller, msg.device).map(Json)
}

async fn pair_start(
    Caller(caller): Caller,
    State(node): State<Node>,
    Json(msg): Json<StartMsg>,
) -> Result<Json<StartReply>, PairError> {
    node.pairing_start(&caller, msg).map(Json)
}

async fn pair_confirm(
    Caller(caller): Caller,
    State(node): State<Node>,
    Json(msg): Json<ConfirmMsg>,
) -> Result<Json<()>, PairError> {
    node.pairing_confirm(&caller, msg).map(Json)
}

async fn items(_: Paired, State(node): State<Node>) -> Json<Vec<Item>> {
    Json(node.local_items())
}

/// Streams a mounted file, with Range support for resuming.
async fn content(_: Paired, State(node): State<Node>, Path(id): Path<String>, req: Request) -> Response {
    let Some(path) = node.local_path(&id) else {
        return (StatusCode::NOT_FOUND, "No such item").into_response();
    };
    match ServeFile::new(path).oneshot(req).await {
        Ok(res) => res.into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn events(_: Paired, ws: WebSocketUpgrade, State(node): State<Node>) -> Response {
    ws.on_upgrade(move |socket| stream_events(socket, node))
}

async fn stream_events(mut socket: WebSocket, node: Node) {
    let (snapshot, mut rx) = node.subscribe_wire();
    if send(&mut socket, &snapshot).await.is_err() {
        return;
    }
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    loop {
        tokio::select! {
            event = rx.recv() => match event {
                Ok(event) => if send(&mut socket, &event).await.is_err() { break },
                // Lagged: drop the connection; the peer reconnects and gets a fresh snapshot.
                Err(RecvError::Lagged(_) | RecvError::Closed) => break,
            },
            _ = ping.tick() => if socket.send(Message::Ping(Bytes::new())).await.is_err() { break },
            msg = socket.recv() => match msg {
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
        }
    }
}

async fn send(socket: &mut WebSocket, event: &WireEvent) -> Result<()> {
    let json = serde_json::to_string(event)?;
    socket.send(Message::Text(json.into())).await?;
    Ok(())
}
