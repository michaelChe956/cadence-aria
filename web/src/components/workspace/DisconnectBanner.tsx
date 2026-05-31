import { Check, ListTree, RefreshCw, TriangleAlert } from "lucide-react";

const ACK_STORAGE_KEY = "aria.workspace.aborted_ack_nodes";

interface DisconnectBannerProps {
  isReconnecting?: boolean;
  attemptCount?: number;
  onManualReconnect?: () => void;
  abortedByDisconnect?: { nodeId?: string; ts: string } | null;
  onAcknowledge?: (acknowledgedNodeIds: string[]) => void;
  onViewTimeline?: () => void;
}

export function DisconnectBanner({
  isReconnecting,
  attemptCount = 0,
  onManualReconnect,
  abortedByDisconnect,
  onAcknowledge,
  onViewTimeline,
}: DisconnectBannerProps) {
  if (isReconnecting && attemptCount > 0) {
    const displayAttemptCount = Math.max(attemptCount, 2);
    return (
      <div className="flex min-h-10 flex-wrap items-center justify-between gap-2 border-b border-amber-200 bg-amber-50 px-4 py-2 text-sm text-amber-800">
        <span className="inline-flex min-w-0 items-center gap-2">
          <RefreshCw className="h-4 w-4 shrink-0" />
          <span>连接断开，重连中（尝试 {displayAttemptCount} 次）</span>
        </span>
        {onManualReconnect ? (
          <button
            type="button"
            onClick={onManualReconnect}
            className="inline-flex h-8 items-center justify-center gap-2 rounded-md border border-amber-200 bg-white px-3 text-xs font-semibold text-amber-800 hover:bg-amber-100"
          >
            <RefreshCw className="h-3.5 w-3.5" />
            手动重连
          </button>
        ) : null}
      </div>
    );
  }

  if (abortedByDisconnect) {
    return (
      <div className="flex min-h-10 flex-wrap items-center justify-between gap-2 border-b border-red-200 bg-red-50 px-4 py-2 text-sm text-red-700">
        <span className="inline-flex min-w-0 items-center gap-2">
          <TriangleAlert className="h-4 w-4 shrink-0" />
          <span>
            上次运行因断开被中止（{new Date(abortedByDisconnect.ts).toLocaleTimeString()}）
          </span>
        </span>
        <span className="inline-flex items-center gap-2">
          {onViewTimeline ? (
            <button
              type="button"
              onClick={onViewTimeline}
              className="inline-flex h-8 items-center justify-center gap-2 rounded-md border border-red-200 bg-white px-3 text-xs font-semibold text-red-700 hover:bg-red-100"
            >
              <ListTree className="h-3.5 w-3.5" />
              查看 Timeline
            </button>
          ) : null}
          {onAcknowledge ? (
            <button
              type="button"
              onClick={() => {
                const acknowledged = abortedByDisconnect.nodeId
                  ? saveAcknowledgedAbortedNode(abortedByDisconnect.nodeId)
                  : loadAcknowledgedAbortedNodes();
                onAcknowledge(acknowledged);
              }}
              className="inline-flex h-8 items-center justify-center gap-2 rounded-md border border-red-200 bg-white px-3 text-xs font-semibold text-red-700 hover:bg-red-100"
            >
              <Check className="h-3.5 w-3.5" />
              我知道了
            </button>
          ) : null}
        </span>
      </div>
    );
  }

  return null;
}

export function loadAcknowledgedAbortedNodes(): string[] {
  try {
    const raw = window.localStorage.getItem(ACK_STORAGE_KEY);
    return raw ? (JSON.parse(raw) as string[]) : [];
  } catch {
    return [];
  }
}

export function saveAcknowledgedAbortedNode(nodeId: string) {
  const next = Array.from(new Set([...loadAcknowledgedAbortedNodes(), nodeId]));
  window.localStorage.setItem(ACK_STORAGE_KEY, JSON.stringify(next));
  return next;
}
