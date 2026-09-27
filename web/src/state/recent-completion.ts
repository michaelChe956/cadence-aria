import type { RecentCompletionInfoItem } from "../api/types";
import {
  codingFinalConfirmInfoItem,
  planConfirmedInfoItem,
  type CockpitInboxItem,
} from "./workspace-cockpit-projection";

// P3 WIGA（tasks.md §4.1 / REQ-WIGA-07）：近期完成事实的纯前端稳定身份与
// TTL 判据。事实 key 只取服务端 durable 目录条目的完整 scope（
// project/issue/kind/key），不取 item.id 或 WS seq——跨 project/issue 的
// 重名 key 不得互相覆盖或互斥。

/** 服务端近期窗口：完成事实最多回看 24 小时（与后端
 * `RECENT_COMPLETION_WINDOW_HOURS` 同源取值）。 */
export const RECENT_COMPLETION_WINDOW_MS = 24 * 60 * 60 * 1000;
/** info 展示/提醒 TTL：与近期窗口一致（到期不展示、不提醒、不续期）。 */
export const INFO_TTL_MS = RECENT_COMPLETION_WINDOW_MS;

/** 稳定身份：JSON.stringify([project_id, issue_id, kind, key])——数组序列化
 * 避免跨 project/issue/key 分隔符冲突。 */
export function recentCompletionIdentity(item: RecentCompletionInfoItem): string {
  return JSON.stringify([item.project_id, item.issue_id, item.kind, item.key]);
}

/** 近期完成事实 → 只读 info 收件箱条目（复用既有 plan/coding 投影文案与
 * 下钻，附加 completionIdentity 供提示去重）。 */
export function recentCompletionInboxItem(item: RecentCompletionInfoItem): CockpitInboxItem {
  return item.kind === "plan_confirmed"
    ? {
        ...planConfirmedInfoItem({
          key: item.key,
          plan_id: item.plan_id,
          session_id: item.session_id,
          occurred_at: item.occurred_at,
          title: item.title,
        }),
        completionIdentity: recentCompletionIdentity(item),
      }
    : {
        ...codingFinalConfirmInfoItem({
          key: item.key,
          project_id: item.project_id,
          issue_id: item.issue_id,
          plan_id: item.plan_id,
          attempt_id: item.attempt_id,
          occurred_at: item.occurred_at,
          title: item.title,
          final_confirmed: item.final_confirmed,
        }),
        completionIdentity: recentCompletionIdentity(item),
      };
}

/** TTL 可见性：0 <= nowMs - Date.parse(occurredAt) < INFO_TTL_MS；未来
 * 时刻与坏时间 fail-closed 不展示（Date.parse 有限性先验证）。 */
export function isRecentCompletionVisible(occurredAt: string, nowMs: number): boolean {
  const occurredMs = Date.parse(occurredAt);
  if (!Number.isFinite(occurredMs)) {
    return false;
  }
  const ageMs = nowMs - occurredMs;
  return ageMs >= 0 && ageMs < INFO_TTL_MS;
}

