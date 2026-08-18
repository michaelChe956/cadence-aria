import { ArrowLeft, Wifi, WifiOff } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import {
  getGroupChatSession,
  type DraftSlotKey,
  type GroupChatSession,
  type GroupChatSessionResponse,
  type TimelineEvent,
} from "../api/groupChat";
import { ArtifactLinePanel } from "../components/chat-room/ArtifactLinePanel";
import { ChatRoomTimeline } from "../components/chat-room/ChatRoomTimeline";
import { MentionInput } from "../components/chat-room/MentionInput";
import { RoleBar } from "../components/chat-room/RoleBar";
import { useGroupChatWs } from "../hooks/useGroupChatWs";

export function ChatRoomPage({
  sessionId,
  onBack,
}: {
  sessionId: string;
  onBack: () => void;
}) {
  const [session, setSession] = useState<GroupChatSessionResponse | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const {
    connectionStatus,
    error: socketError,
    timeline: socketTimeline,
    turns,
    sendMessage,
  } = useGroupChatWs(sessionId);
  const [sendError, setSendError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setLoadError(null);
    setSession(null);

    getGroupChatSession(sessionId)
      .then((response) => {
        if (!cancelled) {
          setSession(response);
        }
      })
      .catch((cause: unknown) => {
        if (!cancelled) {
          setLoadError(errorMessage(cause, "加载群聊会话失败"));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [sessionId]);

  const timeline = useMemo(
    () => mergeTimeline(session?.timeline ?? [], socketTimeline),
    [session?.timeline, socketTimeline],
  );
  const error = loadError ?? socketError ?? sendError;
  const inputDisabled =
    loading ||
    !session ||
    session.status !== "active" ||
    connectionStatus !== "connected";

  useEffect(() => {
    if (socketTimeline.length === 0) {
      return;
    }
    let cancelled = false;
    getGroupChatSession(sessionId)
      .then((response) => {
        if (!cancelled) {
          setSession(response);
        }
      })
      .catch(() => {
        // 时间线已由 WebSocket 呈现；快照刷新失败时保留当前面板数据。
      });
    return () => {
      cancelled = true;
    };
  }, [sessionId, socketTimeline]);

  function handleSend(text: string, mentions: string[]) {
    setSendError(null);
    if (!sendMessage(text, mentions)) {
      setSendError("群聊连接尚未就绪，请稍后重试");
    }
  }

  function handleDraftSlot(slotKey: DraftSlotKey) {
    setSendError(null);
    if (!sendMessage(`请起草草稿槽：${slotKey}`, [], slotKey)) {
      setSendError("群聊连接尚未就绪，请稍后重试");
    }
  }

  function handleSessionUpdated(updated: GroupChatSession) {
    setSession((current) =>
      current ? { ...updated, timeline: current.timeline } : current,
    );
    getGroupChatSession(sessionId)
      .then(setSession)
      .catch(() => {
        // 定稿结果已经返回最新会话快照，补拉时间线失败不影响面板状态。
      });
  }

  return (
    <main className="flex h-screen min-w-0 flex-col overflow-hidden bg-[var(--aria-bg)] text-[var(--aria-ink)]">
      <header className="flex h-11 min-w-0 shrink-0 items-center justify-between gap-3 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-3">
        <button
          type="button"
          onClick={onBack}
          className="inline-flex h-8 shrink-0 items-center gap-2 rounded-md px-2 text-sm text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)]"
        >
          <ArrowLeft aria-hidden="true" className="h-4 w-4" />
          返回
        </button>
        <div className="min-w-0 flex-1 truncate text-center text-sm font-semibold">
          群聊式 Spec 生成 · {session?.issue_id ?? sessionId}
        </div>
        <div className="flex shrink-0 items-center gap-2 text-xs text-[var(--aria-ink-muted)]">
          {connectionStatus === "connected" ? (
            <Wifi aria-label="已连接" className="h-4 w-4 text-emerald-600" />
          ) : (
            <WifiOff aria-label="未连接" className="h-4 w-4 text-[var(--aria-ink-muted)]" />
          )}
          <span>{connectionLabel(connectionStatus)}</span>
        </div>
      </header>
      {error ? (
        <div role="alert" className="mx-3 mt-3 rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700">
          {error}
        </div>
      ) : null}
      {loading ? (
        <div className="flex min-h-0 flex-1 items-center justify-center text-sm text-[var(--aria-ink-muted)]">
          正在加载群聊会话…
        </div>
      ) : session ? (
        <>
          <RoleBar
            sessionId={session.id}
            roles={session.roles}
            disabled={session.status !== "active"}
            onSessionUpdated={handleSessionUpdated}
          />
          <div className="flex min-h-0 flex-1 flex-col lg:flex-row">
            <ChatRoomTimeline timeline={timeline} roles={session.roles} turns={turns} />
            <ArtifactLinePanel
              sessionId={session.id}
              artifactLines={session.artifact_lines}
              roles={session.roles}
              sessionActive={session.status === "active"}
              onDraftSlot={handleDraftSlot}
              onSessionUpdated={handleSessionUpdated}
            />
          </div>
          {session.status !== "active" ? (
            <div className="border-t border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2 text-center text-sm text-[var(--aria-ink-muted)]">
              当前会话已{session.status === "finalized" ? "定稿" : "归档"}，不可继续发送消息。
            </div>
          ) : null}
          <MentionInput
            roles={session.roles}
            disabled={inputDisabled}
            onSubmit={handleSend}
          />
        </>
      ) : (
        <div className="flex min-h-0 flex-1 items-center justify-center text-sm text-[var(--aria-ink-muted)]">
          无法加载群聊会话。
        </div>
      )}
    </main>
  );
}

function mergeTimeline(
  initialTimeline: TimelineEvent[],
  socketTimeline: TimelineEvent[],
): TimelineEvent[] {
  const bySeq = new Map<number, TimelineEvent>();
  for (const entry of initialTimeline) {
    bySeq.set(entry.seq, entry);
  }
  for (const entry of socketTimeline) {
    bySeq.set(entry.seq, entry);
  }
  return [...bySeq.values()].sort((left, right) => left.seq - right.seq);
}

function connectionLabel(status: ReturnType<typeof useGroupChatWs>["connectionStatus"]) {
  switch (status) {
    case "connected":
      return "已连接";
    case "connecting":
      return "连接中";
    case "error":
      return "连接异常";
    case "disconnected":
      return "已断开";
  }
}

function errorMessage(cause: unknown, fallback: string) {
  return cause instanceof Error && cause.message ? cause.message : fallback;
}
