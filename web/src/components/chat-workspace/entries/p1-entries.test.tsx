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

    expect(screen.getByText("建议返修")).toBeInTheDocument();
    expect(screen.getByText("需要补充失败路径")).toBeInTheDocument();
    expect(onSelectPath).toHaveBeenCalledWith("revise-with-context");
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
    fireEvent.click(screen.getByRole("button", { name: "确认" }));
    fireEvent.click(screen.getByRole("button", { name: "终止" }));

    expect(screen.getByText("等待人工确认")).toBeInTheDocument();
    expect(screen.getByText("可以进入人工确认")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "修改" })).not.toBeInTheDocument();
    expect(onDecision).toHaveBeenNthCalledWith(1, "confirm");
    expect(onDecision).toHaveBeenNthCalledWith(2, "terminate");
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
    expect(screen.queryByRole("button", { name: "确认" })).not.toBeInTheDocument();
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
