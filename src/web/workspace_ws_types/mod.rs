//! WebSocket DTOs for the workspace protocol.
//!
//! Note on HTTP vs WS DTO boundaries: types in this module are optimized for the
//! WebSocket wire protocol (`WsOutMessage`/`WsInMessage`) and may evolve
//! independently from the HTTP REST DTOs in `src/web/types.rs`. When a field or
//! shape differs from its HTTP counterpart, it is intentional to keep the WS
//! contract stable while the REST API changes.

pub mod artifact;
pub mod artifact_version;
pub mod common;
pub mod in_;
pub mod out;
pub mod plan_candidate;
pub mod review;
pub mod stage;
pub mod timeline;

#[cfg(test)]
pub mod tests;

pub use artifact::*;
pub use artifact_version::*;
pub use common::*;
pub use in_::*;
pub use out::*;
pub use plan_candidate::*;
pub use review::*;
pub use stage::*;
pub use timeline::*;
