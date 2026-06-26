import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type {
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

  it("groups artifact versions by phase and keeps missing typed artifacts visible", () => {
    const onSelectVersion = vi.fn();
    const outlineArtifact = workItemPlanOutlineArtifact("后端数据层 v2", [
      "src/product/provider_catalog.rs",
    ]);
    const draftArtifact = workItemDraftArtifact("outline_backend", "draft_backend_001");

    render(
      <WorkItemPlanArtifactPanel
        artifact={outlineArtifact}
        versions={[
          workItemPlanArtifactVersion(1, outlineArtifact, false),
          workItemPlanArtifactVersion(2, draftArtifact, true),
          {
            ...workItemPlanArtifactVersion(3, outlineArtifact, false),
            artifact: null,
          },
        ]}
        selectedVersion={1}
        onSelectVersion={onSelectVersion}
      />,
    );

    expect(screen.getByTestId("work-item-version-group-outline")).toHaveTextContent(
      "Outline",
    );
    expect(screen.getByTestId("work-item-version-group-drafts")).toHaveTextContent(
      "Drafts",
    );
    expect(screen.getByText("无内容")).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("work-item-plan-version-2"));

    expect(onSelectVersion).toHaveBeenCalledWith(2);
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
        artifact={{
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
                title: "后端数据层",
                kind: "backend",
                goal: "实现 ProviderCatalog 与全局状态持久化。",
                implementation_context: "{&quot;required_gates&quot;:[&quot;cmd_check&quot;]}",
                exclusive_write_scopes: ["src/web/provider_catalog.rs"],
                forbidden_write_scopes: ["web/src/"],
                depends_on_outline_ids: [],
                required_handoff_from_outline_ids: [],
                verification_plan: {
                  commands: [
                    {
                      command: "cargo test --locked --lib provider_catalog",
                      description: "ProviderCatalog 单测",
                      expected_exit_code: 0,
                      id: "cmd_catalog",
                    },
                  ],
                  manual_checks: [],
                  required_gates: [
                    {
                      gate_id: "gate_backend_data",
                      name: "后端数据层验证",
                      description: "单测和格式检查通过",
                      depends_on: ["cmd_catalog"],
                    },
                  ],
                },
                handoff_summary: "后续安装器可调用 &quot;ProviderCatalog::required&quot;。",
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
        } as unknown as WorkItemPlanArtifactPayload}
      />,
    );

    const panel = screen.getByTestId("work-item-plan-artifact-panel");
    expect(panel).toHaveTextContent("后端数据层");
    expect(panel).toHaveTextContent("实现 ProviderCatalog 与全局状态持久化。");
    expect(panel).toHaveTextContent('"required_gates": [');
    expect(panel).toHaveTextContent('"cmd_check"');
    expect(panel).toHaveTextContent("src/web/provider_catalog.rs");
    expect(panel).toHaveTextContent("web/src/");
    expect(panel).toHaveTextContent("cargo test --locked --lib provider_catalog");
    expect(panel).toHaveTextContent("后端数据层验证");
    expect(panel).toHaveTextContent('后续安装器可调用 "ProviderCatalog::required"。');
    expect(panel).toHaveTextContent("missing_scope");
    expect(panel).not.toHaveTextContent("&quot;");
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
  record.candidate.verification_plan.commands = [
    {
      id: `cmd_${draftId}`,
      command,
      description: "ProviderCatalog 单测",
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
      title: `${outlineId} draft`,
      kind: "backend",
      goal: `实现 ${outlineId}`,
      implementation_context: "实现上下文",
      exclusive_write_scopes: [`src/product/${outlineId}.rs`],
      forbidden_write_scopes: ["web/src/"],
      depends_on_outline_ids: [],
      required_handoff_from_outline_ids: [],
      verification_plan: {
        commands: [
          {
            id: `cmd_${outlineId}`,
            command: `cargo test --locked --lib ${outlineId}`,
            description: `${outlineId} 单测`,
          },
        ],
        manual_checks: [],
        required_gates: [],
        risk_notes: [],
      },
      handoff_summary: `${outlineId} handoff`,
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
