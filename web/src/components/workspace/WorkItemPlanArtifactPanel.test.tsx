import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type {
  EvidenceKind,
  WorkItemPlanArtifactPayload,
  WorkItemPlanArtifactVersion,
} from "../../api/types";
import { WorkItemPlanArtifactPanel } from "./WorkItemPlanArtifactPanel";

vi.mock("@monaco-editor/react", () => ({
  Editor: ({
    value,
    language,
    height,
    options,
    theme,
  }: {
    value?: string;
    language?: string;
    height?: string;
    options?: { readOnly?: boolean; wordWrap?: string; minimap?: { enabled?: boolean } };
    theme?: string;
  }) => (
    <div
      data-testid="monaco-editor"
      data-language={language}
      data-height={height}
      data-read-only={String(options?.readOnly)}
      data-word-wrap={options?.wordWrap}
      data-minimap={String(options?.minimap?.enabled)}
      data-theme={theme}
    >
      {value}
    </div>
  ),
}));

describe("WorkItemPlanArtifactPanel", () => {
  it("renders the work item plan workspace shell with status, versions, and tabs", () => {
    const outlineV1 = workItemPlanOutlineArtifact("后端数据层 v1", [
      "src/product/provider_catalog.rs",
    ]);
    const outlineV2 = workItemPlanOutlineArtifact("后端数据层 v2", [
      "src/product/provider_catalog.rs",
      "src/product/global_provider_state.rs",
    ]);

    render(
      <WorkItemPlanArtifactPanel
        artifact={outlineV2}
        versions={[
          workItemPlanArtifactVersion(1, outlineV1, false),
          workItemPlanArtifactVersion(2, outlineV2, true),
        ]}
        selectedVersion={2}
        onSelectVersion={vi.fn()}
      />,
    );

    expect(screen.getByText("Work Item Plan 工作台")).toBeInTheDocument();
    expect(
      screen.getByText("Outline 已生成，等待确认。Work Item 尚未生成。"),
    ).toBeInTheDocument();
    expect(screen.getByTestId("work-item-plan-version-rail")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Overview" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Outline" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Drafts" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Diff" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Review" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "JSON" })).toBeInTheDocument();
  });

  it("describes draft, batch, compile, and historical states without implying all work items are done", () => {
    const draftArtifact = workItemDraftArtifact("outline_backend", "draft_backend_001");
    const batchArtifact = workItemBatchArtifact([
      workItemDraftRecord("outline_backend", "draft_backend_001"),
      workItemDraftRecord("outline_frontend", "draft_frontend_001"),
    ]);
    const compileArtifact = workItemCompileArtifact();

    const { rerender } = render(<WorkItemPlanArtifactPanel artifact={draftArtifact} />);

    expect(
      screen.getByText("当前仅展示单个 Draft，不代表整组 Work Item 完成。"),
    ).toBeInTheDocument();

    rerender(<WorkItemPlanArtifactPanel artifact={batchArtifact} />);

    expect(
      screen.getByText("已生成 2 个 Draft，等待接受全部或返修。"),
    ).toBeInTheDocument();

    rerender(<WorkItemPlanArtifactPanel artifact={compileArtifact} />);

    expect(
      screen.getByText(
        "Compile 已提交，生成 2 个 Work Item、2 个 Verification Plan、1 个 child session。",
      ),
    ).toBeInTheDocument();

    rerender(
      <WorkItemPlanArtifactPanel
        artifact={draftArtifact}
        readonly
        selectedVersion={1}
      />,
    );

    expect(
      screen.getByText("正在查看历史版本 v1，不影响当前流程。"),
    ).toBeInTheDocument();
  });

  it("switches workspace tabs for outline, drafts, review, and json views", async () => {
    const outlineArtifact = workItemPlanOutlineArtifact("后端数据层 v2", [
      "src/product/provider_catalog.rs",
    ]);
    const { rerender } = render(<WorkItemPlanArtifactPanel artifact={outlineArtifact} />);

    fireEvent.click(screen.getByRole("button", { name: "Outline" }));

    expect(screen.getByTestId("work-item-outline-table")).toHaveTextContent(
      "outline_backend_data",
    );
    expect(screen.getByTestId("work-item-outline-table")).toHaveTextContent(
      "src/product/provider_catalog.rs",
    );

    const batchArtifact = workItemBatchArtifact([
      workItemDraftRecord("outline_backend", "draft_backend_001"),
      workItemDraftRecord("outline_frontend", "draft_frontend_001"),
    ]);
    rerender(<WorkItemPlanArtifactPanel artifact={batchArtifact} />);

    fireEvent.click(screen.getByRole("button", { name: "Drafts" }));

    expect(screen.getByTestId("work-item-draft-list")).toHaveTextContent(
      "draft_backend_001",
    );
    expect(screen.getByTestId("work-item-draft-list")).toHaveTextContent(
      "draft_frontend_001",
    );
    expect(screen.getByTestId("work-item-draft-detail")).toHaveTextContent(
      "cargo test --locked --lib outline_backend",
    );

    rerender(<WorkItemPlanArtifactPanel artifact={workItemDraftArtifactWithFinding()} />);

    fireEvent.click(screen.getByRole("button", { name: "Review" }));

    expect(screen.getByTestId("work-item-review-tab")).toHaveTextContent(
      "Blocking findings",
    );
    expect(screen.getByTestId("work-item-review-tab")).toHaveTextContent(
      "missing_scope",
    );

    fireEvent.click(screen.getByRole("button", { name: "JSON" }));

    const editor = await screen.findByTestId("monaco-editor");
    expect(editor).toHaveAttribute("data-language", "json");
    expect(editor).toHaveTextContent("missing_scope");
  });

  it("uses cascading phase and version selects for artifact versions", () => {
    const onSelectVersion = vi.fn();
    const outlineArtifact = workItemPlanOutlineArtifact("后端数据层 v2", [
      "src/product/provider_catalog.rs",
    ]);
    const draftArtifact = workItemDraftArtifact("outline_backend", "draft_backend_001");
    const compileArtifact = workItemCompileArtifact();

    render(
      <WorkItemPlanArtifactPanel
        artifact={outlineArtifact}
        versions={[
          workItemPlanArtifactVersion(1, outlineArtifact, false),
          workItemPlanArtifactVersion(2, draftArtifact, true),
          workItemPlanArtifactVersion(3, compileArtifact, false),
          workItemPlanArtifactVersion(4, compileArtifact, false),
        ]}
        selectedVersion={1}
        onSelectVersion={onSelectVersion}
      />,
    );

    const phaseSelect = screen.getByLabelText("Artifact phase");
    const versionSelect = screen.getByLabelText("Artifact version");
    expect(phaseSelect).toHaveValue("outline");
    expect(versionSelect).toHaveValue("1");

    fireEvent.change(phaseSelect, { target: { value: "drafts" } });
    expect(onSelectVersion).toHaveBeenCalledWith(2);

    fireEvent.change(phaseSelect, { target: { value: "compile" } });
    expect(onSelectVersion).toHaveBeenCalledWith(4);

    fireEvent.change(versionSelect, { target: { value: "3" } });

    expect(onSelectVersion).toHaveBeenCalledWith(3);
    expect(phaseSelect).toHaveTextContent("Final Compile");
  });

  it("renders compile report as a structured summary without before after debug json", () => {
    render(<WorkItemPlanArtifactPanel artifact={workItemCompileArtifact()} />);

    expect(screen.getByText("Compile Report")).toBeInTheDocument();
    expect(screen.getByText(/work_item_backend/)).toBeInTheDocument();
    expect(screen.getByText(/verification_backend/)).toBeInTheDocument();
    expect(screen.getByText(/child_session_backend/)).toBeInTheDocument();
    expect(screen.queryByText("Before")).not.toBeInTheDocument();
    expect(screen.queryByTestId("compile-report-before-after")).not.toBeInTheDocument();
  });

  it("shows structured diff for outline and draft versions", () => {
    const outlineV1 = workItemPlanOutlineArtifact("后端数据层 v1", [
      "src/product/provider_catalog.rs",
    ]);
    const outlineV2 = workItemPlanOutlineArtifact("后端数据层 v2", [
      "src/product/provider_catalog.rs",
      "src/product/global_provider_state.rs",
    ]);
    const { rerender } = render(
      <WorkItemPlanArtifactPanel
        artifact={outlineV2}
        versions={[
          workItemPlanArtifactVersion(1, outlineV1, false),
          workItemPlanArtifactVersion(2, outlineV2, true),
        ]}
        selectedVersion={2}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Diff" }));

    expect(screen.getByTestId("work-item-diff-tab")).toHaveTextContent(
      "exclusive_write_scopes",
    );
    expect(screen.getByTestId("work-item-diff-tab")).toHaveTextContent(
      "src/product/global_provider_state.rs",
    );

    const draftV1 = workItemDraftArtifactWithCommand(
      "draft_backend_001",
      "cargo test --locked --lib provider_catalog",
    );
    const draftV2 = workItemDraftArtifactWithCommand(
      "draft_backend_002",
      "cargo test --locked --lib provider_catalog_new",
    );
    rerender(
      <WorkItemPlanArtifactPanel
        artifact={draftV2}
        versions={[
          workItemPlanArtifactVersion(1, draftV1, false),
          workItemPlanArtifactVersion(2, draftV2, true),
        ]}
        selectedVersion={2}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Diff" }));

    expect(screen.getByTestId("work-item-diff-tab")).toHaveTextContent(
      "verification.commands",
    );
    expect(screen.getByTestId("work-item-diff-tab")).toHaveTextContent(
      "provider_catalog_new",
    );
  });

  it("renders outline artifacts as readable work item cards from backend fields", () => {
    render(
      <WorkItemPlanArtifactPanel
        artifact={
          {
            type: "outline_candidate",
            payload: {
              outline: {
                id: "outline_artifact_1",
                project_id: "project_0001",
                issue_id: "issue_0001",
                source_story_spec_ids: ["story_spec_0001"],
                source_design_spec_ids: ["design_spec_0001"],
                strategy_summary: "先做后端数据层，再做前端入口。",
                work_item_outlines: [
                  {
                    outline_id: "outline_backend_data",
                    title: "后端数据层",
                    kind: "backend",
                    goal: "实现 ProviderCatalog 与全局状态持久化。",
                    scope: ["新增 provider_catalog.rs", "新增 global_provider_state.rs"],
                    non_goals: ["不实现安装器"],
                    source_story_spec_ids: ["story_spec_0001"],
                    source_design_spec_ids: ["design_spec_0001"],
                    exclusive_write_scopes: ["src/web/provider_catalog.rs"],
                    forbidden_write_scopes: ["web/src/"],
                    depends_on: [],
                    verification_intent: ["cargo test --locked --lib provider_catalog"],
                    handoff_notes: "向安装器交付 catalog 查询接口。",
                  },
                ],
                dependency_graph: [],
                risks: ["全局状态并发写"],
                handoff_strategy: "逐项验证后交付下一项。",
                status: "draft",
              },
              design_context_gaps: [],
              validator_findings: [],
              context_blockers: [],
              current_generation_round_id: "round_001",
            },
          } as unknown as WorkItemPlanArtifactPayload
        }
      />,
    );

    const panel = screen.getByTestId("work-item-plan-artifact-panel");
    expect(panel).toHaveTextContent("先做后端数据层，再做前端入口。");
    expect(panel).toHaveTextContent("后端数据层");
    expect(panel).toHaveTextContent("实现 ProviderCatalog 与全局状态持久化。");
    expect(panel).toHaveTextContent("新增 provider_catalog.rs");
    expect(panel).toHaveTextContent("不实现安装器");
    expect(panel).toHaveTextContent("cargo test --locked --lib provider_catalog");
    expect(panel).toHaveTextContent("向安装器交付 catalog 查询接口。");
    expect(panel).toHaveTextContent("全局状态并发写");
  });

  it("renders draft artifacts with the current work item content needed for review", () => {
    render(
      <WorkItemPlanArtifactPanel
        artifact={
          {
            type: "draft_candidate",
            payload: {
              draft_record: {
                draft_id: "draft_001",
                plan_id: "plan_001",
                generation_round_id: "round_001",
                outline_id: "outline_backend_data",
                batch_id: null,
                candidate: {
                  outline_id: "outline_backend_data",
                  logical_work_item_id: "wi_backend_data",
                  canonical_contract_candidate: {
                    schema_version: 1,
                    identity: {
                      logical_work_item_id: "wi_backend_data",
                      title: "后端数据层",
                      kind: "backend",
                    },
                    goal: { summary: "实现 ProviderCatalog 与全局状态持久化。" },
                    non_goals: [],
                    input_contracts: [],
                    output_contracts: [
                      {
                        contract_id: "contract_backend_data",
                        capabilities: ["ProviderCatalog 查询接口"],
                      },
                    ],
                    tasks: [
                      {
                        task_id: "task_catalog",
                        statement: "实现 ProviderCatalog。",
                        requirement_refs: ["REQ-001"],
                        done_when_refs: ["criterion_catalog"],
                      },
                    ],
                    write_policy: {
                      exclusive_scopes: ["src/web/provider_catalog.rs"],
                      forbidden_scopes: ["web/src/"],
                    },
                    acceptance_criteria: [
                      {
                        criterion_id: "criterion_catalog",
                        statement: "catalog 查询接口可用。",
                        required_evidence: ["source_diff", "manual_check"] as EvidenceKind[],
                      },
                    ],
                    verification_checks: [
                      {
                        check_id: "cmd_catalog",
                        command: "cargo test --locked --lib provider_catalog",
                        manual_instruction: null,
                        required: true,
                        non_zero_test_execution_required: true,
                      },
                    ],
                    handoff_contract: {
                      required_fields: ["catalog_api"],
                      provided_contract_refs: ["contract_backend_data"],
                      reviewer_check_refs: ["criterion_catalog"],
                    },
                    blocker_rules: [],
                    design_traceability: [],
                  },
                  verification_plan: {
                    checks: [
                      {
                        check_id: "cmd_catalog",
                        command: "cargo test --locked --lib provider_catalog",
                        manual_instruction: null,
                        required: true,
                        non_zero_test_execution_required: true,
                      },
                    ],
                  },
                },
                status: "draft",
                active: true,
                superseded: false,
                superseded_by_draft_id: null,
                supersede_reason: null,
                copied_from_draft_id: null,
                generated_from_node_id: "node_draft",
                accepted_by_node_id: null,
                created_at: "2026-06-23T00:00:00Z",
                updated_at: "2026-06-23T00:00:00Z",
              },
              validator_findings: [
                {
                  severity: "error",
                  code: "missing_scope",
                  message: "缺少写入范围",
                  work_item_ids: ["outline_backend_data"],
                },
              ],
              can_accept: false,
            },
          } as unknown as WorkItemPlanArtifactPayload
        }
      />,
    );

    const panel = screen.getByTestId("work-item-plan-artifact-panel");
    expect(panel).toHaveTextContent("后端数据层");
    expect(panel).toHaveTextContent("实现 ProviderCatalog 与全局状态持久化。");
    expect(panel).toHaveTextContent("src/web/provider_catalog.rs");
    expect(panel).toHaveTextContent("web/src/");
    expect(panel).toHaveTextContent("cargo test --locked --lib provider_catalog");
    expect(panel).toHaveTextContent("catalog_api");
    expect(panel).toHaveTextContent("missing_scope");
  });
  it("renders canonical contract draft artifacts without crashing", () => {
    render(
      <WorkItemPlanArtifactPanel
        artifact={
          {
            type: "draft_candidate",
            payload: {
              draft_record: {
                draft_id: "draft_001",
                plan_id: "plan_001",
                generation_round_id: "round_005",
                outline_id: "outline_implement_compact_duration",
                batch_id: null,
                candidate: {
                  outline_id: "outline_implement_compact_duration",
                  logical_work_item_id: "wi_implement_compact_duration",
                  canonical_contract_candidate: {
                    schema_version: 1,
                    identity: {
                      logical_work_item_id: "wi_implement_compact_duration",
                      title: "实现并导出紧凑时长格式化函数",
                      kind: "other",
                    },
                    goal: {
                      summary: "在独立 ESM 模块中提供 formatCompactDuration 命名导出。",
                    },
                    non_goals: ["不创建自动化单元测试文件。"],
                    input_contracts: [],
                    output_contracts: [
                      {
                        contract_id: "contract_compact_duration_module",
                        capabilities: ["提供 ESM 命名导出 formatCompactDuration。"],
                      },
                    ],
                    tasks: [
                      {
                        task_id: "task_module_surface",
                        statement: "建立仅含命名导出的 ESM 模块边界。",
                        requirement_refs: ["REQ-001"],
                        done_when_refs: ["criterion_module_export"],
                      },
                    ],
                    write_policy: {
                      exclusive_scopes: ["src/formatCompactDuration.mjs"],
                      forbidden_scopes: ["test/**"],
                    },
                    acceptance_criteria: [
                      {
                        criterion_id: "criterion_module_export",
                        statement: "源模块提供且仅提供可命名导入的 formatCompactDuration。",
                        required_evidence: ["source_diff", "manual_check"] as EvidenceKind[],
                      },
                    ],
                    verification_checks: [
                      {
                        check_id: "check_vectors",
                        command: "node --test test/formatCompactDuration.test.mjs",
                        manual_instruction: null,
                        required: true,
                        non_zero_test_execution_required: true,
                      },
                    ],
                    handoff_contract: {
                      required_fields: ["module_path", "named_export"],
                      provided_contract_refs: ["contract_compact_duration_module"],
                      reviewer_check_refs: ["criterion_module_export"],
                    },
                    blocker_rules: [
                      {
                        reason_code: "output_contract_capability_missing",
                        route: "coder_rework",
                        target_contract_refs: ["contract_compact_duration_module"],
                      },
                    ],
                    design_traceability: [
                      {
                        source_type: "story_spec",
                        source_id: "story_spec_0001",
                        requirement_id: "REQ-001",
                      },
                    ],
                  },
                  verification_plan: {
                    checks: [
                      {
                        check_id: "check_vectors",
                        command: "node --test test/formatCompactDuration.test.mjs",
                        manual_instruction: null,
                        required: true,
                        non_zero_test_execution_required: true,
                      },
                    ],
                  },
                },
                status: "draft",
                active: true,
                superseded: false,
                superseded_by_draft_id: null,
                supersede_reason: null,
                copied_from_draft_id: null,
                generated_from_node_id: "node_draft",
                accepted_by_node_id: null,
                created_at: "2026-07-25T00:00:00Z",
                updated_at: "2026-07-25T00:00:00Z",
              },
              validator_findings: [],
              can_accept: false,
            },
          } as unknown as WorkItemPlanArtifactPayload
        }
      />,
    );

    const panel = screen.getByTestId("work-item-plan-artifact-panel");
    expect(panel).toHaveTextContent("实现并导出紧凑时长格式化函数");
    expect(panel).toHaveTextContent("在独立 ESM 模块中提供 formatCompactDuration 命名导出。");
    expect(panel).toHaveTextContent("src/formatCompactDuration.mjs");
    expect(panel).toHaveTextContent("test/**");
    expect(panel).toHaveTextContent("node --test test/formatCompactDuration.test.mjs");
    expect(panel).toHaveTextContent("建立仅含命名导出的 ESM 模块边界。");
    expect(panel).toHaveTextContent("module_path");
    expect(panel).toHaveTextContent("contract_compact_duration_module");
  });

  it("switches to a JSON source view rendered with Monaco", async () => {
    const artifact = {
      type: "outline_candidate",
      payload: {
        outline: {
          id: "outline_artifact_1",
          project_id: "project_0001",
          issue_id: "issue_0001",
          source_story_spec_ids: ["story_spec_0001"],
          source_design_spec_ids: ["design_spec_0001"],
          strategy_summary: "先做后端数据层，再做前端入口。",
          work_item_outlines: [
            {
              outline_id: "outline_backend_data",
              title: "后端数据层",
              kind: "backend",
              goal: "实现 ProviderCatalog 与全局状态持久化。",
              scope: ["新增 provider_catalog.rs", "新增 global_provider_state.rs"],
              non_goals: ["不实现安装器"],
              source_story_spec_ids: ["story_spec_0001"],
              source_design_spec_ids: ["design_spec_0001"],
              exclusive_write_scopes: ["src/web/provider_catalog.rs"],
              forbidden_write_scopes: ["web/src/"],
              depends_on: [],
              verification_intent: ["cargo test --locked --lib provider_catalog"],
              handoff_notes: "向安装器交付 catalog 查询接口。",
            },
          ],
          dependency_graph: [],
          risks: ["全局状态并发写"],
          handoff_strategy: "逐项验证后交付下一项。",
          status: "draft",
        },
        design_context_gaps: [],
        validator_findings: [],
        context_blockers: [],
        current_generation_round_id: "round_001",
      },
    } as unknown as WorkItemPlanArtifactPayload;

    render(<WorkItemPlanArtifactPanel artifact={artifact} />);

    expect(screen.getByTestId("outline-view-cards")).toBeInTheDocument();
    expect(screen.getByTestId("outline-view-source")).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("outline-view-source"));

    const editor = await screen.findByTestId("monaco-editor");
    expect(editor).toHaveAttribute("data-language", "json");
    expect(editor).toHaveTextContent("outline_artifact_1");
    expect(editor).toHaveTextContent("outline_backend_data");
  });
});

function workItemPlanOutlineArtifact(
  title: string,
  writeScopes: string[],
): WorkItemPlanArtifactPayload {
  return {
    type: "outline_candidate",
    payload: {
      outline: {
        id: "outline_artifact_1",
        project_id: "project_0001",
        issue_id: "issue_0001",
        plan_id: "plan_001",
        source_story_spec_ids: ["story_spec_0001"],
        source_design_spec_ids: ["design_spec_0001"],
        strategy_summary: "先做后端数据层，再做前端入口。",
        work_item_outlines: [
          {
            outline_id: "outline_backend_data",
            title,
            kind: "backend",
            goal: "实现 ProviderCatalog 与全局状态持久化。",
            scope: ["新增 provider_catalog.rs"],
            non_goals: ["不实现安装器"],
            source_story_spec_ids: ["story_spec_0001"],
            source_design_spec_ids: ["design_spec_0001"],
            exclusive_write_scopes: writeScopes,
            forbidden_write_scopes: ["web/src/"],
            depends_on: [],
            verification_intent: ["cargo test --locked --lib provider_catalog"],
            handoff_notes: "向安装器交付 catalog 查询接口。",
          },
        ],
        dependency_graph: [],
        risks: ["全局状态并发写"],
        handoff_strategy: "逐项验证后交付下一项。",
        status: "draft",
      },
      design_context_gaps: [],
      validator_findings: [],
      context_blockers: [],
      current_generation_round_id: "round_001",
    },
  };
}

function workItemPlanArtifactVersion(
  version: number,
  artifact: WorkItemPlanArtifactPayload,
  isCurrent: boolean,
): WorkItemPlanArtifactVersion {
  return {
    version,
    generated_by: "claude_code",
    reviewed_by: null,
    review_verdict: null,
    confirmed_by: null,
    is_current: isCurrent,
    created_at: "2026-06-26T00:00:00Z",
    source_node_id: `node_v${version}`,
    artifact,
  };
}

function workItemDraftArtifact(
  outlineId: string,
  draftId: string,
): WorkItemPlanArtifactPayload {
  return {
    type: "draft_candidate",
    payload: {
      draft_record: workItemDraftRecord(outlineId, draftId),
      validator_findings: [],
      can_accept: true,
    },
  };
}

function workItemDraftArtifactWithFinding(): WorkItemPlanArtifactPayload {
  return {
    type: "draft_candidate",
    payload: {
      draft_record: workItemDraftRecord("outline_backend", "draft_backend_001"),
      validator_findings: [
        {
          finding_id: "missing_scope",
          level: "error",
          severity: "error",
          code: "missing_scope",
          message: "缺少写入范围",
          affected_scopes: ["src/product"],
          work_item_ids: ["outline_backend"],
        },
      ],
      can_accept: false,
    },
  };
}

function workItemDraftArtifactWithCommand(
  draftId: string,
  command: string,
): WorkItemPlanArtifactPayload {
  const record = workItemDraftRecord("outline_backend", draftId);
  record.candidate.verification_plan.checks = [
    {
      check_id: `cmd_${draftId}`,
      command,
      manual_instruction: null,
      required: true,
      non_zero_test_execution_required: true,
    },
  ];
  return {
    type: "draft_candidate",
    payload: {
      draft_record: record,
      validator_findings: [],
      can_accept: true,
    },
  };
}

function workItemBatchArtifact(
  draftRecords: ReturnType<typeof workItemDraftRecord>[],
): WorkItemPlanArtifactPayload {
  return {
    type: "batch_state",
    payload: {
      batch_id: "batch_001",
      generation_round_id: "round_001",
      queue: draftRecords.map((record) => record.outline_id),
      draft_records: draftRecords,
      batch_status: "review_pending",
      failure_summary: [],
    },
  };
}

function workItemCompileArtifact(): WorkItemPlanArtifactPayload {
  return {
    type: "compile_report",
    payload: {
      compile_id: "compile_001",
      generation_round_id: "round_001",
      status: "committed",
      plan_commit_state: "committed",
      work_item_ids: ["work_item_backend", "work_item_frontend"],
      verification_plan_ids: ["verification_backend", "verification_frontend"],
      child_session_ids: ["child_session_backend"],
      validator_findings: [],
    },
  };
}

function workItemDraftRecord(outlineId: string, draftId: string) {
  return {
    draft_id: draftId,
    plan_id: "plan_001",
    generation_round_id: "round_001",
    outline_id: outlineId,
    batch_id: null,
    attempt_index: 1,
    generation_mode: "serial",
    candidate: {
      outline_id: outlineId,
      logical_work_item_id: `wi_${outlineId}`,
      canonical_contract_candidate: {
        schema_version: 1,
        identity: {
          logical_work_item_id: `wi_${outlineId}`,
          title: `${outlineId} draft`,
          kind: "backend",
        },
        goal: { summary: `实现 ${outlineId}` },
        non_goals: [],
        input_contracts: [],
        output_contracts: [
          {
            contract_id: `contract_${outlineId}`,
            capabilities: [`提供 ${outlineId} 能力`],
          },
        ],
        tasks: [
          {
            task_id: `task_${outlineId}`,
            statement: `实现 ${outlineId} 模块边界`,
            requirement_refs: ["REQ-001"],
            done_when_refs: [`criterion_${outlineId}`],
          },
        ],
        write_policy: {
          exclusive_scopes: [`src/product/${outlineId}.rs`],
          forbidden_scopes: ["web/src/"],
        },
        acceptance_criteria: [
          {
            criterion_id: `criterion_${outlineId}`,
            statement: `${outlineId} 交付可用`,
            required_evidence: ["source_diff", "manual_check"] as EvidenceKind[],
          },
        ],
        verification_checks: [
          {
            check_id: `cmd_${outlineId}`,
            command: `cargo test --locked --lib ${outlineId}`,
            manual_instruction: null,
            required: true,
            non_zero_test_execution_required: true,
          },
        ],
        handoff_contract: {
          required_fields: [`${outlineId}_handoff_field`],
          provided_contract_refs: [`contract_${outlineId}`],
          reviewer_check_refs: [`criterion_${outlineId}`],
        },
        blocker_rules: [],
        design_traceability: [],
      },
      verification_plan: {
        checks: [
          {
            check_id: `cmd_${outlineId}`,
            command: `cargo test --locked --lib ${outlineId}`,
            manual_instruction: null,
            required: true,
            non_zero_test_execution_required: true,
          },
        ],
      },
    },
    status: "draft",
    active: true,
    superseded: false,
    superseded_by_draft_id: null,
    supersede_reason: null,
    copied_from_draft_id: null,
    generated_from_node_id: `node_${draftId}`,
    accepted_by_node_id: null,
    created_at: "2026-06-26T00:00:00Z",
    updated_at: "2026-06-26T00:00:00Z",
  };
}
