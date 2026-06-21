//! Integration tests for the `mk-imap` adapter against a greenmail IMAP server
//! over TLS.
//!
//! A greenmail container is started automatically via `testcontainers` on a
//! dynamically-mapped port and torn down when the test's `Greenmail` handle
//! drops — the only prerequisite is a running docker/colima daemon.
//!
//! Compiled only under the `greenmail` feature (see `main.rs`), so the default
//! `sqlite` run never touches it. The tests are also `#[ignore]`d so an
//! `--all-features` build (e.g. `just insta`) compiles but does not run them
//! without a daemon. Run with: `just imap-integration-tests` (`--run-ignored
//! all`).

use std::{sync::Arc, time::Duration};

use mk_core::{
    Error,
    imap::{ImapCredentials, ImapPort, ImapServerConfig, TlsMode},
};
use mk_imap::ImapAdapter;
use secrecy::SecretString;
use testcontainers::{ContainerAsync, GenericImage, ImageExt, core::IntoContainerPort, runners::AsyncRunner};

const USERNAME: &str = "alice";
const PASSWORD: &str = "pw";
/// greenmail's in-container IMAPS port; testcontainers maps it to a random host
/// port.
const IMAPS_PORT: u16 = 3993;

/// A running greenmail container plus an adapter pointed at its mapped IMAPS
/// port. Hold this for the duration of a test; dropping it tears greenmail
/// down.
struct Greenmail {
    _container: ContainerAsync<GenericImage>,
    adapter: ImapAdapter,
    server: ImapServerConfig,
}

impl Greenmail {
    /// Start greenmail with one preconfigured user and wait until it accepts
    /// logins. Panics with a clear message if the daemon is unavailable.
    async fn start() -> Self {
        let container = GenericImage::new("greenmail/standalone", "2.1.0")
            .with_exposed_port(IMAPS_PORT.tcp())
            .with_env_var(
                "GREENMAIL_OPTS",
                // hostname=0.0.0.0 is required: greenmail defaults to binding
                // 127.0.0.1 inside the container, which Docker's published port
                // can't reach (the handshake just EOFs).
                format!("-Dgreenmail.setup.test.all -Dgreenmail.hostname=0.0.0.0 -Dgreenmail.users={USERNAME}:{PASSWORD}@example.com -Dgreenmail.verbose"),
            )
            .start()
            .await
            .expect("greenmail container should start — is docker/colima running?");

        let host = container.get_host().await.expect("greenmail host").to_string();
        let port = container.get_host_port_ipv4(IMAPS_PORT.tcp()).await.expect("mapped IMAPS port");
        let server = ImapServerConfig { host, port, tls: TlsMode::Tls };
        let adapter = insecure_adapter();

        // greenmail's JVM binds the IMAPS port a few seconds after the container
        // starts. Poll the probe until it answers; retry only on connection
        // (Infrastructure) errors so a genuine auth/config problem fails fast.
        for attempt in 0..60 {
            match adapter.test_connection(&server, &creds(PASSWORD)).await {
                Ok(()) => break,
                Err(Error::Infrastructure(_)) if attempt < 59 => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Err(e) => panic!("greenmail is up but the probe failed (check user setup): {e:?}"),
            }
        }

        Self {
            _container: container,
            adapter,
            server,
        }
    }
}

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
    // Task 2: the harness only exercises connectivity/LIST, so nop sync services
    // suffice. MK-7 Task 7 will switch to `with_tls_config` with real services
    // for end-to-end sync assertions.
    ImapAdapter::probe_with_tls_config(Arc::new(config))
}

fn creds(password: &str) -> ImapCredentials {
    ImapCredentials {
        username: USERNAME.into(),
        password: SecretString::from(password),
    }
}

#[tokio::test]
#[ignore = "needs a docker/colima daemon — run via `just imap-integration-tests`"]
async fn test_connection_succeeds_with_valid_creds() {
    let gm = Greenmail::start().await;
    gm.adapter
        .test_connection(&gm.server, &creds(PASSWORD))
        .await
        .expect("valid creds should connect");
}

#[tokio::test]
#[ignore = "needs a docker/colima daemon — run via `just imap-integration-tests`"]
async fn test_connection_rejects_bad_creds() {
    let gm = Greenmail::start().await;
    let err = gm.adapter.test_connection(&gm.server, &creds("WRONG")).await.unwrap_err();
    assert!(matches!(err, Error::Validation(_)), "got {err:?}");
}

#[tokio::test]
#[ignore = "needs a docker/colima daemon — run via `just imap-integration-tests`"]
async fn list_folders_returns_inbox() {
    let gm = Greenmail::start().await;
    let folders = gm.adapter.list_folders(&gm.server, &creds(PASSWORD)).await.expect("list should succeed");
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
