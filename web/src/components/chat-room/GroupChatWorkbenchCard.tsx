import { MessageCircle, Play, RotateCw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import {
  createGroupChatSession,
  type GroupChatSessionResponse,
} from "../../api/groupChat";
import type {
  ArtifactVersion,
  WorkspaceSessionSummary,
} from "../../api/types";

export function GroupChatWorkbenchCard({
  projectId,
  issueId,
  issueTitle,
  workspaceSessions,
  artifactVersions = [],
  onOpenSession,
}: {
  projectId: string;
  issueId: string;
  issueTitle: string;
  workspaceSessions: WorkspaceSessionSummary[];
  artifactVersions?: ArtifactVersion[];
  onOpenSession: (sessionId: string) => void;
}) {
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [groupChatSession, setGroupChatSession] =
    useState<GroupChatSessionResponse | null>(null);
  const groupChatSessions = useMemo(
    () =>
      workspaceSessions.filter(
        (session) => session.origin === "group_chat",
      ),
    [workspaceSessions],
  );
  const latestSession = groupChatSessions[0] ?? null;
  const latestVersion = artifactVersions.reduce<number | null>(
    (latest, artifact) =>
      latest === null || artifact.version > latest ? artifact.version : latest,
    null,
  );

  useEffect(() => {
    let cancelled = false;
    setBusy(true);
    setError(null);
    createGroupChatSession({ project_id: projectId, issue_id: issueId })
      .then((session) => {
        if (!cancelled) {
          setGroupChatSession(session);
        }
      })
      .catch((reason: unknown) => {
        if (!cancelled) {
          setError(reason instanceof Error ? reason.message : "创建群聊会话失败");
        }
      })
      .finally(() => {
        if (!cancelled) {
          setBusy(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [issueId, projectId]);

  async function handleOpen() {
    setError(null);
    if (groupChatSession) {
      onOpenSession(groupChatSession.id);
      return;
    }

    setBusy(true);
    try {
      const session = await createGroupChatSession({
        project_id: projectId,
        issue_id: issueId,
      });
      setGroupChatSession(session);
      onOpenSession(session.id);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "创建群聊会话失败");
    } finally {
      setBusy(false);
    }
  }

  const finalizedVersions = groupChatSession?.artifact_lines.flatMap(
    (line) => line.finalized_versions,
  ) ?? [];
  const displayedVersion =
    finalizedVersions.length > 0
      ? finalizedVersions[finalizedVersions.length - 1]
      : latestVersion === null
        ? null
        : `v${latestVersion}`;

  return (
    <section
      data-testid="group-chat-workbench-card"
      aria-label="群聊工作台"
      className="rounded-md border border-[var(--aria-primary)]/40 bg-[var(--aria-panel)] p-4"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2 text-sm font-semibold text-[var(--aria-ink)]">
            <MessageCircle aria-hidden="true" className="h-4 w-4 text-[var(--aria-primary)]" />
            群聊式 Spec 生成
          </div>
          <h3 className="mt-1 truncate text-base font-semibold text-[var(--aria-ink)]">
            {issueTitle}
          </h3>
        </div>
        <span className="rounded border border-[var(--aria-primary)]/40 bg-[var(--aria-primary)]/10 px-2 py-0.5 text-xs font-semibold text-[var(--aria-primary)]">
          群聊模式
        </span>
      </div>
      <dl className="mt-4 grid gap-2 text-xs text-[var(--aria-ink-muted)] sm:grid-cols-2">
        <div className="rounded border border-[var(--aria-line)] px-3 py-2">
          <dt>会话状态</dt>
          <dd className="mt-1 font-semibold text-[var(--aria-ink)]">
            {groupChatSession
              ? groupChatStatusLabel(groupChatSession.status)
              : latestSession
                ? sessionStatusLabel(latestSession.status)
                : busy
                  ? "正在准备"
                  : "尚未开始"}
          </dd>
        </div>
        <div className="rounded border border-[var(--aria-line)] px-3 py-2">
          <dt>已定稿产物</dt>
          <dd className="mt-1 font-semibold text-[var(--aria-ink)]">
            {displayedVersion === null ? "暂无" : displayedVersion}
          </dd>
        </div>
      </dl>
      {error ? (
        <p role="alert" className="mt-3 text-xs text-[var(--aria-danger)]">
          {error}
        </p>
      ) : null}
      <button
        type="button"
        disabled={busy}
        onClick={() => void handleOpen()}
        className="mt-4 inline-flex h-9 items-center gap-2 rounded-md bg-[var(--aria-primary)] px-3 text-sm font-semibold text-white disabled:cursor-wait disabled:opacity-60"
      >
        {busy ? (
          <RotateCw aria-hidden="true" className="h-4 w-4 animate-spin" />
        ) : (
          <Play aria-hidden="true" className="h-4 w-4" />
        )}
        {groupChatSession ? "进入群聊" : "开始群聊"}
      </button>
    </section>
  );
}

function groupChatStatusLabel(status: GroupChatSessionResponse["status"]): string {
  switch (status) {
    case "active":
      return "进行中";
    case "finalized":
      return "已定稿";
    case "archived":
      return "已归档";
  }
}

function sessionStatusLabel(status: WorkspaceSessionSummary["status"]): string {
  switch (status) {
    case "confirmed":
      return "已定稿";
    case "open":
      return "待处理";
    case "running":
      return "生成中";
    case "waiting_for_human":
      return "等待确认";
    case "change_requested":
      return "待修改";
    case "blocked_provider_unavailable":
      return "Provider 不可用";
    case "terminated":
      return "已终止";
  }
}
