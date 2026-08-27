# Work Item Plan

## Work Item WI-001: Backend levels API

### Identity
- schema_version: 1
- logical_work_item_id: WI-001
- title: Backend levels API
- kind: backend

### Goal
- summary: WHEN a level list request arrives THE SYSTEM SHALL return the configured levels JSON.

### Non Goals
- non_goals: Rendering HTML and changing browser assets are out of scope.

### Dependencies
- depends_on: []

### Inputs

### Outputs
- contract_id: contract.levels-api
- capabilities: api.levels.read

### Tasks
- task_id: TASK-001
- statement: WHEN the levels API receives GET /api/levels THE SYSTEM SHALL return the configured levels JSON.
- requirement_refs: REQ-WSC-02
- done_when_refs: AC-001

### Write Policy
- exclusive_scopes: src/product/levels/**
- forbidden_scopes: web/**

### Acceptance Criteria
- criterion_id: AC-001
- statement: WHEN GET /api/levels is requested THE SYSTEM SHALL return the configured levels JSON.
- required_evidence: source_diff
- required_evidence: non_zero_test_execution

### Verification
- check_id: CHECK-001
- command: cargo test --locked --lib levels_api
- manual_instruction: Confirm the endpoint returns the configured levels JSON.
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

## Work Item WI-002: Frontend level selector

### Identity
- schema_version: 1
- logical_work_item_id: WI-002
- title: Frontend level selector
- kind: frontend

### Goal
- summary: WHEN the levels page loads THE SYSTEM SHALL present a selector backed by the levels API.

### Non Goals
- non_goals: Changing the levels API response is out of scope.

### Dependencies
- depends_on: WI-001

### Inputs
- contract_id: contract.levels-api
- provider_logical_work_item_id: WI-001
- required_capabilities: api.levels.read
- compatibility_policy: require_all

### Outputs
- contract_id: contract.level-selector
- capabilities: ui.level-selector.rendered

### Tasks
- task_id: TASK-002
- statement: WHEN the levels page loads THE SYSTEM SHALL create the level selector browser behavior.
- requirement_refs: REQ-WSC-02
- done_when_refs: AC-002

### Write Policy
- exclusive_scopes: web/src/levels/**
- forbidden_scopes: src/product/levels/**

### Acceptance Criteria
- criterion_id: AC-002
- statement: WHEN the levels page loads THE SYSTEM SHALL render the #level-selector container and load level-select.js.
- required_evidence: source_diff
- required_evidence: manual_check

### Verification
- check_id: CHECK-002
- command: pnpm test level-select
- manual_instruction: Confirm the levels page contains #level-selector and loads level-select.js.
- required: true
- non_zero_test_execution_required: true

### Handoff Schema
- required_fields: commit_sha
- provided_contract_refs: contract.level-selector
- reviewer_check_refs: AC-002

### Blockers
- reason_code: level_selector_contract_invalid
- route: plan_repair_upstream
- target_contract_refs: contract.levels-api

### Traceability
- source_type: design_spec
- source_id: design_spec_levels_0001
- requirement_id: REQ-WSC-06

## Work Item WI-003: Integration levels API coverage

### Identity
- schema_version: 1
- logical_work_item_id: WI-003
- title: Integration levels API coverage
- kind: integration

### Goal
- summary: WHEN the levels integration suite runs THE SYSTEM SHALL verify the browser script consumes the levels API.

### Non Goals
- non_goals: Product implementation is out of scope; tests/integration/** is explicitly allowed.
- non_goals: Upstream tests are out of scope.

### Dependencies
- depends_on: WI-001
- depends_on: WI-002

### Inputs
- contract_id: contract.levels-api
- provider_logical_work_item_id: WI-001
- required_capabilities: api.levels.read
- compatibility_policy: require_all
- contract_id: contract.level-selector
- provider_logical_work_item_id: WI-002
- required_capabilities: ui.level-selector.rendered
- compatibility_policy: require_all

### Outputs
- contract_id: contract.levels-integration
- capabilities: tests.integration.levels.covered

### Tasks
- task_id: TASK-003
- statement: WHEN the integration suite runs THE SYSTEM SHALL exercise the levels API and level selector together.
- requirement_refs: REQ-WSC-07
- done_when_refs: AC-003

### Write Policy
- exclusive_scopes: tests/integration/**
- forbidden_scopes: src/product/levels/**
- forbidden_scopes: web/src/levels/**

### Acceptance Criteria
- criterion_id: AC-003
- statement: WHEN level-select.js runs THE SYSTEM SHALL request /api/levels and render returned options.
- required_evidence: source_diff
- required_evidence: non_zero_test_execution

### Verification
- check_id: CHECK-003
- command: cargo test --locked --test levels_integration
- manual_instruction: Confirm level-select.js requests /api/levels and renders the returned options.
- required: true
- non_zero_test_execution_required: true

### Handoff Schema
- required_fields: commit_sha
- provided_contract_refs: contract.levels-integration
- reviewer_check_refs: AC-003

### Blockers
- reason_code: levels_integration_contract_invalid
- route: verification_retry
- target_contract_refs: contract.levels-api
- target_contract_refs: contract.level-selector

### Traceability
- source_type: design_spec
- source_id: design_spec_levels_0001
- requirement_id: REQ-WSC-07

### Notes
This fixture is static compiler input only.

### Rationale
The three items keep backend, frontend, and integration ownership separate.
