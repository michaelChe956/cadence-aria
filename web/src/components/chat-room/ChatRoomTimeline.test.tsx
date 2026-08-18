import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { RoleInstance, TimelineEvent } from "../../api/groupChat";
import { MentionInput } from "./MentionInput";
import { ChatRoomTimeline } from "./ChatRoomTimeline";

const roles: RoleInstance[] = [
  {
    id: "role-author",
    role_key: "author",
    provider: "claude_code",
    display_name: "需求作者",
    permission_mode: "auto",
    seen_cursor: 0,
    injection_watermark: 0,
  },
  {
    id: "role-frontend",
    role_key: "frontend_design",
    provider: "codex",
    display_name: "前端设计师",
    permission_mode: "supervised",
    seen_cursor: 0,
    injection_watermark: 0,
  },
  {
    id: "role-backend",
    role_key: "backend_design",
    provider: "claude_code",
    display_name: "后端设计师",
    permission_mode: "auto",
    seen_cursor: 0,
    injection_watermark: 0,
  },
];

const timeline: TimelineEvent[] = [
  {
    seq: 1,
    event: {
      type: "user_message",
      text: "请讨论登录流程",
      mentions: ["role-frontend"],
    },
  },
  {
    seq: 2,
    event: {
      type: "agent_message",
      role_instance_id: "role-frontend",
      text: "建议先定义登录状态。",
      artifact_ref: { line: "design_spec", slot: "frontend", version: 2 },
      cursor_after: 2,
    },
  },
  {
    seq: 3,
    event: {
      type: "held_event",
      role_instance_id: "role-backend",
      reason: "等待前端接口约定",
      cursor_after: 3,
    },
  },
  {
    seq: 4,
    event: {
      type: "claim_event",
      role_instance_id: "role-frontend",
      line: "design_spec",
      slot_key: "frontend",
      claimed: true,
    },
  },
  {
    seq: 5,
    event: {
      type: "finalize_event",
      artifact_line: "design_spec",
      version: "design-v2",
      included_slots: ["frontend", "backend"],
    },
  },
  {
    seq: 6,
    event: { type: "system_notice", text: "本轮讨论已结束" },
  },
];

describe("ChatRoomTimeline", () => {
  it("渲染全部 RoomEvent 变体及角色实例名牌", () => {
    const { asFragment } = render(
      <ChatRoomTimeline timeline={timeline} roles={roles} />,
    );

    expect(screen.getByText("请讨论登录流程")).toBeInTheDocument();
    expect(screen.getByText("前端设计师")).toBeInTheDocument();
    expect(screen.getByText("建议先定义登录状态。")).toBeInTheDocument();
    expect(screen.getByText("后端设计师 暂缓发言：等待前端接口约定")).toBeInTheDocument();
    expect(screen.getByText("前端设计师 认领设计规格 / frontend")).toBeInTheDocument();
    expect(screen.getByTestId("room-finalize-event")).toHaveTextContent(
      "已定稿设计规格 · design-v2",
    );
    expect(screen.getByText("本轮讨论已结束")).toBeInTheDocument();
    expect(asFragment()).toMatchSnapshot();
  });

  it("随着 TurnDelta 累积追加流式角色消息，并在落盘全文一致后收敛", () => {
    const { rerender } = render(
      <ChatRoomTimeline
        timeline={timeline.slice(0, 1)}
        roles={roles}
        turns={{
          "role-frontend": { text: "正在", status: "started" },
        }}
      />,
    );

    expect(screen.getByTestId("room-stream-role-frontend")).toHaveTextContent("正在");

    rerender(
      <ChatRoomTimeline
        timeline={timeline.slice(0, 1)}
        roles={roles}
        turns={{
          "role-frontend": { text: "正在生成设计方案", status: "started" },
        }}
      />,
    );

    expect(screen.getByTestId("room-stream-role-frontend")).toHaveTextContent(
      "正在生成设计方案",
    );

    rerender(
      <ChatRoomTimeline
        timeline={[
          ...timeline.slice(0, 1),
          {
            seq: 2,
            event: {
              type: "agent_message",
              role_instance_id: "role-frontend",
              text: "正在生成设计方案",
              artifact_ref: null,
              cursor_after: 2,
            },
          },
        ]}
        roles={roles}
        turns={{
          "role-frontend": { text: "正在生成设计方案", status: "started" },
        }}
      />,
    );

    expect(screen.queryByTestId("room-stream-role-frontend")).not.toBeInTheDocument();
    expect(screen.getByTestId("room-agent-message-role-frontend")).toHaveTextContent(
      "正在生成设计方案",
    );
  });

  it("按 @ 后的关键字过滤角色并提交角色实例 ID", async () => {
    const onSubmit = vi.fn();
    const user = userEvent.setup();
    render(<MentionInput roles={roles} onSubmit={onSubmit} />);

    await user.type(screen.getByRole("textbox", { name: "群聊消息" }), "请 @前");

    expect(screen.getByRole("option", { name: "前端设计师" })).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "后端设计师" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("option", { name: "前端设计师" }));
    await user.click(screen.getByRole("button", { name: "发送" }));

    expect(onSubmit).toHaveBeenCalledWith("请 @前端设计师", ["role-frontend"]);
  });
});
