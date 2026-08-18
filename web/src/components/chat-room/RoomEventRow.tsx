import { CheckCircle2, CirclePause, FileCheck2, MessageCircle, UserRound } from "lucide-react";
import type { ReactNode } from "react";
import type { RoleInstance, RoomEvent } from "../../api/groupChat";
import { MarkdownContent } from "../chat-workspace/entries/ProviderStreamEntry";

interface RoomEventRowProps {
  event: RoomEvent;
  roles: RoleInstance[];
}

const ARTIFACT_LINE_LABELS = {
  issue_refinement: "需求澄清",
  story_spec: "故事规格",
  design_spec: "设计规格",
} as const;

/** 仅服务群聊 RoomEvent 的行渲染，不依赖共享 ChatEntry 的角色枚举。 */
export function RoomEventRow({ event, roles }: RoomEventRowProps) {
  switch (event.type) {
    case "user_message":
      return (
        <div className="flex justify-end" data-testid="room-user-message">
          <article className="max-w-3xl rounded-md border border-gray-200 bg-gray-50 px-3 py-2 text-sm shadow-sm">
            <div className="text-xs font-semibold text-gray-600">用户</div>
            <div className="mt-2 whitespace-pre-wrap break-words text-[var(--aria-ink)]">
              {event.text}
            </div>
            {event.mentions.length > 0 ? (
              <div className="mt-2 flex flex-wrap gap-1">
                {event.mentions.map((roleId) => (
                  <span
                    key={roleId}
                    className="rounded bg-white px-1.5 py-0.5 text-xs text-[var(--aria-ink-muted)]"
                  >
                    @{roleName(roleId, roles)}
                  </span>
                ))}
              </div>
            ) : null}
          </article>
        </div>
      );
    case "agent_message":
      return <AgentMessageGroup events={[event]} roles={roles} />;
    case "held_event":
      return (
        <InlineRoomEvent icon={CirclePause} testId="room-held-event">
          {roleName(event.role_instance_id, roles)} 暂缓发言：{event.reason}
        </InlineRoomEvent>
      );
    case "claim_event":
      return (
        <InlineRoomEvent icon={CheckCircle2} testId="room-claim-event">
          {roleName(event.role_instance_id, roles)}
          {event.claimed ? " 认领" : " 释放"}
          {artifactLineLabel(event.line)} / {event.slot_key}
        </InlineRoomEvent>
      );
    case "finalize_event":
      return (
        <InlineRoomEvent icon={FileCheck2} testId="room-finalize-event">
          已定稿{artifactLineLabel(event.artifact_line)} · {event.version}
          {event.included_slots.length > 0
            ? `（包含：${event.included_slots.join("、")}）`
            : ""}
        </InlineRoomEvent>
      );
    case "system_notice":
      return (
        <InlineRoomEvent icon={MessageCircle} testId="room-system-notice">
          {event.text}
        </InlineRoomEvent>
      );
  }
}

type AgentMessageEvent = Extract<RoomEvent, { type: "agent_message" }>;

type AgentMessageContent = {
  content: string;
  artifact?: string | null;
};

/** 连续同角色消息共用头像与名牌，避免依赖共享 ChatEntry 的角色分组。 */
export function AgentMessageGroup({
  events,
  roles,
}: {
  events: AgentMessageEvent[];
  roles: RoleInstance[];
}) {
  const roleId = events[0]?.role_instance_id ?? "unknown";
  return (
    <AgentMessageCard
      testId={`room-agent-message-${roleId}`}
      role={roleFor(roleId, roles)}
      messages={events.map((event) => ({
        content: event.text,
        artifact: event.artifact_ref
          ? `${artifactLineLabel(event.artifact_ref.line)} / ${event.artifact_ref.slot} · v${event.artifact_ref.version}`
          : null,
      }))}
    />
  );
}

export function StreamingTurnRow({
  roleId,
  text,
  status,
  roles,
}: {
  roleId: string;
  text: string;
  status: "started" | "held";
  roles: RoleInstance[];
}) {
  return (
    <AgentMessageCard
      testId={`room-stream-${roleId}`}
      role={roleFor(roleId, roles)}
      messages={[{ content: text }]}
      pending={status === "started"}
    />
  );
}

function AgentMessageCard({
  role,
  messages,
  pending = false,
  testId,
}: {
  role: RoleInstance | null;
  messages: AgentMessageContent[];
  pending?: boolean;
  testId: string;
}) {
  return (
    <div className="flex justify-start" data-testid={testId}>
      <article className="w-full max-w-3xl rounded-md border border-blue-200 bg-blue-50 px-3 py-2 text-sm shadow-sm">
        <div className="flex min-w-0 items-center gap-2">
          <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-blue-100 text-blue-700">
            <UserRound aria-hidden="true" className="h-3.5 w-3.5" />
          </span>
          <span className="truncate text-xs font-semibold text-blue-700">
            {role?.display_name ?? "未知角色"}
          </span>
          {pending ? (
            <span className="text-xs text-[var(--aria-ink-muted)]">正在回复…</span>
          ) : null}
        </div>
        <div className="mt-2 space-y-3">
          {messages.map((message, index) => (
            <div
              key={`${index}:${message.content}`}
              className={index > 0 ? "border-t border-blue-100 pt-3" : ""}
            >
              {message.content ? (
                <MarkdownContent content={message.content} />
              ) : (
                <div className="text-sm text-[var(--aria-ink-muted)]">正在准备回复…</div>
              )}
              {message.artifact ? (
                <div className="mt-2 text-xs text-[var(--aria-ink-muted)]">
                  草稿：{message.artifact}
                </div>
              ) : null}
            </div>
          ))}
        </div>
      </article>
    </div>
  );
}

function InlineRoomEvent({
  icon: Icon,
  children,
  testId,
}: {
  icon: typeof CirclePause;
  children: ReactNode;
  testId: string;
}) {
  return (
    <div
      data-testid={testId}
      className="flex items-center justify-center gap-2 rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2 text-center text-xs text-[var(--aria-ink-muted)]"
    >
      <Icon aria-hidden="true" className="h-3.5 w-3.5 shrink-0 text-[var(--aria-primary)]" />
      <span>{children}</span>
    </div>
  );
}

function roleFor(roleId: string, roles: RoleInstance[]) {
  return roles.find((role) => role.id === roleId) ?? null;
}

function roleName(roleId: string, roles: RoleInstance[]) {
  return roleFor(roleId, roles)?.display_name ?? "未知角色";
}

function artifactLineLabel(line: keyof typeof ARTIFACT_LINE_LABELS) {
  return ARTIFACT_LINE_LABELS[line];
}
