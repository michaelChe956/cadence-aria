import { renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceWsApi } from "../hooks/useWorkspaceWs";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import type { WorkspaceHumanAction } from "../api/types";
import { useCockpitGateConfirm } from "./useCockpitGateConfirm";

/**
 * P2（REQ-WSC-15 归因扩展，REST 通道收口）：owner=server 会话的「排除此项」
 * 经 sendHumanActionRest → POST /api/workspace-sessions/{id}/human-actions 出站。
 * 测试按生产真实形态打桩全局 fetch（不打桩 api/client、不模拟 WS 的
 * WsOutMessage::Error 帧）：
 * - 200/409 回执体 `{command_id, state, gate_id}`
 *   （src/web/handlers/workspace_human_action.rs HumanActionStatus）；
 * - 排除落盘失败 → ApiError 体 `{code: human_action_engine_error, message,
 *   details}`（error.rs IntoResponse → Json(self)，500）；
 * - 引擎语义拒绝 422 `human_action_rejected`；
 * - 网络层异常 → fetch 直接 reject。
 */
const GATE_KEY = "turn_0001:gate-prompt";

const EXCLUSION_ACTION: WorkspaceHumanAction = {
  type: "feedback",
  command_id: "cmd_exclusion_1",
  expected_gate_id: "gate_node_1",
  feedback: "",
  excluded_finding_ids: ["0123456789abcdef"],
};

const PLAIN_FEEDBACK_ACTION: WorkspaceHumanAction = {
  type: "feedback",
  command_id: "cmd_feedback_1",
  expected_gate_id: "gate_node_1",
  feedback: "把反馈收窄成一条可执行的修改后重新提交",
};

const ABANDON_ACTION: WorkspaceHumanAction = {
  type: "abandon",
  command_id: "cmd_abandon_1",
  expected_gate_id: "gate_node_1",
};

