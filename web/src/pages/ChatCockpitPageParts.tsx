import { useEffect, useState } from "react";
import { Radio } from "lucide-react";
import type { ChatEntry } from "../state/chat-entries";
import { STALE_DRIVER_LEASE_CODE } from "../state/workspace-cockpit-projection";
import { workspaceStageLabel } from "../state/workspace-stage-labels";

export function useNowTicker(intervalMs = 1000): number {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), intervalMs);
    return () => window.clearInterval(timer);
  }, [intervalMs]);

  return now;
}

/** 引擎侧 provider 正在产出的阶段；starting 残留遇这些阶段一律按「正在生成」呈现（F-04）。 */
const GENERATING_STAGES: Record<string, true> = {
  running: true,
  cross_review: true,
  revision: true,
};
/** timeline 节点终态：引擎已落盘的运行结论，优先于连接态 providerStatus（F-01/F-04）。 */
export const TERMINAL_NODE_STATUSES: Record<string, true> = {
  completed: true,
  failed: true,
  skipped: true,
};
export type TerminalNodeContext = {
  status: "completed" | "failed" | "skipped";
  elapsedMs: number | null;
};

export function runningProviderName(
  stage: string,
  providers: { author: string; reviewer?: string | null } | null,
): string | null {
  if (!providers) return null;
  if (GENERATING_STAGES[stage] !== true) return null;
  return stage === "cross_review" ? (providers.reviewer ?? null) : providers.author;
}

function formatElapsedMs(elapsedMs: number): string {
  const totalSeconds = Math.max(0, Math.floor(elapsedMs / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`
    : `${minutes}:${String(seconds).padStart(2, "0")}`;
}

/** 终态节点的固定时长：优先 duration_ms，退化用 completed_at−started_at；无法确定时省略。 */
export function terminalElapsedMs(node: {
  duration_ms?: number | null;
  started_at: string;
  completed_at?: string | null;
}): number | null {
  if (typeof node.duration_ms === "number") {
    return node.duration_ms;
  }
  if (node.completed_at == null) {
    return null;
  }
  const startedAtMs = Date.parse(node.started_at);
  const completedAtMs = Date.parse(node.completed_at);
  if (Number.isNaN(startedAtMs) || Number.isNaN(completedAtMs)) {
    return null;
  }
  return Math.max(0, completedAtMs - startedAtMs);
}

export function generationStatusText(
  providerStatus: string,
  stage: string,
  running: { provider: string | null; elapsedMs: number | null } | null,
  terminal: TerminalNodeContext | null,
) {
  if (providerStatus === "not_started") {
    return "等待发起 · 选择 Provider 后点击「开始生成」";
  }

  const stageLabel = workspaceStageLabel(stage);

  // F-01/F-04：session_state 快照会把 providerStatus 重置回 starting，而 timeline
  // 节点终态是引擎落盘的事实——终态优先呈现，「已用」冻结在节点结束时刻，
  // 不再随墙钟给死 run 计时。
  if (terminal !== null && (providerStatus === "starting" || providerStatus === "running")) {
    const label =
      terminal.status === "failed"
        ? "生成失败"
        : terminal.status === "completed"
          ? "生成完成"
          : "生成已跳过";
    return [
      label,
      stageLabel,
      terminal.elapsedMs === null ? null : `已用 ${formatElapsedMs(terminal.elapsedMs)}`,
    ]
      .filter((segment): segment is string => segment !== null)
      .join(" · ");
  }

  switch (providerStatus) {
    case "running":
    case "starting": {
      // F-04：引擎 stage 已进入生成类阶段时，starting 只是快照重置残留——统一为
      // 「正在生成」，避免与「运行中」同屏拼接；starting 遇到确认/终态等非生成
      // 阶段则不呈现残留前缀，直接给出阶段事实。
      if (
        providerStatus === "starting" &&
        GENERATING_STAGES[stage] !== true &&
        stage !== "prepare_context"
      ) {
        return stageLabel;
      }
      const statusLabel =
        providerStatus === "starting" && stage === "prepare_context"
          ? "正在启动生成"
          : "正在生成";
      const segments = [
        statusLabel,
        running?.provider,
        stageLabel,
        running?.elapsedMs === null || running?.elapsedMs === undefined
          ? null
          : `已用 ${formatElapsedMs(running.elapsedMs)}`,
      ];
      return segments.filter((segment): segment is string => segment !== null).join(" · ");
    }
    case "waiting_approval":
      return `等待生成确认 · ${stageLabel}`;
    case "completed":
      return `生成完成 · ${stageLabel}`;
    case "failed":
      return `生成失败 · ${stageLabel}`;
    case "aborted":
      return `生成已终止 · ${stageLabel}`;
    default:
      return stageLabel;
  }
}

export function StreamingConversationBlock({
  content,
  role,
}: {
  content: string;
  role: ChatEntry["role"];
}) {
  if (!content) {
    return null;
  }

  const roleLabel = role === "reviewer" ? "审核者" : "作者";
  return (
    <article
      data-testid="cockpit-streaming-content"
      data-frame-window-ms="50"
      aria-live="polite"
      aria-label={`${roleLabel}正在生成`}
      className="mx-3 mb-3 rounded-lg border border-[var(--aria-primary)] bg-[var(--aria-panel-muted)] p-3"
    >
      <div className="flex items-center gap-2 text-xs font-semibold text-[var(--aria-ink-muted)]">
        <Radio className="h-3.5 w-3.5 text-[var(--aria-primary)]" aria-hidden="true" />
        {roleLabel}正在生成
      </div>
      <pre className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap break-words text-xs leading-5 text-[var(--aria-ink)]">
        {content}
      </pre>
    </article>
  );
}

/** F-28 二轮：仲裁层拒收、可经 sendHello(role=driver) 夺回的两码——
 * STALE_DRIVER_LEASE（driver 丢租约）与 OBSERVER_WRITE_REJECTED（本连接以
 * observer 身份重连后写操作被拒）。 */
export const RETAKABLE_LEASE_CODES: Record<string, true> = {
  [STALE_DRIVER_LEASE_CODE]: true,
  OBSERVER_WRITE_REJECTED: true,
};
