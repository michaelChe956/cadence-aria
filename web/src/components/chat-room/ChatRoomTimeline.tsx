import { useEffect, useMemo, useRef } from "react";
import type { RoleInstance, RoomEvent, TimelineEvent } from "../../api/groupChat";
import type { GroupChatTurnState } from "../../hooks/useGroupChatWs";
import {
  AgentMessageGroup,
  RoomEventRow,
  StreamingTurnRow,
} from "./RoomEventRow";

interface ChatRoomTimelineProps {
  timeline: TimelineEvent[];
  roles: RoleInstance[];
  turns?: Record<string, GroupChatTurnState>;
}

type TimelineRow =
  | { kind: "event"; seq: number; event: RoomEvent }
  | {
      kind: "agent_group";
      firstSeq: number;
      events: Extract<RoomEvent, { type: "agent_message" }>[];
    };

/** 群聊独立时间线，按角色实例展示事件与尚未落盘的流式回复。 */
export function ChatRoomTimeline({
  timeline,
  roles,
  turns = {},
}: ChatRoomTimelineProps) {
  const endRef = useRef<HTMLDivElement | null>(null);
  const events = useMemo(
    () => [...timeline].sort((left, right) => left.seq - right.seq),
    [timeline],
  );
  const rows = useMemo(() => groupTimelineEvents(events), [events]);
  const streamedTurns = useMemo(
    () =>
      Object.entries(turns).filter(
        ([roleId, turn]) => !hasPersistedTurnResult(roleId, turn, events),
      ),
    [events, turns],
  );

  useEffect(() => {
    if (typeof endRef.current?.scrollIntoView === "function") {
      endRef.current.scrollIntoView({ behavior: "auto", block: "end" });
    }
  }, [rows, streamedTurns]);

  return (
    <div
      data-testid="chat-room-timeline"
      className="min-h-0 flex-1 overflow-auto px-3 py-4"
    >
      {events.length === 0 && streamedTurns.length === 0 ? (
        <div className="flex min-h-full items-center justify-center text-sm text-[var(--aria-ink-muted)]">
          暂无群聊记录
        </div>
      ) : (
        <div className="space-y-3">
          {rows.map((row) =>
            row.kind === "agent_group" ? (
              <AgentMessageGroup
                key={`agent-group:${row.firstSeq}`}
                events={row.events}
                roles={roles}
              />
            ) : (
              <RoomEventRow key={row.seq} event={row.event} roles={roles} />
            ),
          )}
          {streamedTurns.map(([roleId, turn]) => (
            <StreamingTurnRow
              key={roleId}
              roleId={roleId}
              text={turn.text}
              status={turn.status}
              roles={roles}
            />
          ))}
          <div ref={endRef} />
        </div>
      )}
    </div>
  );
}

function groupTimelineEvents(events: TimelineEvent[]): TimelineRow[] {
  const rows: TimelineRow[] = [];
  let currentAgentGroup: Extract<TimelineRow, { kind: "agent_group" }> | null = null;

  for (const entry of events) {
    if (entry.event.type !== "agent_message") {
      currentAgentGroup = null;
      rows.push({ kind: "event", seq: entry.seq, event: entry.event });
      continue;
    }
    const previous = currentAgentGroup?.events.at(-1);
    if (
      !currentAgentGroup ||
      previous?.role_instance_id !== entry.event.role_instance_id
    ) {
      currentAgentGroup = {
        kind: "agent_group",
        firstSeq: entry.seq,
        events: [entry.event],
      };
      rows.push(currentAgentGroup);
    } else {
      currentAgentGroup.events.push(entry.event);
    }
  }
  return rows;
}

function hasPersistedTurnResult(
  roleId: string,
  turn: GroupChatTurnState,
  events: TimelineEvent[],
) {
  const terminalEvent = [...events]
    .reverse()
    .find(
      ({ event }) =>
        (event.type === "agent_message" || event.type === "held_event") &&
        event.role_instance_id === roleId,
    )?.event;
  if (!terminalEvent) {
    return false;
  }
  if (terminalEvent.type === "held_event") {
    return turn.status === "held";
  }
  // 当前服务端可能只发送 TurnStarted 后直接写入 RoomEvent；空增量不应留下空占位行。
  if (turn.status === "started" && turn.text.length === 0) {
    return true;
  }
  return terminalEvent.type === "agent_message" && terminalEvent.text === turn.text;
}
