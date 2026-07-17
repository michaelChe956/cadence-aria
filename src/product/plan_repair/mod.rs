mod fingerprint;
mod model;

pub use crate::product::models::{
    PlanDefectClass, PlanDefectEvidence, PlanDefectRoute, PlanRepairRequest,
    PlanRepairRequestStatus, RepairTarget, RepairTargetKind,
};
pub use fingerprint::plan_defect_fingerprint;
pub use model::*;

#[cfg(test)]
mod tests;
