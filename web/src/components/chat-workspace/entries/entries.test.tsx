import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryRenderer } from "../ChatEntryRenderer";
import { ErrorEntry } from "./ErrorEntry";
import { ExecutionEventEntry } from "./ExecutionEventEntry";
import { PermissionRequestEntry } from "./PermissionRequestEntry";
import { PermissionResponseEntry } from "./PermissionResponseEntry";
import { MarkdownContent, ProviderStreamEntry } from "./ProviderStreamEntry";
import { UserContextEntry } from "./UserContextEntry";

describe("chat workspace entries", () => {
  it("renders user context entries", () => {
    const entry = makeEntry({
      type: "context_note",
      role: "user",
      content: "需要支持手机号登录",
    });

    render(<UserContextEntry entry={entry} />);

    expect(screen.getByText("你")).toBeInTheDocument();
    expect(screen.getByText("需要支持手机号登录")).toBeInTheDocument();
  });

  it("renders prepared story and design context entries as shared audit input", () => {
    const entry = makeEntry({
      type: "context_note",
      role: "user",
      content: "Workspace 生成任务已准备\n\n[system]\n你是候选 spec 生成器。",
      metadata: {
        prepared: true,
      },
    });

    render(<UserContextEntry entry={entry} />);

    expect(screen.getByText("初始化输入")).toBeInTheDocument();
    expect(screen.getByText("CONTEXT")).toBeInTheDocument();
    expect(screen.getByText("Workspace 初始化上下文")).toBeInTheDocument();
    expect(screen.getByText("4 行")).toBeInTheDocument();
    expect(screen.getByText(/候选 spec 生成器/)).toBeInTheDocument();
    expect(screen.queryByText("PROMPT")).not.toBeInTheDocument();
  });

  it("renders prepared work item plan prompt entries as audit input", () => {
    const entry = makeEntry({
      type: "context_note",
      role: "user",
      content: "你是 Aria 的 WorkItemPlan Outline Planner。\n[strict_output_contract]",
      metadata: {
        prepared: true,
        prompt_source: "provider_prompt",
        prompt_node_title: "WorkItemPlan Outline 生成",
        provider: "claude_code",
      },
    });

    render(<UserContextEntry entry={entry} />);

    expect(screen.getByText("实际执行 Prompt")).toBeInTheDocument();
    expect(screen.getByText("PROMPT")).toBeInTheDocument();
    expect(screen.getByText("WorkItemPlan Outline 生成")).toBeInTheDocument();
    expect(screen.getByText("claude_code")).toBeInTheDocument();
    expect(screen.getByText(/WorkItemPlan Outline Planner/)).toBeInTheDocument();
  });

  it("renders provider stream entries as markdown-like blocks", () => {
    const entry = makeEntry({
      type: "provider_stream",
      role: "author",
      content: "# Story Spec\n\n支持手机号登录。",
    });

    render(<ProviderStreamEntry entry={entry} />);

    expect(screen.getByRole("heading", { name: "Story Spec" })).toBeInTheDocument();
    expect(screen.getByText("支持手机号登录。")).toBeInTheDocument();
  });

  it("labels tester and analyst provider stream entries by role", () => {
    render(
      <ProviderStreamEntry
        entry={makeEntry({
          id: "tester-stream",
          type: "provider_stream",
          role: "tester",
          content: "测试计划",
        })}
      />,
    );
    expect(screen.getByText("Tester")).toBeInTheDocument();

    render(
      <ProviderStreamEntry
        entry={makeEntry({
          id: "analyst-stream",
          type: "provider_stream",
          role: "analyst",
          content: "路由判断",
        })}
      />,
    );
    expect(screen.getByText("Analyst")).toBeInTheDocument();
  });

  it("renders provider stream content with escaped and single newlines as readable blocks", () => {
    const entry = makeEntry({
      type: "provider_stream",
      role: "author",
      content: "# Story Spec\\n## 范围\\n实现爬楼梯问题\\n- n=1\\n- n=2",
    });

    render(<ProviderStreamEntry entry={entry} />);

    expect(screen.getByRole("heading", { name: "Story Spec" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "范围" })).toBeInTheDocument();
    expect(screen.getByText(/实现爬楼梯问题/)).toBeInTheDocument();
    expect(screen.getByText(/n=1/)).toBeInTheDocument();
  });

  it("decodes html entity escaped provider stream JSON-like content", () => {
    const { container } = render(
      <ProviderStreamEntry
        entry={makeEntry({
          type: "provider_stream",
          role: "author",
          content: "{&quot;required_gates&quot;:[&quot;cmd_check&quot;]}",
        })}
      />,
    );

    expect(container.textContent).toContain('"required_gates": [');
    expect(container.textContent).toContain('"cmd_check"');
    expect(container.textContent).not.toContain("&quot;");
  });

  it("renders provider stream markdown semantics inside message bubbles", () => {
    const entry = makeEntry({
      type: "provider_stream",
      role: "author",
      content:
        "## 范围\n\n- 支持 **粗体** 和 `inline code`\n- 查看 [设计说明](https://example.com/design)",
    });

    render(<ProviderStreamEntry entry={entry} />);

    expect(screen.getByRole("heading", { name: "范围" })).toBeInTheDocument();
    const list = screen.getByRole("list");
    expect(list).toBeInTheDocument();
    expect(within(list).getAllByRole("listitem")[0]).toHaveTextContent(
      "支持 粗体 和 inline code",
    );
    expect(screen.getByText("粗体").tagName).toBe("STRONG");
    expect(screen.getByText("inline code").tagName).toBe("CODE");
    expect(screen.getByRole("link", { name: "设计说明" })).toHaveAttribute(
      "href",
      "https://example.com/design",
    );
  });

  it("renders tester plan summaries as readable markdown", () => {
    render(
      <ProviderStreamEntry
        entry={makeEntry({
          id: "tester-plan",
          type: "provider_stream",
          role: "tester",
          content:
            "## Tester 测试计划\n\nunit plan\n\n### 步骤\n- unit · Unit · required · low\n  - 证据预期：unit evidence",
          node_id: "coding_node_0003",
          metadata: {
            phase: "test_plan",
            role_run_id: "coding_role_run_0001",
          },
        })}
      />,
    );

    expect(screen.getByRole("heading", { name: "Tester 测试计划" })).toBeInTheDocument();
    expect(screen.getByText(/unit plan/)).toBeInTheDocument();
    expect(screen.getByText(/证据预期/)).toBeInTheDocument();
  });

  it("formats raw tester TestPlan JSON streams instead of showing escaped JSON", () => {
    const testPlanJson = JSON.stringify({
      summary: "覆盖 work item 的 provider 门禁变更",
      context_warnings: [],
      assumptions: [],
      steps: [
        {
          id: "step_001_gate",
          title: "验证 Provider 门禁",
          intent: "检查不可用 provider 不会被静默默认使用",
          required: true,
          tool: "run_command",
          risk_level: "high",
          command_or_tool_input: { command: "cargo test --locked --lib provider_gate" },
          evidence_expectation: "provider_unavailable / provider_fallback 契约稳定",
          related_requirements: ["REQ-014"],
          related_design_constraints: ["DEC-014"],
          related_work_item_tasks: ["TASK-007"],
        },
      ],
    });

    const { container } = render(
      <ProviderStreamEntry
        entry={makeEntry({
          id: "tester-raw-json",
          type: "provider_stream",
          role: "tester",
          content: testPlanJson,
          metadata: {
            phase: "plan_tests",
          },
        })}
      />,
    );

    expect(screen.getByText("Tester 测试计划")).toBeInTheDocument();
    expect(screen.getByText(/覆盖 work item 的 provider 门禁变更/)).toBeInTheDocument();
    expect(screen.getByText(/step_001_gate/)).toBeInTheDocument();
    expect(screen.getByText(/REQ-014/)).toBeInTheDocument();
    expect(screen.getByText(/TASK-007/)).toBeInTheDocument();
    expect(container.textContent).not.toContain("&quot;");
    expect(container.textContent).not.toContain('"summary"');
  });

  it("formats HTML entity escaped tester TestPlan JSON streams", () => {
    const testPlanJson = JSON.stringify({
      summary: "针对本次 diff 生成后端 Provider 依赖自检验证计划",
      context_warnings: ["context_truncated"],
      assumptions: ["当前阶段仅生成 TestPlan，不直接执行命令。"],
      steps: [
        {
          id: "step_001_rules_context",
          title: "读取仓库验证规则",
          intent: "确认本 worktree 的语言、TDD、Rust 构建测试命令和禁止项。",
          required: true,
          tool: "read_file",
          risk_level: "low",
          command_or_tool_input: { paths: ["CLAUDE.md"] },
          evidence_expectation: "读取到规则文件内容。",
          related_requirements: ["REQ-provider-gate"],
          related_design_constraints: ["DEC-provider-gate"],
          related_work_item_tasks: ["TASK-007"],
        },
      ],
    }).replaceAll('"', "&quot;");

    const { container } = render(
      <ProviderStreamEntry
        entry={makeEntry({
          id: "tester-entity-json",
          type: "provider_stream",
          role: "tester",
          content: testPlanJson,
          metadata: {
            phase: "plan_tests",
          },
        })}
      />,
    );

    expect(screen.getByText("Tester 测试计划")).toBeInTheDocument();
    expect(screen.getByText(/针对本次 diff 生成后端 Provider 依赖自检验证计划/)).toBeInTheDocument();
    expect(screen.getByText(/step_001_rules_context/)).toBeInTheDocument();
    expect(screen.getByText(/TASK-007/)).toBeInTheDocument();
    expect(container.textContent).not.toContain("&quot;");
    expect(container.textContent).not.toContain('"summary"');
  });

  it("hides reviewer trailing JSON contract from provider stream bubbles", () => {
    const entry = makeEntry({
      type: "provider_stream",
      role: "reviewer",
      content:
        '当前产物可以进入下一阶段。\n\n```json\n{"verdict":"pass","summary":"审核通过","findings":[]}\n```',
    });

    const { container } = render(<ProviderStreamEntry entry={entry} />);

    expect(screen.getByText("当前产物可以进入下一阶段。")).toBeInTheDocument();
    expect(container.textContent).not.toContain('"verdict"');
    expect(container.textContent).not.toContain('"findings"');
  });

  it("keeps work item draft structured output out of the main message bubble", () => {
    const entry = makeEntry({
      type: "provider_stream",
      role: "author",
      content:
        '## Work Item Draft\n\n实现 Provider 依赖探测。\n\n<ARIA_STRUCTURED_OUTPUT nonce="draft-nonce">\n{"outline_id":"outline_backend_probe","draft_id":"draft_004","title":"后端 - Provider 探测","handoff_summary":"给 installer 消费 refresh_into。"}\n</ARIA_STRUCTURED_OUTPUT nonce="draft-nonce">',
    });

    const { container } = render(<ProviderStreamEntry entry={entry} />);

    expect(screen.getByRole("heading", { name: "Work Item Draft" })).toBeInTheDocument();
    expect(screen.getByText("实现 Provider 依赖探测。")).toBeInTheDocument();
    expect(screen.getByText("结构化输出")).toBeInTheDocument();
    expect(screen.getByText("draft_004")).toBeInTheDocument();
    expect(screen.getByText("outline_backend_probe")).toBeInTheDocument();
    expect(container.textContent).not.toContain("ARIA_STRUCTURED_OUTPUT");
  });

  it("summarizes work item review structured output before collapsed raw JSON", () => {
    const entry = makeEntry({
      type: "provider_stream",
      role: "reviewer",
      content:
        '发现当前 draft 仍需返修。\n\n<ARIA_STRUCTURED_OUTPUT nonce="review-nonce">\n{"verdict":"pass","review_scope":"item","target_outline_id":"outline_backend_probe","draft_id":"draft_004","summary":"需要修正写域。","findings":[{"severity":"strong_recommend_fix","message":"写域包含 installer 文件","evidence":"exclusive_write_scopes 包含 src/web/provider_installer.rs","impact":"会导致后续 item 边界漂移","required_action":"移除 installer 写域"}]}\n</ARIA_STRUCTURED_OUTPUT nonce="review-nonce">',
    });

    const { container } = render(<ProviderStreamEntry entry={entry} />);

    expect(screen.getByText("发现当前 draft 仍需返修。")).toBeInTheDocument();
    expect(screen.getByText("结构化输出")).toBeInTheDocument();
    expect(screen.getByText("verdict: pass")).toBeInTheDocument();
    expect(screen.getByText("最高严重度: strong_recommend_fix")).toBeInTheDocument();
    expect(screen.getByText("需要修正写域。")).toBeInTheDocument();
    const details = container.querySelector("details");
    expect(details).not.toBeNull();
    expect(details?.open).toBe(false);
    expect(container.textContent).not.toContain("ARIA_STRUCTURED_OUTPUT");
  });

  it("does not inject raw HTML or unsafe markdown links in message bubbles", () => {
    const entry = makeEntry({
      type: "provider_stream",
      role: "author",
      content:
        '正文\n\n<script>alert("x")</script>\n\n<img src=x onerror=alert(1)>\n\n[危险链接](javascript:alert(1))',
    });

    const { container } = render(<ProviderStreamEntry entry={entry} />);

    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByText('<script>alert("x")</script>')).toBeInTheDocument();
    expect(screen.getByText("<img src=x onerror=alert(1)>")).toBeInTheDocument();
    expect(screen.getByText("危险链接")).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "危险链接" })).not.toBeInTheDocument();
  });

  it("breaks long provider prose into sentence lines when the provider streams one dense line", () => {
    const entry = makeEntry({
      type: "provider_stream",
      role: "author",
      content:
        "仓库规则已确认。 我现在进入红灯阶段。 红灯已成立：python -m unittest discover -s tests 因 ModuleNotFoundError 失败。 绿色阶段测试已通过。",
    });

    render(<ProviderStreamEntry entry={entry} />);

    const prose = screen.getByText(/仓库规则已确认/);
    expect(prose.textContent).toBe(
      "仓库规则已确认。\n我现在进入红灯阶段。\n红灯已成立：python -m unittest discover -s tests 因 ModuleNotFoundError 失败。\n绿色阶段测试已通过。",
    );
  });

  it("collapses very large markdown content until expanded", async () => {
    const huge = "intro\n" + "line\n".repeat(30_000) + "\n# Full Title\nOnly after expansion";

    render(<MarkdownContent content={huge} />);

    expect(screen.getByText(/内容较长/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /展开全文/ })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Full Title" })).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /展开全文/ }));

    expect(screen.getByRole("heading", { name: "Full Title" })).toBeInTheDocument();
    expect(screen.getByText("Only after expansion")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /收起全文/ })).toBeInTheDocument();
  });

  it("renders execution event entries with command detail", () => {
    const entry = makeEntry({
      type: "execution_event",
      role: "system",
      content: "读取认证模块 · exit code 0",
      metadata: { command: "sed -n '1,120p' src/auth.rs", output: "ok" },
    });

    render(<ExecutionEventEntry entry={entry} />);

    expect(screen.getByText("执行事件")).toBeInTheDocument();
    expect(screen.getByText("读取认证模块 · exit code 0")).toBeInTheDocument();
    expect(screen.getByText("sed -n '1,120p' src/auth.rs")).toBeInTheDocument();
    expect(screen.getByText("ok")).toBeInTheDocument();
  });

  it("labels provider prompt execution entries explicitly", () => {
    const entry = makeEntry({
      type: "execution_event",
      role: "system",
      content: "WorkItemPlan Outline Planner · Provider Prompt · 42KB",
      metadata: { title: "Provider Prompt" },
      content_ref: { kind: "provider_prompt", nodeId: "node-author-1" },
    });

    render(<ExecutionEventEntry entry={entry} />);

    expect(screen.getByText("Provider Prompt")).toBeInTheDocument();
    expect(screen.getByText("WorkItemPlan Outline Planner · Provider Prompt · 42KB")).toBeInTheDocument();
  });

  it("renders permission request entries and emits response actions", () => {
    const onRespond = vi.fn();
    const entry = makeEntry({
      type: "permission_request",
      role: "system",
      content: "shell · cargo test",
      metadata: {
        request_id: "permission-1",
        request: {
          tool_name: "shell",
          description: "cargo test",
          risk_level: "medium",
        },
        risk_level: "medium",
      },
    });

    render(<PermissionRequestEntry entry={entry} onRespond={onRespond} />);

    fireEvent.click(screen.getByRole("button", { name: "允许" }));

    expect(screen.getByText("shell")).toBeInTheDocument();
    expect(screen.getByText("cargo test")).toBeInTheDocument();
    expect(onRespond).toHaveBeenCalledWith(entry, true);
  });

  it("renders permission response entries as compact tags", () => {
    const entry = makeEntry({
      type: "permission_response",
      role: "user",
      content: "已允许 shell",
      metadata: { approved: true },
    });

    render(<PermissionResponseEntry entry={entry} />);

    expect(screen.getByText("已允许 shell")).toBeInTheDocument();
  });

  it("renders error entries with code metadata", () => {
    const entry = makeEntry({
      type: "error",
      role: "system",
      content: "阶段不允许",
      metadata: { code: "INVALID_MESSAGE_FOR_STAGE" },
    });

    render(<ErrorEntry entry={entry} />);

    expect(screen.getByText("错误 INVALID_MESSAGE_FOR_STAGE")).toBeInTheDocument();
    expect(screen.getByText("阶段不允许")).toBeInTheDocument();
  });

  it("renders analyst verdict entries with fix hints and human questions", () => {
    render(
      <ChatEntryRenderer
        entry={makeEntry({
          type: "analyst_verdict",
          role: "analyst",
          content: "测试仍失败",
          metadata: {
            verdict: "needs_fix",
            fix_hints: ["补充 climb_stairs 动态规划实现", "覆盖 n=10"],
          },
        })}
      />,
    );

    expect(screen.getByTestId("analyst-verdict-entry")).toHaveTextContent("需要修复");
    expect(screen.getByTestId("analyst-verdict-entry")).toHaveTextContent("测试仍失败");
    expect(screen.getByText("补充 climb_stairs 动态规划实现")).toBeInTheDocument();

    render(
      <ChatEntryRenderer
        entry={makeEntry({
          id: "entry-2",
          type: "analyst_verdict",
          role: "analyst",
          content: "需要人工补充",
          metadata: {
            verdict: "needs_human_input",
            questions: ["n 的输入范围是多少？"],
          },
        })}
      />,
    );

    expect(screen.getByText("需要人工输入")).toBeInTheDocument();
    expect(screen.getByText("n 的输入范围是多少？")).toBeInTheDocument();
  });

  it("renders analyst routing decisions with parse diagnostics", () => {
    render(
      <ChatEntryRenderer
        entry={makeEntry({
          type: "analyst_verdict",
          role: "analyst",
          content: "Analyst 输出不是有效 JSON，已转人工确认。",
          metadata: {
            verdict: "needs_human_input",
            structured_verdict: "human_required",
            next_stage: "human_gate",
            reason: "Analyst 输出不是有效 JSON，已转人工确认。",
            evidence_refs: ["testing_report_0001.json"],
            raw_provider_output_refs: ["provider-raw/rework/analyst_decision_0001.txt"],
            parse_error: "key must be a string at line 1 column 2",
          },
        })}
      />,
    );

    const entry = screen.getByTestId("analyst-verdict-entry");
    expect(entry).toHaveTextContent("human_required");
    expect(entry).toHaveTextContent("human_gate");
    expect(entry).toHaveTextContent("key must be a string at line 1 column 2");
    expect(entry).toHaveTextContent("provider-raw/rework/analyst_decision_0001.txt");
  });

  it("dispatches entries through the renderer", () => {
    const onRespond = vi.fn();
    const entry = makeEntry({
      type: "permission_request",
      role: "system",
      content: "shell · cargo test",
      metadata: {
        request: {
          tool_name: "shell",
          description: "cargo test",
          risk_level: "medium",
        },
        risk_level: "medium",
      },
    });

    render(<ChatEntryRenderer entry={entry} onPermissionResponse={onRespond} />);

    fireEvent.click(screen.getByRole("button", { name: "拒绝" }));

    expect(onRespond).toHaveBeenCalledWith(entry, false);
  });
});

function makeEntry(overrides: Partial<ChatEntry>): ChatEntry {
  return {
    id: "entry-1",
    type: "context_note",
    role: "user",
    content: "",
    timestamp: "2026-05-21T10:00:00Z",
    ...overrides,
  } as ChatEntry;
}
