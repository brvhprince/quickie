//! TLS 1.3 between devices, authenticated by pinned certificates instead of a CA.
//!
//! Servers accept any client certificate (proof of key possession is still
//! checked by the handshake) and decide per request whether that device is
//! paired. Clients only accept the exact device they meant to dial.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use rustls::{
    ClientConfig, DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{CryptoProvider, WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    server::danger::{ClientCertVerified, ClientCertVerifier},
    version::TLS13,
};
use tokio::{net::TcpStream, time::timeout};
use tokio_rustls::{TlsAcceptor, TlsConnector, client::TlsStream};

use crate::{DeviceId, identity::{Identity, device_id}};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct Tls {
    provider: Arc<CryptoProvider>,
    cert: CertificateDer<'static>,
    key: PrivatePkcs8KeyDer<'static>,
    acceptor: TlsAcceptor,
}

impl Tls {
    pub fn new(identity: &Identity) -> Result<Self> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let algs = provider.signature_verification_algorithms;
        let server = ServerConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&TLS13])?
            .with_client_cert_verifier(Arc::new(AnyDevice { algs }))
            .with_single_cert(vec![identity.cert.clone()], PrivateKeyDer::Pkcs8(identity.key.clone_key()))?;
        Ok(Self {
            provider,
            cert: identity.cert.clone(),
            key: identity.key.clone_key(),
            acceptor: TlsAcceptor::from(Arc::new(server)),
        })
    }

    pub fn acceptor(&self) -> TlsAcceptor {
        self.acceptor.clone()
    }

    /// Dials `addr`, succeeding only if the far end proves it is `expected`.
    pub async fn connect(&self, addr: SocketAddr, expected: &DeviceId) -> Result<TlsStream<TcpStream>> {
        let config = ClientConfig::builder_with_provider(self.provider.clone())
            .with_protocol_versions(&[&TLS13])?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedDevice {
                expected: expected.clone(),
                algs: self.provider.signature_verification_algorithms,
            }))
            .with_client_auth_cert(vec![self.cert.clone()], PrivateKeyDer::Pkcs8(self.key.clone_key()))?;
        let connect = async {
            let tcp = TcpStream::connect(addr).await?;
            let name = ServerName::IpAddress(addr.ip().into());
            Ok::<_, anyhow::Error>(TlsConnector::from(Arc::new(config)).connect(name, tcp).await?)
        };
        timeout(CONNECT_TIMEOUT, connect).await.context("timed out")?
    }
}

#[derive(Debug)]
struct PinnedDevice {
    expected: DeviceId,
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedDevice {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if device_id(end_entity) == self.expected {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("certificate does not belong to the expected device".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

#[derive(Debug)]
struct AnyDevice {
    algs: WebPkiSupportedAlgorithms,
}

impl ClientCertVerifier for AnyDevice {
    // Optional so that unpaired clients (and later the phone browser) can still
    // reach the public routes; paired-only routes check the certificate.
    fn client_auth_mandatory(&self) -> bool {
        false
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}
