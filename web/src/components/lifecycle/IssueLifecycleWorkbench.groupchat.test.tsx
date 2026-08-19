import { render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  getSpecGenerationMode,
  setSpecGenerationMode,
} from "../../api/groupChat";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import { lifecycleFetch } from "./IssueLifecycleWorkbench.test-utils";
import { GroupChatWorkbenchCard } from "../chat-room/GroupChatWorkbenchCard";
import { SpecGenerationSettings } from "../settings/SpecGenerationSettings";

vi.mock("../../api/groupChat", async () => {
  const actual = await vi.importActual<typeof import("../../api/groupChat")>(
    "../../api/groupChat",
  );
  return {
    ...actual,
    getSpecGenerationMode: vi.fn(),
    setSpecGenerationMode: vi.fn(),
  };
});

describe("IssueLifecycleWorkbench 群聊模式入口", () => {
  it("两种模式的 Issue 内容区域互斥渲染", async () => {
    const pipelineFetch = lifecycleFetch();
    vi.stubGlobal("fetch", pipelineFetch);
    const pipeline = render(
      <IssueLifecycleWorkbench specGenerationMode="pipeline" />,
    );

    expect(
      await screen.findByRole("region", { name: "Issue 生命周期详情" }),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("group-chat-workbench-card")).not.toBeInTheDocument();

    pipeline.unmount();
    const groupChatFetch = lifecycleFetch();
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        if (String(input) === "/api/group-chat/sessions") {
          return new Response(
            JSON.stringify({
              id: "group-chat-mode-test",
              project_id: "project_0001",
              issue_id: "issue_0001",
              status: "active",
              roles: [],
              artifact_lines: [],
              created_at: "2026-08-18T00:00:00Z",
              updated_at: "2026-08-18T00:00:00Z",
              timeline: [],
            }),
            { status: 200 },
          );
        }
        return groupChatFetch(input, init);
      }),
    );
    render(<IssueLifecycleWorkbench specGenerationMode="group_chat" />);

    expect(
      await screen.findByTestId("group-chat-workbench-card"),
    ).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Issue 生命周期详情" })).not.toBeInTheDocument();
  });

  it("群聊模式入口使用幂等创建得到的真实群聊会话 ID", async () => {
    const onOpenSession = vi.fn();
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          id: "group-chat-1",
          project_id: "project-1",
          issue_id: "issue-1",
          status: "active",
          roles: [],
          artifact_lines: [],
          created_at: "2026-08-18T00:00:00Z",
          updated_at: "2026-08-18T00:00:00Z",
          timeline: [],
        }),
        { status: 200 },
      ),
    );
    vi.stubGlobal("fetch", fetchMock);

    render(
      <GroupChatWorkbenchCard
        projectId="project-1"
        issueId="issue-1"
        issueTitle="登录会话过期"
        workspaceSessions={[
          {
            workspace_session_id: "bridge-session-1",
            issue_id: "issue-1",
            entity_id: "story-1",
            workspace_type: "story",
            status: "confirmed",
            author_provider: "codex",
            reviewer_provider: "claude_code",
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            origin: "group_chat",
          },
        ]}
        onOpenSession={onOpenSession}
      />,
    );

    await waitFor(() =>
      expect(screen.getByTestId("group-chat-workbench-card")).toHaveTextContent(
        "进行中",
      ),
    );
    await userEvent.click(screen.getByRole("button", { name: "进入群聊" }));
    expect(onOpenSession).toHaveBeenCalledWith("group-chat-1");
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/group-chat/sessions",
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("开始群聊会幂等创建会话并进入群聊页", async () => {
    const onOpenSession = vi.fn();
    vi.mocked(getSpecGenerationMode).mockResolvedValue("pipeline");
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          id: "group-chat-1",
          project_id: "project-1",
          issue_id: "issue-1",
          status: "active",
          roles: [],
          artifact_lines: [],
          created_at: "2026-08-18T00:00:00Z",
          updated_at: "2026-08-18T00:00:00Z",
          timeline: [],
        }),
        { status: 200 },
      ),
    );
    vi.stubGlobal("fetch", fetchMock);

    render(
      <GroupChatWorkbenchCard
        projectId="project-1"
        issueId="issue-1"
        issueTitle="登录会话过期"
        workspaceSessions={[]}
        onOpenSession={onOpenSession}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "开始群聊" }));
    await waitFor(() => expect(onOpenSession).toHaveBeenCalledWith("group-chat-1"));
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/group-chat/sessions",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({ project_id: "project-1", issue_id: "issue-1" }),
      }),
    );
  });

  it("设置开关写入 Spec 生成模式（受控 + 乐观更新）", async () => {
    vi.mocked(setSpecGenerationMode).mockResolvedValue("group_chat");
    const user = userEvent.setup();
    const onModeChange = vi.fn();

    render(
      <SpecGenerationSettings mode="pipeline" onModeChange={onModeChange} />,
    );

    expect(screen.getByRole("radio", { name: "流水线模式" })).toBeChecked();
    await user.click(screen.getByRole("radio", { name: "群聊模式" }));

    // 乐观更新：点击后立即回调，不等保存请求。
    expect(onModeChange).toHaveBeenCalledWith("group_chat");
    await waitFor(() =>
      expect(setSpecGenerationMode).toHaveBeenCalledWith("group_chat"),
    );
  });

  it("保存期间只禁用待切换选项，不让设置面板整体闪动", async () => {
    let resolveSave: (mode: "group_chat") => void = () => undefined;
    vi.mocked(setSpecGenerationMode).mockReturnValue(
      new Promise((resolve) => {
        resolveSave = resolve;
      }),
    );
    const user = userEvent.setup();
    const onModeChange = vi.fn();

    function ControlledSettings() {
      const [mode, setMode] = useState<"pipeline" | "group_chat">("pipeline");
      return (
        <SpecGenerationSettings
          mode={mode}
          onModeChange={(nextMode) => {
            onModeChange(nextMode);
            setMode(nextMode);
          }}
        />
      );
    }

    render(<ControlledSettings />);
    await user.click(screen.getByRole("radio", { name: "群聊模式" }));

    expect(screen.getByRole("radio", { name: "群聊模式" })).not.toBeDisabled();
    expect(screen.getByRole("radio", { name: "流水线模式" })).toBeDisabled();
    // 乐观更新下不展示「正在保存」提示，避免视觉闪动。
    expect(screen.queryByText("正在保存…")).not.toBeInTheDocument();

    resolveSave("group_chat");
    await waitFor(() => expect(screen.queryByText("正在保存…")).not.toBeInTheDocument());
  });
});
