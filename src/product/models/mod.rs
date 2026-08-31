pub mod human_gate;
pub mod lifecycle;
pub mod outline;
pub mod plan_repair;
pub mod project;
pub mod provider;
pub mod verification;
pub mod work_item_revision;
pub mod workspace;
pub mod workspace_link;

fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    <Option<T> as serde::Deserialize>::deserialize(deserializer)
}

#[cfg(test)]
pub mod tests;

pub use human_gate::*;
pub use lifecycle::*;
pub use outline::*;
pub use plan_repair::*;
pub use project::*;
pub use provider::*;
pub use verification::*;
pub use work_item_revision::*;
pub use workspace::*;
pub use workspace_link::*;
