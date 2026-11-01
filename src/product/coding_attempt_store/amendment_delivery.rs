use chrono::Utc;

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::path::PathBuf;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::sync::{Mutex, OnceLock};

use crate::product::coding_models::{
    CodingPlanAmendmentDelivery, CodingPlanAmendmentDeliveryStatus,
};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

use super::locking::with_exclusive_lock;

#[cfg(test)]
pub(crate) struct PlanAmendmentDeliveryMarkFailpointGuard {
    path: PathBuf,
    registration_id: u64,
}

#[cfg(test)]
static DELIVERY_MARK_FAILPOINTS: OnceLock<Mutex<HashMap<PathBuf, u64>>> = OnceLock::new();
#[cfg(test)]
static NEXT_DELIVERY_MARK_FAILPOINT_ID: AtomicU64 = AtomicU64::new(1);

impl super::CodingAttemptStore {
    pub fn load_or_prepare_plan_amendment_delivery(
        &self,
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
        amendment_id: &str,
    ) -> Result<CodingPlanAmendmentDelivery, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        validate_relative_id(amendment_id)?;
        let path = self.amendment_event_delivery_path(
            &current.project_id,
            &current.issue_id,
            &current.id,
            amendment_id,
        );
        with_exclusive_lock(&path, || {
            if super::path_is_regular_file(&path)? {
                let delivery: CodingPlanAmendmentDelivery = read_json(&path)?;
                validate_delivery(&current.id, amendment_id, &delivery)?;
                return Ok(delivery);
            }
            let now = Utc::now().to_rfc3339();
            let delivery = CodingPlanAmendmentDelivery {
                id: delivery_id(&current.id, amendment_id),
                event_id: delivery_event_id(&current.id, amendment_id),
                attempt_id: current.id.clone(),
                amendment_id: amendment_id.to_string(),
                status: CodingPlanAmendmentDeliveryStatus::Pending,
                delivered_at: None,
                created_at: now.clone(),
                updated_at: now,
            };
            write_json(&path, &delivery)?;
            Ok(delivery)
        })
    }

    pub fn get_plan_amendment_delivery(
        &self,
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
        amendment_id: &str,
    ) -> Result<CodingPlanAmendmentDelivery, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        validate_relative_id(amendment_id)?;
        let path = self.amendment_event_delivery_path(
            &current.project_id,
            &current.issue_id,
            &current.id,
            amendment_id,
        );
        let delivery: CodingPlanAmendmentDelivery = read_json(&path)?;
        validate_delivery(&current.id, amendment_id, &delivery)?;
        Ok(delivery)
    }

    pub fn mark_plan_amendment_delivery_delivered(
        &self,
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
        amendment_id: &str,
        event_id: &str,
    ) -> Result<CodingPlanAmendmentDelivery, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        validate_relative_id(amendment_id)?;
        validate_relative_id(event_id)?;
        let path = self.amendment_event_delivery_path(
            &current.project_id,
            &current.issue_id,
            &current.id,
            amendment_id,
        );
        #[cfg(test)]
        maybe_fail_delivery_mark(&path)?;
        with_exclusive_lock(&path, || {
            let mut delivery: CodingPlanAmendmentDelivery = read_json(&path)?;
            validate_delivery(&current.id, amendment_id, &delivery)?;
            if delivery.event_id != event_id {
                return Err(identity_mismatch(amendment_id));
            }
            if delivery.status == CodingPlanAmendmentDeliveryStatus::Delivered {
                return Ok(delivery);
            }
            let now = Utc::now().to_rfc3339();
            delivery.status = CodingPlanAmendmentDeliveryStatus::Delivered;
            delivery.delivered_at = Some(now.clone());
            delivery.updated_at = now;
            write_json(&path, &delivery)?;
            Ok(delivery)
        })
    }

    /// 投递尝试失败收口（REQ-WIGA-06）：Pending|Unsent -> Unsent（幂等）；
    /// 已 Delivered 原样返回，绝不降级、绝不写 delivered_at。
    /// event_id 不匹配 => IdentityMismatch。
    pub fn mark_plan_amendment_delivery_unsent(
        &self,
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
        amendment_id: &str,
        event_id: &str,
    ) -> Result<CodingPlanAmendmentDelivery, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        validate_relative_id(amendment_id)?;
        validate_relative_id(event_id)?;
        let path = self.amendment_event_delivery_path(
            &current.project_id,
            &current.issue_id,
            &current.id,
            amendment_id,
        );
        with_exclusive_lock(&path, || {
            let mut delivery: CodingPlanAmendmentDelivery = read_json(&path)?;
            validate_delivery(&current.id, amendment_id, &delivery)?;
            if delivery.event_id != event_id {
                return Err(identity_mismatch(amendment_id));
            }
            if delivery.status == CodingPlanAmendmentDeliveryStatus::Delivered {
                return Ok(delivery);
            }
            let now = Utc::now().to_rfc3339();
            delivery.status = CodingPlanAmendmentDeliveryStatus::Unsent;
            delivery.delivered_at = None;
            delivery.updated_at = now;
            write_json(&path, &delivery)?;
            Ok(delivery)
        })
    }

    /// 列出该 attempt 目录下全部 delivery 记录（attempt 身份不符的文件 =>
    /// IdentityMismatch fail-closed；目录缺失 => 空表）；按 created_at、
    /// event_id 稳定排序。
    pub fn list_plan_amendment_deliveries(
        &self,
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
    ) -> Result<Vec<CodingPlanAmendmentDelivery>, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        let mut deliveries: Vec<CodingPlanAmendmentDelivery> = super::list_json_records(
            &self
                .attempt_dir(&current.project_id, &current.issue_id, &current.id)
                .join("amendment-event-deliveries"),
        )?;
        for delivery in &deliveries {
            validate_delivery(&current.id, &delivery.amendment_id, delivery)?;
        }
        deliveries.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.event_id.cmp(&right.event_id))
        });
        Ok(deliveries)
    }
}

