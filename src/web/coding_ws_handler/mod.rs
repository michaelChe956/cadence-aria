mod context;
pub(crate) mod delivery_ack;
mod gates;
mod outbound;
pub mod protocol;
mod runner;
mod runner_support;
mod socket;
mod state;

#[cfg(test)]
mod tests;

pub use protocol::{CodingWsInMessage, CodingWsOutMessage};
pub use socket::{coding_ws, is_coding_ws_message_allowed, scoped_coding_ws};

pub(crate) use context::*;
pub(crate) use gates::*;
pub(crate) use protocol::coding_attempt_lookup_protocol_error;
pub(crate) use runner::*;
pub(crate) use state::*;

// Re-export types used by internal tests to keep test imports concise.
#[cfg(test)]
pub(crate) use crate::product::coding_models::CodingExecutionAttempt;
#[cfg(test)]
pub(crate) use crate::web::workspace_ws_types::ProviderConfigSnapshot;
#[cfg(test)]
pub(crate) use outbound::{OutboundEventReceiver, send_coding_event};
