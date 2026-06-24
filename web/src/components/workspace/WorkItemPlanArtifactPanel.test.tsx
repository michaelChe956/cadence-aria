import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { WorkItemPlanArtifactPayload } from "../../api/types";
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
                implementation_context: "需要复用 json_store 原子写和 flock 文件锁。",
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
                handoff_summary: "后续安装器可调用 ProviderCatalog::required。",
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
    expect(panel).toHaveTextContent("需要复用 json_store 原子写和 flock 文件锁。");
    expect(panel).toHaveTextContent("src/web/provider_catalog.rs");
    expect(panel).toHaveTextContent("web/src/");
    expect(panel).toHaveTextContent("cargo test --locked --lib provider_catalog");
    expect(panel).toHaveTextContent("后端数据层验证");
    expect(panel).toHaveTextContent("后续安装器可调用 ProviderCatalog::required。");
    expect(panel).toHaveTextContent("missing_scope");
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
