use super::*;

pub(crate) fn work_item_split_findings_to_dto(
    findings: &[WorkItemSplitFinding],
) -> Vec<ValidatorFindingDto> {
    findings
        .iter()
        .map(|finding| ValidatorFindingDto {
            severity: finding.severity.as_str().to_string(),
            code: finding.code.clone(),
            message: finding.message.clone(),
            work_item_ids: finding.work_item_ids.clone(),
        })
        .collect()
}

pub(crate) fn work_item_plan_context_blockers_to_dto(
    blockers: &[WorkItemPlanContextBlocker],
) -> Vec<WorkItemPlanContextBlockerDto> {
    blockers
        .iter()
        .map(|blocker| WorkItemPlanContextBlockerDto {
            code: blocker.code.clone(),
            message: blocker.message.clone(),
            needed_context: blocker.needed_context.clone(),
        })
        .collect()
}

pub(crate) fn build_work_item_plan_candidate_dto(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> Result<WorkItemPlanCandidateDto, ProductStoreError> {
    let plan = lifecycle.get_issue_work_item_plan(project_id, issue_id, plan_id)?;
    let work_items = lifecycle.list_work_items(project_id, issue_id)?;
    let plan_work_item_ids: HashSet<String> = plan.work_item_ids.iter().cloned().collect();
    let plan_work_items: Vec<&LifecycleWorkItemRecord> = work_items
        .iter()
        .filter(|wi| plan_work_item_ids.contains(&wi.id))
        .collect();

    let found_ids: HashSet<&String> = plan_work_items.iter().map(|wi| &wi.id).collect();
    if let Some(missing_id) = plan_work_item_ids
        .iter()
        .find(|id| !found_ids.contains(*id))
    {
        return Err(ProductStoreError::NotFound {
            kind: "work_item",
            id: missing_id.clone(),
        });
    }

    let verification_plans: Vec<VerificationPlanDto> = plan
        .verification_plan_ids
        .iter()
        .map(|vp_id| {
            lifecycle
                .get_verification_plan(project_id, issue_id, vp_id)
                .map(|vp| VerificationPlanDto {
                    plan_ref: vp.id,
                    scope: vp.scope.as_str().to_string(),
                    commands: vp
                        .commands
                        .iter()
                        .map(|cmd| VerificationCommandDto {
                            label: cmd.label.clone(),
                            command: cmd.command.clone(),
                            cwd: cmd.cwd.clone(),
                            purpose: cmd.purpose.clone(),
                            required: cmd.required,
                            timeout_seconds: cmd.timeout_seconds,
                            safety: cmd.safety.as_str().to_string(),
                        })
                        .collect(),
                    manual_checks: vp
                        .manual_checks
                        .iter()
                        .map(|check| VerificationManualCheckDto {
                            label: check.label.clone(),
                            instructions: check.instructions.clone(),
                            required: check.required,
                        })
                        .collect(),
                    required_gates: vp.required_gates,
                    risk_notes: vp.risk_notes,
                    confidence: vp.confidence.as_str().to_string(),
                    fallback_policy: vp.fallback_policy.as_str().to_string(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let repository_profile = plan
        .repository_profile_ref
        .as_ref()
        .map(|rid| lifecycle.get_repository_profile(project_id, issue_id, rid))
        .transpose()?
        .map(|rp| RepositoryProfileDto {
            profile_id: rp.id,
            repository_id: rp.repository_id,
            languages: rp.languages,
            frameworks: rp.frameworks,
            package_managers: rp.package_managers,
            test_frameworks: rp.test_frameworks,
            build_systems: rp.build_systems,
            detected_layers: rp.detected_layers,
            split_recommendation: rp.split_recommendation,
            confidence: rp.confidence.as_str().to_string(),
        });

    let work_item_dtos: Vec<WorkItemCandidateDto> = plan_work_items
        .iter()
        .map(|wi| WorkItemCandidateDto {
            id: wi.id.clone(),
            kind: wi.kind.as_str().to_string(),
            title: wi.title.clone(),
            depends_on: wi.depends_on.clone(),
            exclusive_write_scopes: wi.exclusive_write_scopes.clone(),
            verification_plan_ref: wi.verification_plan_ref.clone(),
            meta: WorkItemCandidateMetaDto {
                reverted: false,
                revert_feedback: None,
            },
        })
        .collect();

    Ok(WorkItemPlanCandidateDto {
        plan: WorkItemPlanDto {
            id: plan.id,
            status: plan.status.as_str().to_string(),
            options: WorkItemSplitOptionsDto {
                include_integration_tests: plan.options.include_integration_tests,
                include_e2e_tests: plan.options.include_e2e_tests,
                force_frontend_backend_split: plan.options.force_frontend_backend_split,
                require_execution_plan_confirm: plan.options.require_execution_plan_confirm,
            },
            dependency_graph: plan
                .dependency_graph
                .iter()
                .map(|e| WorkItemDependencyEdgeDto {
                    from_work_item_id: e.from_work_item_id.clone(),
                    to_work_item_id: e.to_work_item_id.clone(),
                })
                .collect(),
        },
        work_items: work_item_dtos,
        verification_plans,
        repository_profile,
        validator_findings: work_item_split_findings_to_dto(&plan.validator_findings),
    })
}
