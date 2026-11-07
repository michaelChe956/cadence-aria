// REQ-WSC-26（gate-card-resolved-and-inflight-state，D1）：已收口的人工门卡不得
// 再以行动号召形态呈现。现场：会话 terminated 后门卡仍显示命令式标题「需要判断
// reviewer 意图」+ 完整可点选项块 +「选一个，把选项名原文作为补充指令发回去即可」
// + 「…需要你来定」meta 行，仅靠一个「已终止」徽章，owner 误以为还要操作。
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../../state/chat-entries";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import { useWorkspaceStore } from "../../../state/workspace-ws-store";
import { installWorkspaceStoreTestHooks } from "../../../state/workspace-ws-store.test-utils";
import { timelineNode } from "../../../state/workspace-cockpit-projection.test-fixtures";
import { GatePromptEntry } from "./GatePromptEntry";

const ownerGuidanceFindings = [
  {
    severity: "suggestion",
    message: "WI-004 的 write_policy.exclusive_scopes 仍声明 scripts/dev.sh",
    owner_question: "WI-004 还要不要继续负责 scripts/dev.sh 的改动？",
    owner_options: [
      { label: "不负责，只做验证", consequence: "WI-004 变成纯验证项，脚本改动另行安排" },
      { label: "继续负责", consequence: "把 scripts/ 纳入可写入范围，后续改动都记在 WI-004 上" },
    ],
  },
];

function triageGateEntry(overrides: Partial<ChatEntry> = {}): ChatEntry {
  return {
    id: "gate:entry",
    type: "gate_prompt",
    role: "system",
    content: "等待人工确认",
    timestamp: "2026-11-07T06:00:00Z",
    metadata: {
      action_facade: "typed",
      command_id: "cmd_1",
      action_block_reason: null,
      gate_identity: "turn_1",
      turn_id: "turn_1",
      gate_trigger: "native_human_required",
      remaining_budget: 2,
      review_gate: "user_triage_required",
      verdict: "needs_human",
      findings: ownerGuidanceFindings,
    },
    ...overrides,
  };
}

function actions(): CockpitActionFacade {
  return {
    confirm: vi.fn(() => true),
    confirmReview: vi.fn(() => true),
    feedback: vi.fn(() => true),
    terminate: vi.fn(() => true),
    advance: vi.fn(() => true),
    adoptReview: vi.fn(() => true),
    confirmBatch: vi.fn(async () => undefined),
    recoverCompile: vi.fn(async () => undefined),
    recoverCandidate: vi.fn(),
    retryInitialization: vi.fn(async () => undefined),
    confirmTakeover: vi.fn(async () => undefined),
    rebind: vi.fn(),
    sendBootstrapAction: vi.fn(),
  };
}