function httpReply(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function sendHumanActionRest(): (action: WorkspaceHumanAction) => boolean {
  const { result } = renderHook(() =>
    useCockpitGateConfirm({
      sessionId: "session_rest_attribution",
      workspaceWs: {} as WorkspaceWsApi,
    }),
  );
  return result.current.sendHumanActionRest;
}

/** 门卡点击「排除此项」时经 markLocalAction → recordGateLocalAction 落的归因线索。 */
function markExclusionClick() {
  useWorkspaceStore.getState().recordGateLocalAction(GATE_KEY, "exclusion");
}

describe("useCockpitGateConfirm sendHumanActionRest exclusion attribution", () => {
  beforeEach(() => {
    useWorkspaceStore.getState().reset();
    useWorkspaceStore.setState({ sessionId: "session_rest_attribution" });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("attributes an exclusion write failure (500 ApiError body) to the gate card", async () => {
    markExclusionClick();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        httpReply(500, {
          code: "human_action_engine_error",
          message: "排除项没有落盘：durable store unavailable",
          details: {
            command_id: "cmd_exclusion_1",
            session_id: "session_rest_attribution",
          },
        }),
      ),
    );
    const send = sendHumanActionRest();

    expect(send(EXCLUSION_ACTION)).toBe(true);

    // 与 WS WsOutMessage::Error 同一条归因通道：同一份失败事实与文案，
    // 门卡据此撤销乐观「已排除」并就地呈现服务端原文。
    await waitFor(() => {
      expect(useWorkspaceStore.getState().gateExclusionFailure).toMatchObject({
        code: "EXCLUSION_NOT_RECORDED",
        message: "排除项没有落盘：durable store unavailable",
        gateKey: GATE_KEY,
      });
    });
    // 双呈现：全局协议错误面仍如实亮（不互相吞）。
    expect(useWorkspaceStore.getState().protocolError).toMatchObject({
      code: "human_action_engine_error",
      message: "排除项没有落盘：durable store unavailable",
    });
    // 归因即消费：同一失败不得重复归因。
    expect(useWorkspaceStore.getState().lastGateLocalAction).toBeNull();
  });

  it("attributes a busy receipt (409 status body) to the gate card", async () => {
    markExclusionClick();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        httpReply(409, {
          command_id: "cmd_exclusion_1",
          state: "busy",
          gate_id: "gate_node_1",
        }),
      ),
    );
    const send = sendHumanActionRest();

    send(EXCLUSION_ACTION);

    await waitFor(() => {
      expect(useWorkspaceStore.getState().gateExclusionFailure).toMatchObject({
        code: "EXCLUSION_NOT_RECORDED",
        message: "人工命令被服务端拒绝（busy）",
        gateKey: GATE_KEY,
      });
    });
    expect(useWorkspaceStore.getState().protocolError).toMatchObject({
      code: "human_action_busy",
    });
  });

  it("attributes a network-level failure with the transport message", async () => {
    markExclusionClick();
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));
    const send = sendHumanActionRest();

    send(EXCLUSION_ACTION);

    await waitFor(() => {
      expect(useWorkspaceStore.getState().gateExclusionFailure).toMatchObject({
        code: "EXCLUSION_NOT_RECORDED",
        message: "Failed to fetch",
        gateKey: GATE_KEY,
      });
    });
    expect(useWorkspaceStore.getState().protocolError).toMatchObject({
      code: "human_action_failed",
    });
  });

  it("does not attribute non-exclusion REST failures even right after an exclusion click", async () => {
    // 归因线索在场（15s 窗口内），但失败的动作本身不带排除——不得归因。
    markExclusionClick();
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(
        httpReply(422, {
          code: "human_action_rejected",
          message: "INVALID_HUMAN_GATE_FEEDBACK · feedback must not be blank",
          details: {},
        }),
      )
      .mockResolvedValueOnce(
        httpReply(409, {
          command_id: "cmd_abandon_1",
          state: "busy",
          gate_id: "gate_node_1",
        }),
      );
    vi.stubGlobal("fetch", fetchMock);
    const send = sendHumanActionRest();

    send(PLAIN_FEEDBACK_ACTION);
    await waitFor(() => {
      expect(useWorkspaceStore.getState().protocolError).toMatchObject({
        code: "human_action_rejected",
      });
    });
    expect(useWorkspaceStore.getState().gateExclusionFailure).toBeNull();

    send(ABANDON_ACTION);
    await waitFor(() => {
      expect(useWorkspaceStore.getState().protocolError).toMatchObject({
        code: "human_action_busy",
      });
    });
    expect(useWorkspaceStore.getState().gateExclusionFailure).toBeNull();
  });

  it("does not attribute when the latest local gate action is feedback (kind criterion)", async () => {
    useWorkspaceStore.getState().recordGateLocalAction(GATE_KEY, "feedback");
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        httpReply(500, {
          code: "human_action_engine_error",
          message: "排除项没有落盘：durable store unavailable",
          details: {},
        }),
      ),
    );
    const send = sendHumanActionRest();

    send(EXCLUSION_ACTION);

    await waitFor(() => {
      expect(useWorkspaceStore.getState().protocolError).toMatchObject({
        code: "human_action_engine_error",
      });
    });
    expect(useWorkspaceStore.getState().gateExclusionFailure).toBeNull();
  });

  it("reconciles on accepted: clears the failure fact and the attribution marker", async () => {
    // 现实路径：上一轮排除失败落了失败事实，owner 重试点击重新记归因线索。
    markExclusionClick();
    useWorkspaceStore.setState({
      gateExclusionFailure: {
        code: "EXCLUSION_NOT_RECORDED",
        message: "排除项没有落盘：durable store unavailable",
        gateKey: GATE_KEY,
        at: Date.now(),
      },
    });
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        httpReply(200, {
          command_id: "cmd_exclusion_1",
          state: "accepted",
          gate_id: "gate_node_1",
        }),
      ),
    );
    const send = sendHumanActionRest();

    send(EXCLUSION_ACTION);

    // 受理 = 排除已 durable 落盘——与 WS human_gate_exclusions_accepted 回执
    // 同款对账：失败事实与归因线索都清空。
    await waitFor(() => {
      expect(useWorkspaceStore.getState().lastGateLocalAction).toBeNull();
    });
    expect(useWorkspaceStore.getState().gateExclusionFailure).toBeNull();
    expect(useWorkspaceStore.getState().protocolError).toBeNull();
  });

  it("keeps the exclusion failure fact when a non-exclusion action is accepted", async () => {
    const failure = {
      code: "EXCLUSION_NOT_RECORDED",
      message: "排除项没有落盘：durable store unavailable",
      gateKey: GATE_KEY,
      at: Date.now(),
    };
    useWorkspaceStore.setState({ gateExclusionFailure: failure });
    const fetchMock = vi.fn().mockResolvedValue(
      httpReply(200, {
        command_id: "cmd_feedback_1",
        state: "accepted",
        gate_id: "gate_node_1",
      }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const send = sendHumanActionRest();

    send(PLAIN_FEEDBACK_ACTION);

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalled();
    });
    // 纯反馈受理不确认排除——失败事实不得被清。
    expect(useWorkspaceStore.getState().gateExclusionFailure).toMatchObject(failure);
  });
});
