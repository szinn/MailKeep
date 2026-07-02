//! Shared greenmail end-to-end test harness.
//!
//! Both `imap_sync.rs` (MK-7 sync-engine acceptance) and `account_add.rs` (MK-8
//! account-add orchestration) drive a real greenmail IMAP server over TLS. This
//! module owns the parts they share verbatim:
//!
//! - [`Greenmail`] container bring-up (mapped IMAPS port + readiness poll),
//! - [`insecure_tls`] — a TEST-ONLY rustls config trusting greenmail's
//!   self-signed cert,
//! - [`Control`] — an independent async-imap session used to seed/APPEND/create
//!   mailboxes,
//! - [`run_core`] — runs the core subsystem so the parser job worker drains
//!   `ParseMessageJob`s,
//! - DB/storage assertion helpers ([`list_messages`], [`wait_for_messages`],
//!   [`assert_ciphertext_on_disk`]),
//! - the shared constants and the quarantined [`danger`] cert verifier.
//!
//! Test-specific wiring (each file's `setup_pipeline`, fixture builders, and
//! scenario assertions) stays in the test modules.
//!
//! Compiled only under the `greenmail` feature (see `main.rs`).

use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use mk_core::{
    Error,
    account::AccountId,
    create_core_subsystem,
    imap::{ImapServerConfig, TlsMode},
    message::Message,
    repository::{RepositoryService, transaction},
};
use rustls::ClientConfig;
use testcontainers::{ContainerAsync, GenericImage, ImageExt, core::IntoContainerPort, runners::AsyncRunner};
use tokio::net::TcpStream;
use tokio_graceful_shutdown::{IntoSubsystem, SubsystemBuilder, SubsystemHandle, Toplevel};
use tokio_rustls::{TlsConnector, client::TlsStream};

use crate::context::TestContext;

pub(crate) const USERNAME: &str = "alice";
pub(crate) const PASSWORD: &str = "pw";
/// greenmail's in-container IMAPS port; testcontainers maps it to a random host
/// port.
pub(crate) const IMAPS_PORT: u16 = 3993;
pub(crate) const ACCOUNT_TIMEOUT: Duration = Duration::from_secs(20);

// ─── greenmail bring-up ──────────────────────────────────────────────────────

/// A running greenmail container plus its mapped IMAPS host/port. Dropping it
/// tears greenmail down. Exposes the raw connection coordinates so tests can
/// build both the system-under-test adapter and an independent control client.
pub(crate) struct Greenmail {
    _container: ContainerAsync<GenericImage>,
    pub(crate) host: String,
    pub(crate) port: u16,
}

impl Greenmail {
    pub(crate) async fn start() -> Self {
        let container = GenericImage::new("greenmail/standalone", "2.1.0")
            .with_exposed_port(IMAPS_PORT.tcp())
            .with_env_var(
                "GREENMAIL_OPTS",
                format!("-Dgreenmail.setup.test.all -Dgreenmail.hostname=0.0.0.0 -Dgreenmail.users={USERNAME}:{PASSWORD}@example.com -Dgreenmail.verbose"),
            )
            .start()
            .await
            .expect("greenmail container should start — is docker/colima running?");

        let host = container.get_host().await.expect("greenmail host").to_string();
        let port = container.get_host_port_ipv4(IMAPS_PORT.tcp()).await.expect("mapped IMAPS port");

        let gm = Self {
            _container: container,
            host,
            port,
        };

        // greenmail's JVM binds the IMAPS port a few seconds after the container
        // starts; poll a control login until it answers.
        for attempt in 0..60 {
            match Control::connect(&gm).await {
                Ok(mut control) => {
                    let _ = control.logout().await;
                    break;
                }
                Err(_) if attempt < 59 => tokio::time::sleep(Duration::from_millis(500)).await,
                Err(e) => panic!("greenmail is up but a control login failed (check user setup): {e:?}"),
            }
        }

        gm
    }

