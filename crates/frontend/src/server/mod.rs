use std::sync::Arc;

use axum::{
    Extension,
    extract::DefaultBodyLimit,
    http::{HeaderName, Request},
};
use axum_session::{SessionConfig, SessionLayer, SessionStore};
use axum_session_auth::{AuthConfig, AuthSessionLayer};
use chrono::Duration;
use dioxus::server::DioxusRouterExt;
use mk_core::{CoreServices, user::UserId};
use tokio_graceful_shutdown::{IntoSubsystem, SubsystemHandle};
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    limit::RequestBodyLimitLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

use crate::{FrontendConfig, MailKeepFrontend, OidcConfig};

pub(crate) mod oidc;
pub(crate) mod session_pool;

pub(crate) use oidc::OidcClient;
pub(crate) use session_pool::{AuthSession, BackendSessionPool};

pub(crate) mod auth_user;
pub(crate) mod events;

pub(crate) use auth_user::AuthUser;

const REQUEST_ID_HEADER: &str = "x-request-id";
const DEFAULT_EXPIRATION_DURATION: Duration = Duration::days(7);
const MAX_REQUEST_BODY_SIZE: usize = 70 * 1024 * 1024; // 70 MiB

pub struct FrontendSubsystem {
    config: FrontendConfig,
    oidc_config: Option<OidcConfig>,
    core_services: Arc<CoreServices>,
}

impl IntoSubsystem<anyhow::Error> for FrontendSubsystem {
    async fn run(self, subsys: &mut SubsystemHandle) -> Result<(), anyhow::Error> {
        tracing::info!("FrontendSubsystem starting...");

        let core_services = self.core_services.clone();
        let backend_pool = BackendSessionPool::new(core_services.clone());
        let session_config = SessionConfig::default().with_lifetime(DEFAULT_EXPIRATION_DURATION);
        let auth_config = AuthConfig::<UserId>::default();

        let x_request_id = HeaderName::from_static(REQUEST_ID_HEADER);
        let session_store = SessionStore::<BackendSessionPool>::new(Some(backend_pool.clone()), session_config).await?;

        let middleware = ServiceBuilder::new()
            .layer(CompressionLayer::new())
            .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_SIZE))
            .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BODY_SIZE))
            .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
            .layer(TraceLayer::new_for_http().make_span_with(|request: &Request<_>| {
                let request_id = request
                    .headers()
                    .get(REQUEST_ID_HEADER)
                    .map(|v| v.to_str().unwrap_or_default())
                    .unwrap_or_default();

                tracing::trace_span!(
                    "",
                    request_id = ?request_id,
                )
            }))
            .layer(PropagateRequestIdLayer::new(x_request_id))
            .layer(SessionLayer::new(session_store))
            .layer(AuthSessionLayer::<AuthUser, UserId, BackendSessionPool, BackendSessionPool>::new(Some(backend_pool)).with_config(auth_config));

        let frontend_config = Arc::new(self.config.clone());

        // Build the OIDC client up-front if SSO is fully configured. Missing /
        // partial config is logged inside `is_sso_available()`; discovery
        // failures here are logged and treated as "SSO disabled" so a bad IdP
        // does not block server startup.
        let oidc_client: Option<Arc<OidcClient>> = match self.oidc_config.as_ref() {
            Some(cfg) if cfg.is_sso_available() => match OidcClient::new(cfg, &self.config.base_url).await {
                Ok(client) => Some(Arc::new(client)),
                Err(e) => {
                    tracing::error!(error = %e, "OIDC client init failed; SSO disabled");
                    None
                }
            },
            _ => None,
        };

        let mut app_router = axum::Router::new().serve_dioxus_application(dioxus_server::ServeConfig::new(), MailKeepFrontend);

        // When SSO is configured, merge the OIDC router and expose the client
        // and config to handlers / server fns. `oidc_client` is `Some` exactly
        // when `oidc_config.is_set()` was true above, so unwrapping the cloned
        // config here is safe by construction.
        if let Some(client) = oidc_client {
            let cfg = self.oidc_config.clone().expect("oidc_config is Some when oidc_client was built");
            app_router = app_router.merge(oidc::oidc_router()).layer(Extension(client)).layer(Extension(Arc::new(cfg)));
        }

        let app_router = app_router
            .merge(events::events_router())
            .layer(Extension(core_services))
            .layer(Extension(frontend_config))
            .layer(middleware);

        let health_handler = || async { axum::http::StatusCode::OK };
        let router = axum::Router::new()
            .route("/healthz", axum::routing::get(health_handler))
            .route("/readyz", axum::routing::get(health_handler))
            .merge(app_router);

        let ip = std::env::var("IP").ok().unwrap_or_else(|| self.config.listen_ip.clone());
        let port: u16 = std::env::var("PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(self.config.listen_port);
        let listener = tokio::net::TcpListener::bind(&format!("{ip}:{port}")).await?;

        tracing::info!("Frontend listening on {}", listener.local_addr()?);

        tokio::select! {
            () = subsys.on_shutdown_requested() => {
                tracing::info!("Frontend shutting down...");
            }
            result = axum::serve(listener, router) => {
                if let Err(e) = result {
                    tracing::error!("Frontend server error: {}", e);
                }
                subsys.request_shutdown();
            }
        }

        Ok(())
    }
}

#[must_use]
pub fn create_frontend_subsystem(config: &FrontendConfig, oidc_config: Option<OidcConfig>, core_services: Arc<CoreServices>) -> FrontendSubsystem {
    FrontendSubsystem {
        config: config.to_owned(),
        oidc_config,
        core_services,
    }
}
