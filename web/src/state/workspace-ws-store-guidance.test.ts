import { describe, expect, it } from "vitest";
import { refreshPreparedContextAuthorGuidance } from "./workspace-ws-store-guidance";

const KIMI_GUIDANCE =
  "当前 author provider 是 Kimi Code；Supervised 模式下工具操作必须使用结构化 permission request（逐工具审批），并等待用户审批或回答。需要向用户确认时，必须使用结构化 AskUserQuestion（用户可选择选项或自由输入）并等待回答。禁止输出文本 A/B/C 选择题作为交互替代。";

describe("workspace WebSocket store guidance", () => {
  it("refreshes prepared Kimi Code context with permission and choice guidance", () => {
    const messages = [
      {
        id: "message_001",
        role: "system",
        content:
          "Workspace 生成任务已准备\n\n[workflow_discipline]\n基础纪律\n当前 author provider 是 Codex；旧 guidance\n\n[output_schema]\nSchema",
        created_at: "2026-06-30T00:00:00Z",
      },
    ];

    const refreshed = refreshPreparedContextAuthorGuidance(messages, "kimi_code");

    expect(refreshed).not.toBe(messages);
    expect(refreshed[0]?.content).toContain(KIMI_GUIDANCE);
    expect(refreshed[0]?.content).toContain("[output_schema]\nSchema");
  });
});