    pub(crate) fn server(&self) -> ImapServerConfig {
        ImapServerConfig {
            host: self.host.clone(),
            port: self.port,
            tls: TlsMode::Tls,
        }
    }
}

// ─── Test-only TLS that trusts greenmail's self-signed cert ──────────────────

/// Builds a rustls client config that trusts ANY server certificate. TEST-ONLY,
/// shared by both the system-under-test adapter and the control client.
pub(crate) fn insecure_tls() -> Arc<ClientConfig> {
    let config = rustls::ClientConfig::builder_with_provider(rustls::crypto::ring::default_provider().into())
        .with_safe_default_protocol_versions()
        .expect("ring provider supports the default protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(danger::NoVerify))
        .with_no_client_auth();
    Arc::new(config)
}

// ─── Independent control client (seed / APPEND / create mailbox) ─────────────

/// A direct async-imap session against greenmail used to *drive the server* in
/// tests: create mailboxes, seed messages, and APPEND new mail (to wake IDLE).
/// Entirely independent of the system-under-test pipeline (different
/// connection), so it never perturbs the engine's own sessions.
pub(crate) struct Control {
    session: async_imap::Session<TlsStream<TcpStream>>,
}

impl Control {
    pub(crate) async fn connect(gm: &Greenmail) -> Result<Self, Error> {
        let tcp = TcpStream::connect((gm.host.as_str(), gm.port))
            .await
            .map_err(|e| Error::Infrastructure(format!("control TCP connect failed: {e}")))?;
        let server_name = rustls::pki_types::ServerName::try_from(gm.host.clone()).map_err(|e| Error::Infrastructure(format!("invalid server name: {e}")))?;
        let tls = TlsConnector::from(insecure_tls())
            .connect(server_name, tcp)
            .await
            .map_err(|e| Error::Infrastructure(format!("control TLS handshake failed: {e}")))?;
        let mut client = async_imap::Client::new(tls);
        match client.read_response().await {
            Ok(Some(_greeting)) => {}
            Ok(None) => return Err(Error::Infrastructure("control: no greeting".into())),
            Err(e) => return Err(Error::Infrastructure(format!("control greeting failed: {e}"))),
        }
        let session = client
            .login(USERNAME, PASSWORD)
            .await
            .map_err(|(e, _)| Error::Infrastructure(format!("control login failed: {e}")))?;
        Ok(Self { session })
    }

    /// APPEND a minimal RFC-822 message with a unique Message-ID into
    /// `mailbox`.
    pub(crate) async fn append(&mut self, mailbox: &str, subject: &str) -> Result<(), Error> {
        let id = format!("{}@mailkeep.test", subject.replace(' ', "-"));
        let body = format!(
            "From: sender@example.com\r\nTo: alice@example.com\r\nSubject: {subject}\r\nMessage-ID: <{id}>\r\nDate: Mon, 1 Jan 2024 00:00:00 \
             +0000\r\n\r\nBody of {subject}.\r\n"
        );
        self.session
            .append(mailbox, None, None, body.as_bytes())
            .await
            .map_err(|e| Error::Infrastructure(format!("control APPEND to {mailbox} failed: {e}")))
    }

    /// CREATE a mailbox if it does not already exist. greenmail returns an
    /// error for an already-existing mailbox; either way the mailbox ends
    /// up present, which is all the caller needs, so we ignore the result.
    pub(crate) async fn ensure_mailbox(&mut self, mailbox: &str) {
        let _ = self.session.create(mailbox).await;
    }

    /// SELECT a mailbox and return its server UIDVALIDITY.
    pub(crate) async fn select_uidvalidity(&mut self, mailbox: &str) -> Result<u32, Error> {
        let mb = self
            .session
            .select(mailbox)
            .await
            .map_err(|e| Error::Infrastructure(format!("control SELECT {mailbox} failed: {e}")))?;
        mb.uid_validity
            .ok_or_else(|| Error::Infrastructure(format!("control: no UIDVALIDITY for {mailbox}")))
    }

    /// UID FETCH the FLAGS of every message in `mailbox` and report whether ANY
    /// message carries the `\Seen` flag. Uses `FETCH FLAGS` (never `BODY[]`),
    /// so the read itself cannot set `\Seen` — this observes the state the
    /// system-under-test left behind, without perturbing it.
    pub(crate) async fn any_seen(&mut self, mailbox: &str) -> Result<bool, Error> {
        use async_imap::types::Flag;
        self.session
            .select(mailbox)
            .await
            .map_err(|e| Error::Infrastructure(format!("control SELECT {mailbox} failed: {e}")))?;
        let mut stream = self
            .session
            .uid_fetch("1:*", "FLAGS")
            .await
            .map_err(|e| Error::Infrastructure(format!("control UID FETCH FLAGS failed: {e}")))?;
        let mut seen = false;
        while let Some(item) = stream.next().await {
            let fetch = item.map_err(|e| Error::Infrastructure(format!("control FETCH item error: {e}")))?;
            if fetch.flags().any(|f| f == Flag::Seen) {
                seen = true;
            }
        }
        Ok(seen)
    }

    pub(crate) async fn logout(&mut self) -> Result<(), Error> {
        self.session
            .logout()
            .await
            .map_err(|e| Error::Infrastructure(format!("control logout failed: {e}")))
    }
}

// ─── Core subsystem runner ───────────────────────────────────────────────────

/// Run the core subsystem so the parser job worker drains `ParseMessageJob`s.
/// Returns a handle to abort at the end of the test.
pub(crate) fn run_core(ctx: &TestContext) -> tokio::task::JoinHandle<()> {
    let core = ctx.services.clone();
    tokio::spawn(async move {
        Toplevel::new(async move |s: &mut SubsystemHandle| {
            s.start(SubsystemBuilder::new("Core", create_core_subsystem(&core).into_subsystem()));
        })
        .handle_shutdown_requests(Duration::from_secs(5))
        .await
        .unwrap();
    })
}

// ─── DB / storage assertion helpers ──────────────────────────────────────────

pub(crate) async fn list_messages(repos: &Arc<RepositoryService>, account_id: AccountId) -> Vec<Message> {
    transaction(&**repos.repository(), |tx| {
        let r = repos.message_repository().clone();
        Box::pin(async move { r.list_for_account(tx, account_id, 1000, 0).await })
    })
    .await
    .unwrap()
}

/// Poll until `account_id` has at least `n` Message rows, or panic after
/// `timeout`. Returns the rows.
pub(crate) async fn wait_for_messages(repos: &Arc<RepositoryService>, account_id: AccountId, n: usize, timeout: Duration) -> Vec<Message> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let msgs = list_messages(repos, account_id).await;
        if msgs.len() >= n {
            return msgs;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "expected ≥{n} message rows within {timeout:?}, got {}",
            msgs.len()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// On-disk ciphertext assertion: the raw blob for `content_hash` must exist in
/// storage (it was written encrypted) and decrypt back to non-empty bytes.
pub(crate) async fn assert_ciphertext_on_disk(ctx: &TestContext, account_id: AccountId, msg: &Message) {
    let exists = ctx.services.raw_storage_service.exists(account_id, &msg.content_hash).await.unwrap();
    assert!(exists, "encrypted raw blob must be on disk for message {}", msg.id);
    let back = ctx.services.raw_storage_service.get(account_id, &msg.content_hash).await.unwrap();
    assert!(!back.is_empty(), "decrypted raw blob must be non-empty for message {}", msg.id);
}

// ─── danger: test-only cert verifier ─────────────────────────────────────────

/// TEST-ONLY certificate verifier that accepts any server certificate
/// (greenmail ships a self-signed cert). Lives in a child module so the
/// dangerous trait impl is clearly quarantined. Parameter names match the
/// `rustls::client::danger::ServerCertVerifier` trait (the
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
