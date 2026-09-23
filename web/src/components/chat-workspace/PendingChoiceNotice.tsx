import { CircleAlert } from "lucide-react";
import type { ChatEntry } from "../../state/chat-entries";

/**
 * F-43 ②：未处理 choice 到达时的可见提示。coding attempt 的 choice 不进
 * workspace observer（controller 实测 attach 无选择帧），CockpitShell 的
 * 收件箱 toast 因此对它无效；这里在对话流内联常驻横幅，并给「定位选择卡」
 * 一键回到卡所在行（虚拟列表按行索引定位，长卡也不会停在卡尾）。
 */
export function PendingChoiceNotice({
  entries,
  onJump,
}: {
  entries: readonly ChatEntry[];
  onJump: (entryId: string) => void;
}) {
  const pending = pendingChoiceEntries(entries);
  const newest = pending.at(-1);
  if (!newest) {
    return null;
  }
  const prompt = typeof newest.metadata?.prompt === "string" ? newest.metadata.prompt : newest.content;
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
        onClick={() => onJump(newest.id)}
        className="inline-flex h-7 shrink-0 items-center rounded-md border border-amber-300 bg-white px-2 text-xs font-semibold text-amber-900 hover:bg-amber-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
      >
        定位选择卡
      </button>
    </div>
  );
}

/** 未处理（未应答/未失效）的选择请求，按对话流顺序返回。 */
export function pendingChoiceEntries(entries: readonly ChatEntry[]): ChatEntry[] {
  return entries.filter(
    (entry) => entry.type === "choice_request" && entry.resolved !== true,
  );
}
