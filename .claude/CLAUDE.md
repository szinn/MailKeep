# MailKeep: Archive Your IMAP Email

## Version Control

This is a **jj (jujutsu) repo**. Never use git commands (including `git worktree`).
Use only `jj` commands for all version control operations.

## Commands

- Build: `just build`
- Run: `just run`
- Format: `just fmt`
- Lint: `just clippy`
- Quick tests (component + postgres): `just quick-test`
- All tests: `just test`
- Component tests: `just component-tests`
- Integration tests: `just integration-tests`
- SQLite integration tests: `just sqlite-integration-tests`
- Insta tests: `just insta`
- Start colima (for integration/all tests): `colima start`
- Stop colima: `colima stop`

## Architecture

This project follows hexagonal (ports & adapters) architecture. Dependencies point inward
toward the core domain. Never introduce dependencies from `core` to outer crates.

```
crates/
├── core/               # Domain layer: business logic, domain models, and port traits (interfaces)
├── database/           # Adapter: implements persistence ports defined in core (SeaORM/Postgres)
├── frontend/           # Adapter: user interface, calls into core ports
├── utils/              # Shared utilities: hashing, token generation
├── mailkeep/           # Application entry point, wires adapters to ports
└── integration-tests/  # Integration tests
```

### Core Crate Organization

The core crate uses **domain-based modules** — each domain concept groups its model,
repository trait (port), and service together:

```
crates/core/src/
├── lib.rs              # CoreServices composition root, create_services()
├── error.rs            # Error, ErrorKind, RepositoryError
├── types.rs            # Shared newtypes (Email, Age) used across domains
├── repository.rs       # Shared infrastructure: Repository, Transaction traits,
│                       #   RepositoryService, and transaction macros
├── test_support.rs     # Mock implementations (behind "test-support" feature)
├── auth/               # Session auth: Session, AuthService, SessionRepository
└── user/               # Users and settings: User, UserService, UserSettingService
```

**Adding a new domain:** Create a new directory (e.g. `order/`) with `mod.rs`, `model.rs`,
`repository.rs`, and `service.rs`. Add re-exports in `mod.rs` and register the module in
`lib.rs`. Wire the new service into `CoreServices`.

**Import conventions:** Use flat re-exports from domain modules, not submodule paths:

- `use crate::user::{User, UserService, UserId}` (not `user::model::User`)
- `use crate::session::{Session, NewSession}` (not `session::model::Session`)
- `use crate::repository::{Repository, Transaction}` for shared infrastructure
- `use crate::types::{Email, Age}` for shared newtypes

### Subsystem Pattern (tokio-graceful-shutdown)

Each crate that owns background work exposes a `XxxSubsystem` struct + `create_xxx_subsystem()` factory
in its `lib.rs` — same pattern as `ApiSubsystem` in `bb-api`. The subsystem's `run()` starts its
child subsystems via `subsys.start(SubsystemBuilder::new(...))` then awaits `on_shutdown_requested()`.
`mailkeep/main.rs` stays clean: call the factories, pass results to `Toplevel`).

## Database

Persistence uses SeaORM with Postgres, MySQL, and SQLite support. See @.claude/Database.md for migration naming conventions, adapter patterns, and SQLite-specific notes.

## Frontend

The frontend is built using Dioxus. See @.claude/Dioxus.md for more info.

## Workflows

**Multi-step implementations:** Each logical step **MUST** be its own jj changeset. Before
starting a step, ensure the working copy is empty (`jj new` if needed). At the end of each
step, run the end-of-task routine below.

**After completing each task (end-of-task routine):**

These **MUST** be run as separate Bash commands. Do **NOT** join them into a single one with `&&`.

1. `just fmt-lint` — format code
2. `just component-tests` — verify tests pass
3. `jj desc -m "type(scope): description\n\nbody"` — update working copy description

### Workspaces

When creating a new workspace, run

```bash
direnv allow
mise trust
just tailwindcss
```

To verify the baseline state, run `just component-tests`.

## Testing

- Tests live alongside source code in `#[cfg(test)]` modules
- Colima manages docker containers required for integration testing

## Conventions

- **Commits:** Valid scopes: `api`, `cli`, `core`, `database`, `frontend` (match crate names)
- **Error handling:** `thiserror` for `core`, `api`, `database`; `anyhow` for `mailkeep` (binary)

## Insights

This project uses `.insights/` for research, triage docs, specs, plans, and personal notes
managed by the `insights` CLI.

**At the start of brainstorming, spec writing, or planning work**, dispatch the
`insights-locator` agent to check for prior context before proceeding. Use
`insights-analyzer` to read the most relevant documents. Use the `insights-research`
skill to orchestrate both and save a research document.

Directory layout:

- `.insights/issues/` — triage documents (BB-XX-triage-\*.md)
- `.insights/shared/specs/` — specs (BB-XX-spec-\*.md)
- `.insights/shared/plans/` — plans (BB-XX-plan-\*.md)
- `.insights/shared/research/` — research documents
- `.insights/scotte/` — personal notes
- `.insights/searchable/` — hardlink mirror for grep/search (read-only; strip "searchable/"
  from any path before reporting or editing)
  All `.insights/` artifact files must include YAML front-matter.
  See `.insights/shared/schema.md` for the full schema and vocabulary.
