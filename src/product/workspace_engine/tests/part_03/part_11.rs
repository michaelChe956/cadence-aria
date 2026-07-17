#[derive(Debug, Clone, Copy)]
enum ProjectionPayloadHashCorruption {
    CanonicalContract,
    WorkItemHuman,
    WorkItemCoder,
    WorkItemReviewer,
    PlanHuman,
    PlanCoder,
    PlanReviewer,
}

#[tokio::test]
async fn work_item_plan_reviewer_prompt_rejects_projection_payload_hash_corruption() {
    let mut accepted = Vec::new();
    for corruption in [
        ProjectionPayloadHashCorruption::CanonicalContract,
        ProjectionPayloadHashCorruption::WorkItemHuman,
        ProjectionPayloadHashCorruption::WorkItemCoder,
        ProjectionPayloadHashCorruption::WorkItemReviewer,
        ProjectionPayloadHashCorruption::PlanHuman,
        ProjectionPayloadHashCorruption::PlanCoder,
        ProjectionPayloadHashCorruption::PlanReviewer,
    ] {
        let (_tmp, lifecycle, plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        let outcome = engine.run_work_item_plan_compile().await.unwrap();
        let plan_root = persisted_plan_review_context_root(&lifecycle, &plan_id);
        engine.session.artifact = Some(ArtifactPayload::WorkItemPlanProjection {
            projection: Box::new(outcome.plan_projection_bundle.clone()),
        });

        match corruption {
            ProjectionPayloadHashCorruption::CanonicalContract => {
                let mut revision = outcome.work_items[0].work_item_revision.clone();
                revision
                    .canonical_contract
                    .goal
                    .summary
                    .push_str(" tampered without hash update");
                overwrite_persisted_review_context_json(
                    plan_root
                        .join("logical-work-items")
                        .join(&revision.logical_work_item_id)
                        .join("revisions")
                        .join(format!("{}.json", revision.id)),
                    &revision,
                );
            }
            ProjectionPayloadHashCorruption::WorkItemHuman
            | ProjectionPayloadHashCorruption::WorkItemCoder
            | ProjectionPayloadHashCorruption::WorkItemReviewer => {
                let mut bundle = outcome.work_items[0].projection_bundle.clone();
                match corruption {
                    ProjectionPayloadHashCorruption::WorkItemHuman => bundle
                        .human_projection
                        .title
                        .push_str(" tampered without hash update"),
                    ProjectionPayloadHashCorruption::WorkItemCoder => bundle
                        .coder_projection
                        .objective
                        .push_str(" tampered without hash update"),
                    ProjectionPayloadHashCorruption::WorkItemReviewer => bundle
                        .reviewer_projection
                        .criterion_refs
                        .push("criterion_tampered".to_string()),
                    _ => unreachable!(),
                }
                overwrite_persisted_review_context_json(
                    plan_root
                        .join("work-item-projection-bundles")
                        .join(format!("{}.json", bundle.id)),
                    &bundle,
                );
            }
            ProjectionPayloadHashCorruption::PlanHuman
            | ProjectionPayloadHashCorruption::PlanCoder
            | ProjectionPayloadHashCorruption::PlanReviewer => {
                let mut projection = outcome.plan_projection_bundle.clone();
                match corruption {
                    ProjectionPayloadHashCorruption::PlanHuman => projection
                        .human_group_projection
                        .goal
                        .push_str(" tampered without hash update"),
                    ProjectionPayloadHashCorruption::PlanCoder => projection
                        .coder_group_context
                        .ordered_logical_work_item_ids
                        .reverse(),
                    ProjectionPayloadHashCorruption::PlanReviewer => {
                        projection.reviewer_group_matrix.work_items.reverse()
                    }
                    _ => unreachable!(),
                }
                overwrite_persisted_review_context_json(
                    plan_root
                        .join("plan-projection-bundles")
                        .join(format!("{}.json", projection.id)),
                    &projection,
                );
                engine.session.artifact = Some(ArtifactPayload::WorkItemPlanProjection {
                    projection: Box::new(projection),
                });
            }
        }

        if engine.build_work_item_plan_review_input().is_ok() {
            accepted.push(format!("{corruption:?}"));
        }
    }

    assert!(
        accepted.is_empty(),
        "Plan Review Context accepted payloads with stale hashes: {accepted:?}"
    );
}
