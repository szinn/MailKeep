//! Integration tests for the `mk-imap` adapter against a live greenmail IMAP
//! server (TLS).
//!
//! Requires a greenmail container reachable on localhost (IMAPS 3993). The
//! `GREENMAIL_OPTS` user below must match `USERNAME`/`PASSWORD` in this file:
//!
//! ```text
//! docker run -d --name greenmail -p 3993:3993 -p 3143:3143 \
//!   -e GREENMAIL_OPTS='-Dgreenmail.setup.test.all -Dgreenmail.users=alice:pw@example.com' \
//!   greenmail/standalone:2.1.0
//! ```
//!
//! This module is compiled only under the `greenmail` feature (see
//! `main.rs`), so the default `sqlite` integration run never touches it. The
//! tests are additionally `#[ignore]`d so an `--all-features` build (e.g.
//! `just insta`) *compiles* but does not *run* them without a server. Run with:
//! `just imap-integration-tests` (which adds `--run-ignored all`).

use std::sync::Arc;

use mk_core::imap::{ImapCredentials, ImapPort, ImapServerConfig, TlsMode};
use mk_imap::ImapAdapter;
use secrecy::SecretString;

const HOST: &str = "127.0.0.1";
const IMAPS_PORT: u16 = 3993;
const USERNAME: &str = "alice";
const PASSWORD: &str = "pw";

/// Builds an adapter whose rustls config trusts ANY server certificate.
/// TEST-ONLY — greenmail ships a self-signed cert. Never used in production
/// wiring (`ImapAdapter::new`).
///
/// Mirrors `connect::production_client_config`'s provider handling: we pin the
/// `ring` provider explicitly via `builder_with_provider` so the test never
/// depends on a process-level default `CryptoProvider` being installed.
fn insecure_adapter() -> ImapAdapter {
    let config = rustls::ClientConfig::builder_with_provider(rustls::crypto::ring::default_provider().into())
        .with_safe_default_protocol_versions()
        .expect("ring provider supports the default protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(danger::NoVerify))
        .with_no_client_auth();
    ImapAdapter::with_tls_config(Arc::new(config))
}

fn server() -> ImapServerConfig {
    ImapServerConfig {
        host: HOST.into(),
        port: IMAPS_PORT,
        tls: TlsMode::Tls,
    }
}

fn creds(password: &str) -> ImapCredentials {
    ImapCredentials {
        username: USERNAME.into(),
        password: SecretString::from(password),
    }
}

#[tokio::test]
#[ignore = "requires a live greenmail container — run via `just imap-integration-tests`"]
async fn test_connection_succeeds_with_valid_creds() {
    let adapter = insecure_adapter();
    adapter.test_connection(&server(), &creds(PASSWORD)).await.expect("valid creds should connect");
}

#[tokio::test]
#[ignore = "requires a live greenmail container — run via `just imap-integration-tests`"]
async fn test_connection_rejects_bad_creds() {
    let adapter = insecure_adapter();
    let err = adapter.test_connection(&server(), &creds("WRONG")).await.unwrap_err();
    assert!(matches!(err, mk_core::Error::Validation(_)), "got {err:?}");
}

#[tokio::test]
#[ignore = "requires a live greenmail container — run via `just imap-integration-tests`"]
async fn list_folders_returns_inbox() {
    let adapter = insecure_adapter();
    let folders = adapter.list_folders(&server(), &creds(PASSWORD)).await.expect("list should succeed");
    assert!(
        folders.iter().any(|f| f.special_use == Some(mk_core::folder::SpecialUse::Inbox)),
        "expected an INBOX in {folders:?}"
    );
}

/// TEST-ONLY certificate verifier that accepts any server certificate. Lives in
/// a child module so the dangerous trait impl is clearly quarantined. Parameter
/// names match the `rustls::client::danger::ServerCertVerifier` trait (the
/// `renamed_function_params` clippy lint is on workspace-wide).
mod danger {
    use rustls::{
        DigitallySignedStruct,
        client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        pki_types::{CertificateDer, ServerName, UnixTime},
    };

    #[derive(Debug)]
    pub struct NoVerify;

    impl ServerCertVerifier for NoVerify {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            rustls::crypto::ring::default_provider().signature_verification_algorithms.supported_schemes()
        }
    }
}
