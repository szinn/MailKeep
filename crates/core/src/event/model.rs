//! Application-wide coarse change events. MK-19 introduces a single contentless
//! variant; more (jobs, system messages) can be added later without changing
//! the transport.

/// A coarse "something changed" signal broadcast to connected clients. Carries
/// no payload — clients re-fetch the relevant query, which enforces its own
/// per-user scoping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvent {
    /// The account set changed: a status transition or a list-membership change
    /// (create / enable / disable / delete).
    AccountsChanged,
}
