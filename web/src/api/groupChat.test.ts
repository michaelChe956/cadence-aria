import { afterEach, describe, expect, it, vi } from "vitest";
import {
  addGroupChatRole,
  createGroupChatSession,
  finalizeGroupChat,
  getGroupChatSession,
  getGroupChatTriageProvider,
  getSpecGenerationMode,
  sendGroupChatMessage,
  updateGroupChatTriageProvider,
  updateSpecGenerationMode,
} from "./groupChat";

const session = {
  id: "session/1",
  project_id: "project-1",
  issue_id: "issue-1",
  status: "active",
  roles: [],
  artifact_lines: [],
  created_at: "2026-08-18T00:00:00Z",
  updated_at: "2026-08-18T00:00:00Z",
};

describe("群聊 API client", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("封装会话、消息、角色与定稿端点，并保留 JSON body", async () => {
    const calls: Array<{ url: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ url: String(input), init });
        const body =
          calls.length === 1
            ? { ...session, timeline: [] }
            : calls.length === 2
              ? { summary: { appended_seqs: [1], held_events: 0, circuit_break: false, no_one_notice: false }, session }
              : calls.length === 3
                ? session
                : { event: { type: "system_notice", text: "done" }, session };
        return new Response(JSON.stringify(body), { status: 200 });
      }),
    );

    await createGroupChatSession({ project_id: "project-1", issue_id: "issue-1" });
    await sendGroupChatMessage("session/1", {
      text: "请讨论",
      mentions: ["role-1"],
      draft_slot: "story/slot",
    });
    await addGroupChatRole("session/1", {
      role_key: "author",
      provider: "fake",
      display_name: "作者",
      permission_mode: "supervised",
    });
    await finalizeGroupChat("session/1", {
      line_kind: "story_spec",
      included_slots_override: ["story/slot"],
      confirmed_by: "user-1",
    });

    expect(calls.map(({ url }) => url)).toEqual([
      "/api/group-chat/sessions",
      "/api/group-chat/sessions/session%2F1/messages",
      "/api/group-chat/sessions/session%2F1/roles",
      "/api/group-chat/sessions/session%2F1/finalize",
    ]);
    expect(calls[0].init?.method).toBe("POST");
    expect(JSON.parse(String(calls[1].init?.body))).toEqual({
      text: "请讨论",
      mentions: ["role-1"],
      draft_slot: "story/slot",
    });
  });

  it("编码时间线游标、triage provider 和模式设置", async () => {
    const calls: Array<{ url: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ url: String(input), init });
        const body = calls.length === 1 ? { ...session, timeline: [] } : calls.length === 2 ? { provider: null } : calls.length === 3 ? { provider: "codex" } : calls.length === 4 ? "pipeline" : "group_chat";
        return new Response(JSON.stringify(body), { status: 200 });
      }),
    );

    await getGroupChatSession("session/1", { afterSeq: 7, limit: 50 });
    await getGroupChatTriageProvider("session/1");
    await updateGroupChatTriageProvider("session/1", "codex");
    await getSpecGenerationMode();
    await updateSpecGenerationMode("group_chat");

    expect(calls.map(({ url }) => url)).toEqual([
      "/api/group-chat/sessions/session%2F1?after_seq=7&limit=50",
      "/api/group-chat/sessions/session%2F1/settings/triage-provider",
      "/api/group-chat/sessions/session%2F1/settings/triage-provider",
      "/api/settings/spec-generation-mode",
      "/api/settings/spec-generation-mode",
    ]);
    expect(calls[2].init?.method).toBe("PUT");
    expect(JSON.parse(String(calls[2].init?.body))).toEqual({ provider: "codex" });
    expect(JSON.parse(String(calls[4].init?.body))).toBe("group_chat");
  });

  it("非 2xx 响应转换为 ApiRequestError", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        new Response(JSON.stringify({ code: "not_found", message: "不存在", details: {} }), {
          status: 404,
        }),
      ),
    );

    await expect(getGroupChatSession("missing")).rejects.toMatchObject({
      name: "ApiRequestError",
      code: "not_found",
      message: "不存在",
    });
  });
});
