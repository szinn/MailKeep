#[cfg(feature = "server")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(all(not(feature = "server"), feature = "web"))]
fn main() {
    mk_frontend::web::launch_web_frontend();
}

#[cfg(not(any(feature = "server", feature = "web")))]
fn main() {
    eprintln!("No feature selected. Build with --features server or --features web.");
}

#[cfg(feature = "server")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    use mailkeep::{
        commands::{CommandLine, Commands},
        config::Config,
        logging::init_logging,
    };

    let cli: CommandLine = clap::Parser::parse();

    match cli.command {
        Commands::Server => {
            let config = Config::load().context("Cannot load configuration")?;
            init_logging()?;

            cmd_server(config).await
        }
        Commands::Imap(args) => mailkeep::imap_inspect::run(args).await,
    }
}

#[cfg(feature = "server")]
async fn cmd_server(config: mailkeep::config::Config) -> anyhow::Result<()> {
    use std::time::Duration;

    use anyhow::Context;
    use mk_core::{ExternalServicesBuilder, create_core_subsystem, create_services};
    use mk_database::{create_repository_service, open_database};
    use mk_frontend::server::create_frontend_subsystem;
    use tokio_graceful_shutdown::{IntoSubsystem, SubsystemBuilder, SubsystemHandle, Toplevel};

    tracing::info!("MailKeep {}", clap::crate_version!());

    let span = tracing::span!(tracing::Level::TRACE, "MailKeep Startup").entered();

    let database_path = format!("sqlite:///{}/mailkeep.db?mode=rwc", config.metadata_path.to_string_lossy());
    let database = open_database(&database_path).await.context("Couldn't create database connection")?;
    let repository_service = create_repository_service(database).await.context("Couldn't create database connection")?;

    let cipher_service = mk_core::crypto::create_cipher_service(&config.encryption_secret);
    let storage_service = mk_storage::create_filesystem_storage(&config.storage_path, cipher_service.clone())
        .await
        .context("Couldn't initialize storage")?;

    let external = ExternalServicesBuilder::default()
        .repository_service(repository_service.clone())
        .cipher_service(cipher_service)
        .raw_storage_service(storage_service.raw_storage_service)
        .attachment_storage_service(storage_service.attachment_storage_service)
        .imap_port_factory(mk_imap::create_imap_port_factory())
        .job_concurrency(config.job_concurrency)
        .build()
        .context("ExternalServices missing required field")?;
    let core_services = create_services(external).context("Couldn't create core services")?;

    mk_parser::register_handlers(&core_services);

    let oidc_config = if config.oidc.is_set() { Some(config.oidc.clone()) } else { None };
    let frontend_subsystem = create_frontend_subsystem(&config.frontend, oidc_config, core_services.clone());

    let core_subsystem = create_core_subsystem(&core_services);

    span.exit();

    Toplevel::new(async |s: &mut SubsystemHandle| {
        s.start(SubsystemBuilder::new("Core", core_subsystem.into_subsystem()));
        s.start(SubsystemBuilder::new("Frontend", frontend_subsystem.into_subsystem()));
    })
    .catch_signals()
    .handle_shutdown_requests(Duration::from_secs(3))
    .await?;

    repository_service.repository().close().await.context("Couldn't close database")?;
    Ok(())
}
