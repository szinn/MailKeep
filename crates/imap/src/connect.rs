//! TLS dialing + IMAP LOGIN. Builds a logged-in async-imap [`Session`] over a
//! tokio-rustls stream for both implicit TLS ([`TlsMode::Tls`]) and
//! opportunistic STARTTLS ([`TlsMode::StartTls`]).
//!
//! With async-imap's `runtime-tokio` feature the client speaks tokio's
//! `AsyncRead`/`AsyncWrite` directly, so the raw `TcpStream` and the
//! `tokio_rustls` `TlsStream` are usable without any futures-io compat shim.

use std::sync::Arc;

use async_imap::{Client, Session, error::Error as ImapError};
use mk_core::{
    Error,
    imap::{ImapCredentials, ImapServerConfig, TlsMode},
};
use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
use secrecy::ExposeSecret;
use tokio::net::TcpStream;
use tokio_rustls::{TlsConnector, client::TlsStream};

/// The logged-in session type this adapter operates on. Both TLS paths converge
/// here: implicit TLS wraps the socket up front, STARTTLS upgrades the same
/// `TcpStream` in place, so in either case the transport is a rustls client
/// stream over a tokio `TcpStream`.
pub(crate) type ImapSession = Session<TlsStream<TcpStream>>;

/// Production rustls config trusting the Mozilla webpki root store.
///
/// rustls 0.23 needs a crypto provider; we pin the `ring` provider explicitly
/// (matching what `sea-orm` already pulls into the workspace) so we never
/// depend on a process-level default provider being installed.
#[must_use]
pub fn production_client_config() -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let config = ClientConfig::builder_with_provider(rustls::crypto::ring::default_provider().into())
        .with_safe_default_protocol_versions()
        .expect("ring provider supports the default protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();

    Arc::new(config)
}

/// Connect to `server`, perform LOGIN with `creds`, and return the live
/// session.
///
/// Error mapping:
/// - an authentication rejection (`NO` response to LOGIN) →
///   [`Error::Validation`]
/// - everything else (DNS, TCP, TLS handshake, protocol, BAD) →
///   [`Error::Infrastructure`]
pub(crate) async fn connect_and_login(server: &ImapServerConfig, creds: &ImapCredentials, tls_config: Arc<ClientConfig>) -> Result<ImapSession, Error> {
    let client = match server.tls {
        TlsMode::Tls => connect_implicit_tls(server, tls_config).await?,
        TlsMode::StartTls => connect_starttls(server, tls_config).await?,
    };

    let session = client
        .login(&creds.username, creds.password.expose_secret())
        .await
        .map_err(|(err, _client)| map_login_error(&err))?;

    Ok(session)
}

/// Implicit TLS: wrap the socket immediately, then read the server greeting.
async fn connect_implicit_tls(server: &ImapServerConfig, tls_config: Arc<ClientConfig>) -> Result<Client<TlsStream<TcpStream>>, Error> {
    let tcp = dial_tcp(server).await?;
    let tls = tls_handshake(tcp, &server.host, tls_config).await?;

    let mut client = Client::new(tls);
    read_greeting(&mut client).await?;
    Ok(client)
}

/// STARTTLS: connect in plaintext, read the greeting, issue STARTTLS, then
/// upgrade the same TCP stream to TLS in place.
async fn connect_starttls(server: &ImapServerConfig, tls_config: Arc<ClientConfig>) -> Result<Client<TlsStream<TcpStream>>, Error> {
    let tcp = dial_tcp(server).await?;

    let mut plain = Client::new(tcp);
    read_greeting(&mut plain).await?;

    plain
        .run_command_and_check_ok("STARTTLS", None)
        .await
        .map_err(|e| Error::Infrastructure(format!("IMAP STARTTLS command failed: {e}")))?;

    // Recover the underlying TCP stream and upgrade it; STARTTLS does not emit a
    // second greeting, so we do not read one here.
    // MK-7: if/when we start trusting server CAPABILITY, re-issue it after the
    // TLS upgrade — the pre-STARTTLS capability list must not be trusted.
    let tcp = plain.into_inner();
    let tls = tls_handshake(tcp, &server.host, tls_config).await?;
    Ok(Client::new(tls))
}

/// Open a TCP connection to the configured host/port.
async fn dial_tcp(server: &ImapServerConfig) -> Result<TcpStream, Error> {
    TcpStream::connect((server.host.as_str(), server.port))
        .await
        .map_err(|e| Error::Infrastructure(format!("IMAP TCP connect to {}:{} failed: {e}", server.host, server.port)))
}

/// Perform the rustls handshake over `tcp`, validating `host` against the cert.
async fn tls_handshake(tcp: TcpStream, host: &str, tls_config: Arc<ClientConfig>) -> Result<TlsStream<TcpStream>, Error> {
    let server_name = ServerName::try_from(host.to_owned()).map_err(|e| Error::Infrastructure(format!("invalid IMAP server name {host:?}: {e}")))?;

    TlsConnector::from(tls_config)
        .connect(server_name, tcp)
        .await
        .map_err(|e| Error::Infrastructure(format!("IMAP TLS handshake with {host} failed: {e}")))
}

/// Consume the untagged server greeting (`* OK ...`) that follows connection.
async fn read_greeting<T>(client: &mut Client<T>) -> Result<(), Error>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    match client.read_response().await {
        Ok(Some(_greeting)) => Ok(()),
        Ok(None) => Err(Error::Infrastructure("IMAP server closed connection before greeting".to_string())),
        Err(e) => Err(Error::Infrastructure(format!("IMAP greeting read failed: {e}"))),
    }
}

/// Distinguish auth failure (server `NO` to LOGIN) from transport/protocol
/// faults.
fn map_login_error(err: &ImapError) -> Error {
    match err {
        ImapError::No(_) => Error::Validation("IMAP authentication failed".to_string()),
        other => Error::Infrastructure(format!("IMAP login failed: {other}")),
    }
}
