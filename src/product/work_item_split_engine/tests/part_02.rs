fn valid_outline_author_output() -> serde_json::Value {
    serde_json::json!({
        "outline": {
            "id": "outline_artifact_1",
            "project_id": "project_0001",
            "issue_id": "issue_0001",
            "source_story_spec_ids": ["story_spec_0001"],
            "source_design_spec_ids": ["design_spec_0001"],
            "strategy_summary": "先后端后前端",
            "work_item_outlines": [
                {
                    "outline_id": "outline_backend",
                    "title": "后端 API",
                    "kind": "backend",
                    "goal": "实现 API",
                    "scope": ["src/product"],
                    "non_goals": [],
                    "estimated_context_tokens": 12000,
                    "session_fit": "fits_single_agent_session",
                    "source_story_spec_ids": ["story_spec_0001"],
                    "source_design_spec_ids": ["design_spec_0001"],
                    "exclusive_write_scopes": ["src/product/**"],
                    "forbidden_write_scopes": ["web/**"],
                    "depends_on": [],
                    "verification_intent": ["cargo test --locked --lib api"],
                    "handoff_notes": "提供 API contract"
                },
                {
                    "outline_id": "outline_frontend",
                    "title": "前端 UI",
                    "kind": "frontend",
                    "goal": "接入 API",
                    "scope": ["web/src"],
                    "non_goals": [],
                    "estimated_context_tokens": 10000,
                    "session_fit": "fits_single_agent_session",
                    "source_story_spec_ids": ["story_spec_0001"],
                    "source_design_spec_ids": ["design_spec_0001"],
                    "exclusive_write_scopes": ["web/src/**"],
                    "forbidden_write_scopes": ["src/product/**"],
                    "depends_on": ["outline_backend"],
                    "verification_intent": ["pnpm -C web test"],
                    "handoff_notes": "消费 API contract"
                }
            ],
            "dependency_graph": [
                {
                    "from_outline_id": "outline_backend",
                    "to_outline_id": "outline_frontend"
                }
            ],
            "risks": [],
            "handoff_strategy": "后端输出 contract 给前端",
            "status": "draft"
        },
        "context_blockers": []
    })
}

fn valid_work_item_draft_candidate_json(outline_id: &str) -> serde_json::Value {
    serde_json::json!({
        "outline_id": outline_id,
        "title": "后端 API",
        "kind": "backend",
        "goal": "实现 API",
        "implementation_context": "实现 API handler 与 product service。",
        "exclusive_write_scopes": ["src/product/**"],
        "forbidden_write_scopes": ["web/**"],
        "depends_on_outline_ids": [],
        "required_handoff_from_outline_ids": [],
        "handoff_summary": "输出 SessionStatusDto",
        "verification_plan": {
            "commands": [
                {
                    "id": "cmd_backend",
                    "label": "cargo test",
                    "command": "cargo test --locked --lib session",
                    "cwd": "",
                    "purpose": "验证后端 API",
                    "required": true,
                    "timeout_seconds": 120,
                    "safety": "approved",
                    "source": "local"
                }
            ],
            "manual_checks": [],
            "required_gates": ["cmd_backend"]
        }
    })
}

fn sample_draft_record(
    draft_id: &str,
    outline_id: &str,
    candidate: WorkItemDraftCandidate,
) -> WorkItemDraftRecord {
    WorkItemDraftRecord {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        plan_id: "plan_0001".to_string(),
        draft_id: draft_id.to_string(),
        outline_id: outline_id.to_string(),
        generation_round_id: "round_001".to_string(),
        batch_id: None,
        attempt_index: 1,
        outline_version_ref: "artifact://outline/1".to_string(),
        generation_mode: WorkItemGenerationMode::Serial,
        candidate,
        status: WorkItemDraftStatus::Accepted,
        active: true,
        superseded_by_draft_id: None,
        supersede_reason: None,
        copied_from_draft_id: None,
        review_node_id: None,
        review_verdict_ref: None,
        generated_from_node_id: "node_draft_run".to_string(),
        accepted_at: Some("2026-06-22T10:00:00Z".to_string()),
        superseded_at: None,
        created_at: "2026-06-22T10:00:00Z".to_string(),
        updated_at: "2026-06-22T10:00:00Z".to_string(),
    }
}
