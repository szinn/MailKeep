# Dioxus

The frontend uses Dioxus 0.7. Pre-0.7 APIs (`cx`, `Scope`, `use_state`) are gone; component props
take `ReadSignal<T>`. For general 0.7 API reference, use the `dioxus-modern` skill.

Hydration: the client's first render must match the server's — load data with
`use_server_future`, and put browser-only code (e.g. `localStorage`) in `use_effect`.

# MailKeep Server Function Conventions

- Use `#[get]` / `#[post]` / `#[put]` macros (not `#[server]`) for server functions
- Axum extensions are declared in the **macro attribute**, NOT in the function signature:

  ```rust
  #[get("/api/v1/foo", auth_session: axum::Extension<AuthSession>, core_services: axum::Extension<Arc<CoreServices>>)]
  ```

- Function parameters (request args) are declared normally on the function itself
- Server-side imports are gated:

  ```rust
  #[cfg(feature = "server")] use {crate::server::AuthSession, mk_core::CoreServices, std::sync::Arc};
  ```

- Request spans come from the `TraceLayer` in `server/mod.rs`; inside server functions, log with
  `tracing::error!` / `tracing::info!`

- `use_server_future(fn)?` returns `Resource<Result<T, E>>` after `?` unwraps the outer `RenderError`
- Without `?`, returns `Result<Resource<Result<T, E>>, RenderError>` — extract value with `match result { Ok(r) => r().and_then(...), Err(_) => None }`
- `?` suspends the entire component subtree up to the nearest `SuspenseBoundary`; use `SuspenseBoundary { fallback: |_| rsx! {}, ChildComponent {} }` to contain suspension to a subtree (e.g. keep NavBar visible while page content loads)
- **HTTP method rule**: `#[get]` only for zero-parameter fns (browser rejects GET with body).
  Parameterized fns use `#[put]` to create a resource and `#[post]` for everything else.
  Called with: `use_server_future(move || get_folder(token.clone()))?` (illustrative)
- `navigator.push(Route::Foo {})` returns `Option<_>` — must add `;` in onclick closures or compiler error
- `mk-core` is an optional dep in `mk-frontend`, gated by the `server` feature
