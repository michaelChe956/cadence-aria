import { useEffect, useState } from "react";
import { CircleAlert } from "lucide-react";
import type { ChatEntry } from "../../state/chat-entries";
import type { PendingChoiceRequestProjection } from "../../state/workspace-ws-store-types";

/**
 * F-59：等待窗口展示口径——后端 PROVIDER_CHOICE_WAIT_TIMEOUT=900s 起算于
 * pending 集首次非空；+1s 展示余量避免倒计时先于引擎中止归零。
 */
const CHOICE_WAIT_HINT_WINDOW_MS = 901_000;

/** F-43 ②：未处理 choice 到达时的可见提示。coding attempt 的 choice 不进
 * workspace observer（controller 实测 attach 无选择帧），CockpitShell 的
 * 收件箱 toast 因此对它无效；这里在对话流内联常驻横幅，并给「定位选择卡」
 * 一键回到卡所在行（虚拟列表按行索引定位，长卡也不会停在卡尾）。
 *
 * F-59（缺陷 2）：新增 `requests`（session_state pending_choice_requests
 * 归一投影）驱动的等待提示条——修订/生成运行期间有 pending choice 即常驻，
 * 不依赖 choice 卡已渲染（帧丢失/未送达现场卡可能缺席）；含发问角色、
 * 已等待时长与 901s 超时倒计时（锚点 created_at_ms，旧载荷回退首见时刻）。
 */
export function PendingChoiceNotice({
  entries,
  requests = [],
  onJump,
}: {
  entries: readonly ChatEntry[];
  requests?: readonly PendingChoiceRequestProjection[];
  onJump: (entryId: string) => void;
}) {
  const pending = pendingChoiceEntries(entries);
  const newestRequest = requests.at(-1) ?? null;
  const visible = pending.length > 0 || requests.length > 0;
  const [, setTick] = useState(0);

  // F-59：等待时长/倒计时每秒刷新；无挂起时不留定时器。
  useEffect(() => {
    if (!visible) {
      return;
    }
    const timer = window.setInterval(() => setTick((value) => value + 1), 1_000);
    return () => window.clearInterval(timer);
  }, [visible]);

  if (!visible) {
    return null;
  }

  if (newestRequest) {
    return (
      <WaitHintBar
        requests={requests}
        newestRequest={newestRequest}
        cardEntryId={pending.at(-1)?.id ?? null}
        onJump={onJump}
      />
    );
  }

  const newest = pending.at(-1);
  if (!newest) {
    return null;
  }
  const metadataPrompt = newest.metadata?.prompt;
  const prompt = typeof metadataPrompt === "string" ? metadataPrompt : newest.content;
  const snippet = prompt.split("\n").find((line) => line.trim().length > 0)?.trim() ?? "";
  return (
    <div
      role="status"
      data-testid="pending-choice-notice"
      className="flex min-h-9 shrink-0 items-center gap-2 border-b border-amber-200 bg-amber-50 px-3 py-1 text-xs text-amber-900"
    >
      <CircleAlert aria-hidden="true" className="h-4 w-4 shrink-0" />
      <span className="min-w-0 flex-1 truncate font-semibold">
        有 {pending.length} 个选择请求待处理：{snippet}
      </span>
      <button
        type="button"
        onClick={() => onJump(pending[pending.length - 1].id)}
        className="inline-flex h-7 shrink-0 items-center rounded-md border border-amber-300 bg-white px-2 text-xs font-semibold text-amber-900 hover:bg-amber-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
      >
        定位选择卡
      </button>
    </div>
  );
}

function WaitHintBar({
  requests,
  newestRequest,
  cardEntryId,
  onJump,
}: {
  requests: readonly PendingChoiceRequestProjection[];
  newestRequest: PendingChoiceRequestProjection;
  cardEntryId: string | null;
  onJump: (entryId: string) => void;
}) {
  const windowStartMs = requests.reduce<number | null>((earliest, request) => {
    const startedAt = request.created_at_ms ?? request.first_seen_at_ms;
    if (startedAt === null) {
      return earliest;
    }
    return earliest === null ? startedAt : Math.min(earliest, startedAt);
  }, null);
  const elapsedMs = windowStartMs === null ? null : Date.now() - windowStartMs;
  const remainingMs =
    elapsedMs === null ? null : Math.max(0, CHOICE_WAIT_HINT_WINDOW_MS - elapsedMs);
  const roleLabel = newestRequest.role === "reviewer" ? "reviewer" : "author";
  const countLabel = requests.length > 1 ? `（${requests.length} 个问题）` : "";
  const snippet =
    newestRequest.prompt.split("\n").find((line) => line.trim().length > 0)?.trim() ?? "";

  return (
    <div
      role="status"
      data-testid="pending-choice-notice"
      className="flex min-h-9 shrink-0 items-center gap-2 border-b border-amber-200 bg-amber-50 px-3 py-1 text-xs text-amber-900"
    >
      <CircleAlert aria-hidden="true" className="h-4 w-4 shrink-0" />
      <span className="min-w-0 flex-1 truncate">
        <span className="font-semibold">
          ⏳ {roleLabel} 有问题等你回答{countLabel}：{snippet}
        </span>
        {elapsedMs !== null && remainingMs !== null ? (
          <span className="ml-2 whitespace-nowrap font-mono text-[11px] text-amber-700">
            已等待 {formatClock(elapsedMs)} · {formatClock(remainingMs)} 后超时
          </span>
        ) : null}
      </span>
      {cardEntryId ? (
        <button
          type="button"
          onClick={() => onJump(cardEntryId)}
          className="inline-flex h-7 shrink-0 items-center rounded-md border border-amber-300 bg-white px-2 text-xs font-semibold text-amber-900 hover:bg-amber-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
        >
          定位选择卡
        </button>
      ) : (
        <span className="shrink-0 text-[11px] text-amber-700">选择卡未显示？刷新页面可补卡</span>
      )}
    </div>
  );
}

/** 毫秒 → m:ss（等待提示条的已等待/剩余展示口径）。 */
function formatClock(durationMs: number): string {
  const totalSeconds = Math.floor(durationMs / 1_000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

/** 未处理（未应答/未失效）的选择请求，按对话流顺序返回。 */
export function pendingChoiceEntries(entries: readonly ChatEntry[]): ChatEntry[] {
  return entries.filter(
    (entry) => entry.type === "choice_request" && entry.resolved !== true,
  );
}
