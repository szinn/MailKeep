#[derive(Debug, clap::Parser)]
#[command(
    name = "MailKeep",
    help_template = r#"
{before-help}{name} {version} - {about}

{usage-heading} {usage}

{all-args}{after-help}

AUTHORS:
    {author}
"#,
    version,
    author
)]
#[command(about, long_about = None)]
#[command(propagate_version = true, arg_required_else_help = true)]
pub struct CommandLine {
    #[clap(subcommand)]
    pub command: Commands,
}

#[derive(Debug, clap::Subcommand)]
pub enum Commands {
    #[command(about = "Start server", display_order = 10)]
    Server,

    #[command(
        display_order = 20,
        about = "Inspect an IMAP server (connect + list folders)",
        long_about = "Connect to an IMAP server, authenticate, and list its folders with their special-use flags. Prompts for the password without echoing \
                      it.\n\nNote: Gmail and Fastmail require an app-specific password for IMAP."
    )]
    Imap(ImapArgs),
}

#[derive(Debug, clap::Args)]
pub struct ImapArgs {
    /// IMAP server hostname, e.g. imap.gmail.com
    pub server: String,
    /// Login username (often your full email address)
    pub username: String,
    /// Server port
    #[arg(long, default_value_t = 993)]
    pub port: u16,
    /// Connection security
    #[arg(long, value_enum, default_value_t = TlsArg::Implicit)]
    pub tls: TlsArg,
    /// Print each raw IMAP LIST entry (folder name + attributes) to stderr
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum TlsArg {
    /// Implicit TLS from the first byte (typically port 993)
    Implicit,
    /// Upgrade a plaintext connection via STARTTLS (typically port 143)
    Starttls,
}
