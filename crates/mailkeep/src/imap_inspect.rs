//! `mailkeep imap` — connect to an IMAP server and list its folders.
//!
//! A diagnostic command: it wires the `mk-imap` adapter into the core
//! `ImapAccountService` and calls `list_remote_folders`. No database, storage,
//! or encryption secret is involved.

use std::sync::Arc;

use anyhow::Context;
use mk_core::{
    folder::SpecialUse,
    imap::{ImapCredentials, ImapPort, ImapServerConfig, RemoteFolder, TlsMode},
};
use secrecy::SecretString;

use crate::commands::{ImapArgs, TlsArg};

/// Connect, authenticate (prompting for the password), and print the folder
/// list.
pub async fn run(args: ImapArgs) -> anyhow::Result<()> {
    if args.verbose {
        // Surface the adapter's `IMAP LIST entry` debug spans on stderr so the
        // raw server-reported folders/attributes are visible (keeps stdout = the
        // folder list). Ignore the error if a subscriber is already installed.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new("mk_imap=debug"))
            .with_writer(std::io::stderr)
            .with_target(false)
            .without_time()
            .try_init();
    }

    let tls = match args.tls {
        TlsArg::Implicit => TlsMode::Tls,
        TlsArg::Starttls => TlsMode::StartTls,
    };
    let tls_label = match tls {
        TlsMode::Tls => "implicit TLS",
        TlsMode::StartTls => "STARTTLS",
    };
    println!("Connecting to {}:{} ({tls_label}) as {} …", args.server, args.port, args.username);

    let password = rpassword::prompt_password("Password: ").context("reading password")?;

    let server = ImapServerConfig {
        host: args.server,
        port: args.port,
        tls,
    };
    let credentials = ImapCredentials {
        username: args.username,
        password: SecretString::from(password),
    };

    // Diagnostic path: call the port's folder-listing directly. The full
    // `ImapAccountService` now requires account/folder/cipher services that this
    // database-free command does not have.
    let imap_port: Arc<dyn ImapPort> = Arc::new(mk_imap::ImapAdapter::probe());

    let folders = match imap_port.list_folders(&server, &credentials).await {
        Ok(folders) => folders,
        Err(mk_core::Error::Validation(message)) => {
            anyhow::bail!("authentication failed ({message}) — Gmail and Fastmail require an app-specific password for IMAP");
        }
        Err(e) => return Err(anyhow::Error::new(e).context("listing folders")),
    };

    print!("{}", format_folders(&folders));
    Ok(())
}

/// Render the folder list: a count header, then one `path  <tag>` line per
/// folder. The tag is the special-use flag (e.g. `\Inbox`), or `(noselect)` for
/// container folders that hold no mail, or empty for an ordinary folder.
fn format_folders(folders: &[RemoteFolder]) -> String {
    use std::fmt::Write as _;

    let mut out = format!("{} folder{}:\n", folders.len(), if folders.len() == 1 { "" } else { "s" });
    let width = folders.iter().map(|f| f.path.chars().count()).max().unwrap_or(0);
    for folder in folders {
        let tag = folder
            .special_use
            .map_or_else(|| if folder.no_select { "(noselect)" } else { "" }, special_use_tag);
        // Writing to a String is infallible.
        if tag.is_empty() {
            let _ = writeln!(out, "  {}", folder.path);
        } else {
            let _ = writeln!(out, "  {:<width$}  {tag}", folder.path);
        }
    }
    out
}

/// The RFC 6154 / RFC 3501 attribute tag for a `SpecialUse`, for display.
fn special_use_tag(special_use: SpecialUse) -> &'static str {
    match special_use {
        SpecialUse::Inbox => "\\Inbox",
        SpecialUse::Sent => "\\Sent",
        SpecialUse::Drafts => "\\Drafts",
        SpecialUse::Trash => "\\Trash",
        SpecialUse::Archive => "\\Archive",
        SpecialUse::Junk => "\\Junk",
        SpecialUse::All => "\\All",
        // `SpecialUse` is #[non_exhaustive]; render any future variant generically.
        _ => "\\?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(path: &str, special_use: Option<SpecialUse>, no_select: bool) -> RemoteFolder {
        RemoteFolder {
            path: path.into(),
            special_use,
            has_children: false,
            no_select,
            delimiter: None,
        }
    }

    #[test]
    fn renders_special_use_plain_and_noselect() {
        let folders = vec![
            folder("INBOX", Some(SpecialUse::Inbox), false),
            folder("[Gmail]", None, true),
            folder("[Gmail]/All Mail", Some(SpecialUse::All), false),
            folder("Receipts", None, false),
        ];
        let out = format_folders(&folders);

        assert!(out.starts_with("4 folders:\n"), "header missing: {out}");

        let inbox = out.lines().find(|l| l.contains("INBOX")).unwrap();
        assert!(inbox.contains("\\Inbox"), "inbox tag: {inbox}");

        let noselect = out.lines().find(|l| l.contains("(noselect)")).unwrap();
        assert!(noselect.contains("[Gmail]") && !noselect.contains("All Mail"), "noselect line: {noselect}");

        let all_mail = out.lines().find(|l| l.contains("All Mail")).unwrap();
        assert!(all_mail.contains("\\All"), "all-mail tag: {all_mail}");

        let receipts = out.lines().find(|l| l.contains("Receipts")).unwrap();
        assert!(!receipts.contains('\\') && !receipts.contains("noselect"), "plain folder line: {receipts}");
    }

    #[test]
    fn singular_folder_count() {
        let out = format_folders(&[folder("INBOX", Some(SpecialUse::Inbox), false)]);
        assert!(out.starts_with("1 folder:\n"), "{out}");
    }

    #[test]
    fn empty_list() {
        assert_eq!(format_folders(&[]), "0 folders:\n");
    }
}