describe("REQ-WSC-26 已收口门卡去行动化", () => {
  installWorkspaceStoreTestHooks();

  describe("已终止的 triage 门卡", () => {
    function renderTerminated() {
      return render(
        <GatePromptEntry
          entry={triageGateEntry({ resolved: true, resolution: "terminate" })}
          actions={actions()}
        />,
      );
    }

    it("标题是陈述式，不再是命令式「需要判断 reviewer 意图」", () => {
      renderTerminated();

      expect(screen.getByText("本轮分诊已结束")).toBeVisible();
      expect(screen.queryByText("需要判断 reviewer 意图")).toBeNull();
      expect(screen.queryByText("需要人工确认")).toBeNull();
    });

    it("不渲染人话选项块、选项与作答指引", () => {
      renderTerminated();

      expect(screen.queryByTestId("owner-guidance")).toBeNull();
      expect(screen.queryAllByTestId("owner-option")).toHaveLength(0);
      expect(screen.queryByTestId("owner-question")).toBeNull();
      expect(screen.queryByTestId("owner-option-guidance")).toBeNull();
      expect(screen.queryByText(/选一个，把选项名原文作为补充指令发回去即可/)).toBeNull();
      expect(screen.queryByText("需要你决定一件事")).toBeNull();
    });

    it("不渲染「为什么找你」meta 行", () => {
      renderTerminated();

      expect(screen.queryByTestId("gate-meta")).toBeNull();
      expect(screen.queryByText(/需要你来定/)).toBeNull();
    });

    it("「已终止」徽章仍是独立 span 且文案逐字不变（getByText 精确匹配只命中它）", () => {
      renderTerminated();

      const badge = screen.getByText("已终止");
      expect(badge.tagName).toBe("SPAN");
    });

    it("徽章旁给出来源与后果说明（无被中断证据时不宣称修订被中断）", () => {
      renderTerminated();

      expect(screen.getByTestId("gate-resolution-note")).toHaveTextContent(
        "这一轮已终止，之前的选择未生效，无需再操作",
      );
      expect(screen.queryByText(/修订被中断/)).toBeNull();
    });

    it("时间线存在被中断修订的证据时，说明明说修订被中断、选择未生效", () => {
      useWorkspaceStore.getState().setTimelineNodesForTest([
        timelineNode({
          node_id: "timeline_node_009",
          node_type: "author_run",
          status: "aborted",
          summary: "运行已中止：修订被中断，你的选择未生效",
        }),
      ]);

      renderTerminated();

      expect(screen.getByTestId("gate-resolution-note")).toHaveTextContent(
        "这一轮修订被中断，你的选择未生效，无需再操作",
      );
      expect(screen.getByText("已终止").tagName).toBe("SPAN");
    });

    it("普通中止（运行已中止，非修订）不构成被中断修订的证据", () => {
      useWorkspaceStore.getState().setTimelineNodesForTest([
        timelineNode({
          node_id: "timeline_node_009",
          node_type: "author_run",
          status: "aborted",
          summary: "运行已中止",
        }),
      ]);

      renderTerminated();

      expect(screen.getByTestId("gate-resolution-note")).toHaveTextContent(
        "这一轮已终止，之前的选择未生效，无需再操作",
      );
    });

    it("既有已关元素保持关闭：chip / 指引 / why / 动作区", () => {
      renderTerminated();

      expect(screen.queryByText("需人工")).toBeNull();
      expect(screen.queryByTestId("gate-next-step")).toBeNull();
      expect(screen.queryByTestId("gate-why")).toBeNull();
      expect(screen.queryByTestId("gate-feedback-editor")).toBeNull();
      expect(screen.queryByRole("button", { name: "确认当前版本" })).toBeNull();
      expect(screen.queryByRole("button", { name: "终止此门" })).toBeNull();
    });
  });

  describe("其余收口形态的标题与说明逐字固定", () => {
    it("已确认：标题「本轮人工确认已结束」+ 说明，徽章「已确认」独立", () => {
      render(
        <GatePromptEntry
          entry={{
            ...triageGateEntry({ resolved: true, resolution: "confirm" }),
            metadata: { ...triageGateEntry().metadata, review_gate: "user_confirm_allowed" },
          }}
          actions={actions()}
        />,
      );

      expect(screen.getByText("本轮人工确认已结束")).toBeVisible();
      expect(screen.queryByText("需要人工确认")).toBeNull();
      expect(screen.getByTestId("gate-resolution-note")).toHaveTextContent(
        "已按当前版本确认，这一轮无需再操作",
      );
      expect(screen.getByText("已确认").tagName).toBe("SPAN");
      expect(screen.queryByTestId("owner-guidance")).toBeNull();
    });

    it("已要求修改：说明逐字，徽章「已要求修改」独立", () => {
      render(
        <GatePromptEntry
          entry={triageGateEntry({ resolved: true, resolution: "request-change" })}
          actions={actions()}
        />,
      );

      expect(screen.getByTestId("gate-resolution-note")).toHaveTextContent(
        "已要求修改，这一轮已结束，无需再操作",
      );
      expect(screen.getByText("已要求修改").tagName).toBe("SPAN");
    });

    it("批次确认门收口：标题「本轮发布确认已结束」，不再是命令式「确认发布整组 Work Items」", () => {
      render(
        <GatePromptEntry
          entry={{
            id: "gate:batch",
            type: "gate_prompt",
            role: "system",
            content: "Final Compile 已通过，等待发布确认",
            timestamp: "2026-11-07T06:00:00Z",
            metadata: {
              gate_identity: "node:batch_1",
              action_facade: "legacy",
              gate_kind: "batch_confirm",
            },
            resolved: true,
            resolution: "confirm",
          }}
          actions={actions()}
        />,
      );

      expect(screen.getByText("本轮发布确认已结束")).toBeVisible();
      expect(screen.queryByText("确认发布整组 Work Items")).toBeNull();
    });
  });

  describe("留档卡（superseded）", () => {
    function renderArchived() {
      return render(
        <GatePromptEntry
          entry={triageGateEntry({
            resolved: true,
            resolution: "superseded",
            metadata: {
              ...triageGateEntry().metadata,
              gate_archive_round: 1,
              gate_archive_note: "第 1 轮已提交反馈：请缩短标题",
            },
          })}
          actions={actions()}
        />,
      );
    }

    it("标题陈述式、不渲染人话选项块与作答指引", () => {
      renderArchived();

      expect(screen.queryByText("需要判断 reviewer 意图")).toBeNull();
      expect(screen.queryByTestId("owner-guidance")).toBeNull();
      expect(screen.queryByTestId("owner-option-guidance")).toBeNull();
    });

    it("保留既有 archive-note、「已留档」徽章与历史 meta 行", () => {
      renderArchived();

      expect(screen.getByTestId("gate-archive-note")).toHaveTextContent(
        "第 1 轮已提交反馈：请缩短标题",
      );
      expect(screen.getByTestId("gate-archive-badge")).toHaveTextContent("已留档");
      // 留档卡保留自己的历史 meta 行（创建时快照的预算事实，不读成当前授权）。
      expect(screen.getByTestId("gate-meta")).toHaveTextContent("人工返修还可做 2 次");
      expect(screen.queryByTestId("gate-resolution-note")).toBeNull();
    });
  });

  describe("活门卡不受影响（REQ-WSC-11 逐字不变）", () => {
    it("未收口的 triage 门卡仍渲染命令式标题与人话选项块、作答指引", () => {
      render(<GatePromptEntry entry={triageGateEntry()} actions={actions()} />);

      expect(screen.getByText("需要判断 reviewer 意图")).toBeVisible();
      expect(screen.getByTestId("owner-guidance")).toBeVisible();
      expect(screen.getAllByTestId("owner-option")).toHaveLength(2);
      expect(screen.getByTestId("owner-option-guidance")).toHaveTextContent(
        "把选项名原文作为补充指令发回去",
      );
      expect(screen.queryByTestId("gate-resolution-note")).toBeNull();
    });
  });
});