#[cfg(test)]
pub(crate) fn register_plan_amendment_delivery_mark_failpoint(
    store: &super::CodingAttemptStore,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    amendment_id: &str,
) -> PlanAmendmentDeliveryMarkFailpointGuard {
    let path = store.amendment_event_delivery_path(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        amendment_id,
    );
    let registration_id = NEXT_DELIVERY_MARK_FAILPOINT_ID.fetch_add(1, Ordering::Relaxed);
    let mut failpoints = delivery_mark_failpoints()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        failpoints.insert(path.clone(), registration_id).is_none(),
        "delivery mark failpoint already registered"
    );
    PlanAmendmentDeliveryMarkFailpointGuard {
        path,
        registration_id,
    }
}

#[cfg(test)]
fn delivery_mark_failpoints() -> &'static Mutex<HashMap<PathBuf, u64>> {
    DELIVERY_MARK_FAILPOINTS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
fn maybe_fail_delivery_mark(path: &std::path::Path) -> Result<(), ProductStoreError> {
    if delivery_mark_failpoints()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains_key(path)
    {
        return Err(ProductStoreError::Io(
            "plan_amendment_delivery_mark_failpoint".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
impl Drop for PlanAmendmentDeliveryMarkFailpointGuard {
    fn drop(&mut self) {
        let mut failpoints = delivery_mark_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failpoints.get(&self.path) == Some(&self.registration_id) {
            failpoints.remove(&self.path);
        }
    }
}

fn validate_delivery(
    attempt_id: &str,
    amendment_id: &str,
    delivery: &CodingPlanAmendmentDelivery,
) -> Result<(), ProductStoreError> {
    for id in [
        delivery.id.as_str(),
        delivery.event_id.as_str(),
        delivery.attempt_id.as_str(),
        delivery.amendment_id.as_str(),
    ] {
        validate_relative_id(id)?;
    }
    if delivery.id != delivery_id(attempt_id, amendment_id)
        || delivery.event_id != delivery_event_id(attempt_id, amendment_id)
        || delivery.attempt_id != attempt_id
        || delivery.amendment_id != amendment_id
        || (delivery.delivered_at.is_some()
            != (delivery.status == CodingPlanAmendmentDeliveryStatus::Delivered))
    {
        return Err(identity_mismatch(amendment_id));
    }
    Ok(())
}

fn delivery_id(attempt_id: &str, amendment_id: &str) -> String {
    format!("coding_plan_amendment_delivery_{attempt_id}_{amendment_id}")
}

fn delivery_event_id(attempt_id: &str, amendment_id: &str) -> String {
    format!("coding_plan_amendment_updated_{attempt_id}_{amendment_id}")
}

fn identity_mismatch(id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind: "coding_plan_amendment_delivery",
        id: id.to_string(),
    }
}
