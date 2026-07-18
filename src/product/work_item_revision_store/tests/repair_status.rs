use super::*;

#[test]
fn plan_repair_store_awaiting_transition_is_forward_only_and_idempotent() {
    let (_temp, store, plan) = test_store_and_plan();
    let request = repair_request("plan_repair_request_awaiting");
    store.put_repair_request(&plan, &request).unwrap();
    store
        .update_repair_request_status(&plan, &request.id, PlanRepairRequestStatus::InProgress)
        .unwrap();

    let awaiting = store
        .transition_repair_request_to_awaiting_confirmation(&plan, &request.id)
        .unwrap();
    assert_eq!(
        awaiting.status,
        PlanRepairRequestStatus::AwaitingConfirmation
    );
    assert_eq!(
        store
            .transition_repair_request_to_awaiting_confirmation(&plan, &request.id)
            .unwrap()
            .status,
        PlanRepairRequestStatus::AwaitingConfirmation
    );

    for (index, status) in [
        PlanRepairRequestStatus::Open,
        PlanRepairRequestStatus::Published,
        PlanRepairRequestStatus::Applied,
        PlanRepairRequestStatus::Cancelled,
        PlanRepairRequestStatus::Failed,
    ]
    .into_iter()
    .enumerate()
    {
        let request = repair_request(&format!("plan_repair_request_reject_{index}"));
        store.put_repair_request(&plan, &request).unwrap();
        store
            .update_repair_request_status(&plan, &request.id, status.clone())
            .unwrap();

        let error = store
            .transition_repair_request_to_awaiting_confirmation(&plan, &request.id)
            .unwrap_err();

        assert!(matches!(error, ProductStoreError::IdentityMismatch { .. }));
        assert_eq!(
            store.get_repair_request(&plan, &request.id).unwrap().status,
            status
        );
    }
}

#[test]
fn plan_repair_store_confirm_guard_accepts_only_awaiting_confirmation() {
    let (_temp, store, plan) = test_store_and_plan();
    for (index, status) in [
        PlanRepairRequestStatus::Open,
        PlanRepairRequestStatus::InProgress,
        PlanRepairRequestStatus::AwaitingConfirmation,
        PlanRepairRequestStatus::Published,
        PlanRepairRequestStatus::Applied,
        PlanRepairRequestStatus::Cancelled,
        PlanRepairRequestStatus::Failed,
    ]
    .into_iter()
    .enumerate()
    {
        let request = repair_request(&format!("plan_repair_request_confirm_{index}"));
        store.put_repair_request(&plan, &request).unwrap();
        store
            .update_repair_request_status(&plan, &request.id, status.clone())
            .unwrap();

        let result = store.confirm_repair_request_awaiting_confirmation(&plan, &request.id);

        if status == PlanRepairRequestStatus::AwaitingConfirmation {
            assert_eq!(result.unwrap().status, status);
        } else {
            assert!(matches!(
                result.unwrap_err(),
                ProductStoreError::IdentityMismatch { .. }
            ));
        }
        assert_eq!(
            store.get_repair_request(&plan, &request.id).unwrap().status,
            status
        );
    }
}
