use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::product::models::{PlanDefectClass, RepairTargetKind};

use super::PlanDefectFinding;

#[derive(Serialize)]
struct CanonicalPlanDefect<'a> {
    base_plan_revision_id: &'a str,
    defect_class: &'a PlanDefectClass,
    normalized_reason_code: String,
    repair_target: Option<CanonicalRepairTarget<'a>>,
    contract_refs: Vec<&'a str>,
    capability_refs: Vec<&'a str>,
}

#[derive(Serialize)]
struct CanonicalRepairTarget<'a> {
    kind: &'a RepairTargetKind,
    logical_work_item_ids: Vec<&'a str>,
    work_item_revision_ids: Vec<&'a str>,
}

pub fn plan_defect_fingerprint(base_plan_revision_id: &str, finding: &PlanDefectFinding) -> String {
    let repair_target = finding
        .repair_target
        .as_ref()
        .map(|target| CanonicalRepairTarget {
            kind: &target.kind,
            logical_work_item_ids: normalized_refs(&target.logical_work_item_ids),
            work_item_revision_ids: normalized_refs(&target.work_item_revision_ids),
        });
    let canonical = CanonicalPlanDefect {
        base_plan_revision_id,
        defect_class: &finding.defect_class,
        normalized_reason_code: finding.reason_code.trim().to_ascii_lowercase(),
        repair_target,
        contract_refs: normalized_refs(&finding.contract_refs),
        capability_refs: normalized_refs(&finding.capability_refs),
    };
    let bytes = serde_json::to_vec(&canonical).expect("canonical plan defect must serialize");
    hex::encode(Sha256::digest(bytes))
}

fn normalized_refs(values: &[String]) -> Vec<&str> {
    let mut normalized = values.iter().map(String::as_str).collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    normalized
}
