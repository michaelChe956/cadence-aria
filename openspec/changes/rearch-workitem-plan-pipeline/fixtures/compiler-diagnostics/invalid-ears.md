# Work Item Plan

## Work Item WI-001: Levels API fixture

### Identity
- schema_version: 1
- logical_work_item_id: WI-001
- title: Levels API fixture
- kind: backend

### Goal
- summary: WHEN the levels fixture runs THE SYSTEM SHALL provide a valid backend work item.

### Non Goals
- non_goals: Browser rendering is out of scope.

### Dependencies
- depends_on: []

### Inputs

### Outputs
- contract_id: contract.levels-api
- capabilities: api.levels.read

### Tasks
- task_id: TASK-001
- statement: the selector renders returned options.
- requirement_refs: REQ-WSC-02
- done_when_refs: AC-001

### Write Policy
- exclusive_scopes: src/product/levels/**
- forbidden_scopes: web/**

### Acceptance Criteria
- criterion_id: AC-001
- statement: WHEN GET /api/levels is requested THE SYSTEM SHALL return configured levels.
- required_evidence: source_diff
- required_evidence: non_zero_test_execution

### Verification
- check_id: CHECK-001
- command: cargo test --locked --lib levels_api
- manual_instruction: Confirm the endpoint returns configured levels.
- required: true
- non_zero_test_execution_required: true

### Handoff Schema
- required_fields: commit_sha
- provided_contract_refs: contract.levels-api
- reviewer_check_refs: AC-001

### Blockers
- reason_code: levels_api_contract_invalid
- route: plan_repair_current
- target_contract_refs: contract.levels-api

### Traceability
- source_type: design_spec
- source_id: design_spec_levels_0001
- requirement_id: REQ-WSC-02
