import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryRenderer } from "../ChatEntryRenderer";
import { ArtifactUpdateEntry } from "./ArtifactUpdateEntry";
import { GatePromptEntry } from "./GatePromptEntry";
import { HumanDecisionEntry } from "./HumanDecisionEntry";
import { ReviewVerdictEntry } from "./ReviewVerdictEntry";
import { StageChangeEntry } from "./StageChangeEntry";
import { StartGenerationEntry } from "./StartGenerationEntry";

describe("chat workspace p1 entries", () => {
  it("renders start generation entries with provider snapshot", () => {
    const entry = makeEntry({
      type: "start_generation",
      role: "system",
      content: "开始生成",
      metadata: {
        snapshot: { author: "claude_code", reviewer: "codex", review_rounds: 2 },
      },
    });

    render(<StartGenerationEntry entry={entry} />);

    expect(screen.getByText("开始生成")).toBeInTheDocument();
    expect(screen.getByText("Author: claude_code")).toBeInTheDocument();
    expect(screen.getByText("Reviewer: codex · 2 轮")).toBeInTheDocument();
  });

  it("renders stage change entries", () => {
    const entry = makeEntry({
      type: "stage_change",
      role: "system",
      content: "阶段变更 -> running",
    });

    render(<StageChangeEntry entry={entry} />);

    expect(screen.getByText("阶段变更 -> running")).toBeInTheDocument();
  });

  it("renders artifact update entries", () => {
    const entry = makeEntry({
      type: "artifact_update",
      role: "system",
      content: "产物已更新 -> v2",
      metadata: { version: 2 },
    });

    render(<ArtifactUpdateEntry entry={entry} />);

    expect(screen.getByText("产物已更新 -> v2")).toBeInTheDocument();
  });

  it("renders review verdict entries and path actions", () => {
    const onSelectPath = vi.fn();
    const entry = makeEntry({
      type: "review_verdict",
      role: "reviewer",
      content: "需要补充失败路径",
      metadata: { verdict: "revise", comments: "缺少错误处理" },
    });

    render(<ReviewVerdictEntry entry={entry} onSelectPath={onSelectPath} />);
    fireEvent.click(screen.getByRole("button", { name: "补充上下文后修订" }));
    expect(onSelectPath).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText("补充返修上下文"), {
      target: { value: "请补充错误码说明" },
    });
    fireEvent.click(screen.getByRole("button", { name: "提交补充并修订" }));

    expect(screen.getByText("建议返修")).toBeInTheDocument();
    expect(screen.getByText("需要补充失败路径")).toBeInTheDocument();
    expect(onSelectPath).toHaveBeenCalledWith("revise-with-context", "请补充错误码说明");
  });

  it("groups review findings by required and optional severity", () => {
    const entry = makeEntry({
      type: "review_verdict",
      role: "reviewer",
      content: "存在需要解决和可选建议",
      metadata: {
        verdict: "revise",
        summary: "存在分级 findings",
        review_gate: "requires_revision",
        findings: [
          {
            severity: "must_fix",
            message: "缺少验证命令",
            evidence: "未出现验证命令段落",
            impact: "Coding Workspace 无法执行验收",
            required_action: "补充验证命令",
          },
          {
            severity: "optional",
            message: "可以补充复杂度说明",
            evidence: "主体方案完整",
            impact: "不影响下一阶段",
            required_action: "后续优化时补充",
          },
        ],
      },
    });

    render(<ReviewVerdictEntry entry={entry} />);

    expect(screen.getByText("需要解决")).toBeInTheDocument();
    expect(screen.getByText("缺少验证命令")).toBeInTheDocument();
    expect(screen.getByText("补充验证命令")).toBeInTheDocument();
    expect(screen.getByText("可选建议")).toBeInTheDocument();
    expect(screen.getByText("可以补充复杂度说明")).toBeInTheDocument();
  });

  it("labels optional-only review verdicts as confirmable", () => {
    const entry = makeEntry({
      type: "review_verdict",
      role: "reviewer",
      content: "仅有可选建议",
      metadata: {
        verdict: "needs_human",
        summary: "可确认当前版本",
        review_gate: "user_confirm_allowed",
        findings: [
          {
            severity: "suggestion",
            message: "建议优化措辞",
            evidence: "内容已覆盖主路径",
            impact: "不影响下一阶段",
            required_action: "可后续优化",
          },
        ],
      },
    });

    render(<ReviewVerdictEntry entry={entry} />);

    expect(screen.getAllByText("可确认当前版本").length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText("建议优化措辞")).toBeInTheDocument();
    expect(screen.queryByText("需要解决")).not.toBeInTheDocument();
  });

  it("labels unstructured review intent as user triage required", () => {
    const entry = makeEntry({
      type: "review_verdict",
      role: "reviewer",
      content: "Reviewer 要求返修但没有结构化 finding",
      metadata: {
        verdict: "needs_human",
        summary: "返修意图需要人工判断",
        review_gate: "user_triage_required",
      },
    });

    render(<ReviewVerdictEntry entry={entry} />);

    expect(screen.getByText("需要判断 reviewer 意图")).toBeInTheDocument();
    expect(screen.queryByText("可确认当前版本")).not.toBeInTheDocument();
  });

  it("renders gate prompt entries and human decision actions", () => {
    const onDecision = vi.fn();
    const entry = makeEntry({
      type: "gate_prompt",
      role: "system",
      content: "等待人工确认",
      metadata: { summary: "可以进入人工确认" },
    });

    render(<GatePromptEntry entry={entry} onDecision={onDecision} />);
    fireEvent.click(screen.getByRole("button", { name: "确认产物" }));
    fireEvent.click(screen.getByRole("button", { name: "终止" }));

    expect(screen.getByText("等待人工确认")).toBeInTheDocument();
    expect(screen.getByText("可以进入人工确认")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "修改" })).not.toBeInTheDocument();
    expect(onDecision).toHaveBeenNthCalledWith(1, "confirm");
    expect(onDecision).toHaveBeenNthCalledWith(2, "terminate");
  });

  it("renders needs_human gate prompt as clarification instead of artifact approval", () => {
    const onDecision = vi.fn();
    const entry = makeEntry({
      type: "gate_prompt",
      role: "system",
      content: "需要人工确认",
      metadata: { verdict: "needs_human", summary: "需要先确认弹窗触发时机" },
    });

    render(<GatePromptEntry entry={entry} onDecision={onDecision} />);
    fireEvent.click(screen.getByRole("button", { name: "提交人工确认" }));

    expect(screen.getAllByText("需要人工确认").length).toBeGreaterThanOrEqual(1);
    expect(screen.queryByRole("button", { name: "确认产物" })).not.toBeInTheDocument();
    expect(onDecision).toHaveBeenCalledWith("confirm");
  });

  it("renders user confirm wording when review gate allows current version", () => {
    const onDecision = vi.fn();
    const entry = makeEntry({
      type: "gate_prompt",
      role: "system",
      content: "等待人工确认",
      metadata: {
        verdict: "needs_human",
        review_gate: "user_confirm_allowed",
        summary: "仅有可选建议",
      },
    });

    render(<GatePromptEntry entry={entry} onDecision={onDecision} />);
    fireEvent.click(screen.getByRole("button", { name: "确认使用当前版本" }));

    expect(onDecision).toHaveBeenCalledWith("confirm");
  });

  it("renders adopt-suggestion action when review gate allows confirmation", () => {
    const onDecision = vi.fn();
    const entry = makeEntry({
      type: "gate_prompt",
      role: "system",
      content: "等待人工确认",
      metadata: {
        verdict: "needs_human",
        review_gate: "user_confirm_allowed",
        summary: "仅有可选建议",
        findings: [
          {
            severity: "optional",
            message: "建议补充说明",
            evidence: "当前版本可用",
            impact: "不影响下一阶段",
            required_action: "补充说明段落",
          },
        ],
      },
    });

    render(<GatePromptEntry entry={entry} onDecision={onDecision} />);
    fireEvent.click(screen.getByRole("button", { name: "采纳建议并返修" }));

    expect(screen.getByRole("button", { name: "确认使用当前版本" })).toBeInTheDocument();
    expect(onDecision).toHaveBeenCalledWith(
      "request-change",
      expect.objectContaining({
        description: expect.stringContaining("建议补充说明"),
      }),
    );
    expect(onDecision.mock.calls[0][1].description).toContain("补充说明段落");
  });

  it("renders reviewer-intent revision action when user triage is required", () => {
    const onDecision = vi.fn();
    const entry = makeEntry({
      type: "gate_prompt",
      role: "system",
      content: "需要人工确认",
      metadata: {
        verdict: "needs_human",
        review_gate: "user_triage_required",
        summary: "Reviewer 要求返修但没有结构化 finding",
        comments: "请补齐异常路径说明。",
      },
    });

    render(<GatePromptEntry entry={entry} onDecision={onDecision} />);
    fireEvent.click(screen.getByRole("button", { name: "按 reviewer 意见返修" }));
    fireEvent.click(screen.getByRole("button", { name: "确认当前版本" }));

    expect(screen.getByText("需要判断 reviewer 意图")).toBeInTheDocument();
    expect(onDecision).toHaveBeenNthCalledWith(
      1,
      "request-change",
      expect.objectContaining({
        description: expect.stringContaining("请补齐异常路径说明。"),
      }),
    );
    expect(onDecision).toHaveBeenNthCalledWith(2, "confirm");
  });

  it.each([
    ["confirm", "已确认"],
    ["request-change", "已要求修改"],
    ["terminate", "已终止"],
  ] as const)("renders resolved gate prompt entries as %s", (resolution, label) => {
    const onDecision = vi.fn();
    const entry = makeEntry({
      type: "gate_prompt",
      role: "system",
      content: "等待人工确认",
      resolved: true,
      resolution,
    });

    render(<GatePromptEntry entry={entry} onDecision={onDecision} />);

    expect(screen.getByText(label)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "确认产物" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "终止" })).not.toBeInTheDocument();
    expect(onDecision).not.toHaveBeenCalled();
  });

  it("renders human decision entries", () => {
    const entry = makeEntry({
      type: "human_decision",
      role: "user",
      content: "补充失败路径",
    });

    render(<HumanDecisionEntry entry={entry} />);

    expect(screen.getByText("补充失败路径")).toBeInTheDocument();
  });

  it("dispatches p1 entries through the renderer", () => {
    const onSelectPath = vi.fn();
    const entry = makeEntry({
      type: "review_verdict",
      role: "reviewer",
      content: "需要补充失败路径",
      metadata: { verdict: "revise" },
    });

    render(<ChatEntryRenderer entry={entry} onSelectRevisionPath={onSelectPath} />);
    fireEvent.click(screen.getByRole("button", { name: "接受修订建议" }));

    expect(onSelectPath).toHaveBeenCalledWith("revise");
  });
});

function makeEntry(overrides: Partial<ChatEntry>): ChatEntry {
  return {
    id: "entry-1",
    type: "start_generation",
    role: "system",
    content: "",
    timestamp: "2026-05-21T10:00:00Z",
    ...overrides,
  } as ChatEntry;
}
