// 批 2 T4 后续（owner 2026-10-25 真机死路根修）：统一终止确认遇 409
// `automation_enrollment_conflict` 的自愈，以及会话失效的最小提示。
//
// 现场：owner 反复重开自动化 → enrollment `policy_revision` 轮换（正常现象），
// 页面持有的旧会话/投影提交了过期 revision → 终止端点回 409，前端旧实现只把
// 错误显示成一句话，用户没有任何继续路径。这里把 409 变成「刷新确认面身份 +
// 让用户再确认一次」（不自动重试终止动作、不静默失败）。
//
// 从 CockpitInboxWaitingCards.tsx 拆出：该文件已接近行数上限，新逻辑独立成文件。
import { getAutomationEnrollment } from "../../../api/client";
import type { IssueAutomationEnrollment } from "../../../api/types";
import { useCodingWorkspaceStore } from "../../../state/coding-workspace-store";
import type { AutomationTerminationRequest } from "../../../state/coding-workspace-store";
import { BTN_SECONDARY_CLASS } from "../gate-visual-tokens";

/** 服务端 enrollment 身份冲突（409；details 携带当前身份）。 */
export const AUTOMATION_ENROLLMENT_CONFLICT = "automation_enrollment_conflict";

export interface TerminationConflictResolution {
  /** details 未带 enrollment_id 时沿用页面已有身份；仍缺失为 null（交给提交前 GET 补读）。 */
  enrollmentId: string | null;
  /** 服务端当前 policy_revision。 */
  policyRevision: number;
  /** 给用户的可操作提示（要求重新确认，不是终止失败结论）。 */
  notice: string;
}

/** 409 details 里本前端消费的字段（REST 错误信封已归一化为对象；其余字段忽略）。 */
type EnrollmentConflictDetails = {
  current_revision?: unknown;
  enrollment_id?: unknown;
};

/**
 * 解析 409 `automation_enrollment_conflict` 的 details。契约是 additive：
 * `current_revision` 为既有字段（服务端一直带），`enrollment_id`/`plan_id`/
 * `session_id` 为本次新增（可得时带上，旧客户端忽略即可）。
 *
 * 缺 `current_revision` → null：调用方退回只读 GET enrollment 刷新身份，不猜。
 */
export function resolveTerminationRevisionConflict(
  details: unknown,
  fallbackEnrollmentId: string | null,
): TerminationConflictResolution | null {
  if (typeof details !== "object" || details === null) {
    return null;
  }
  // 边界处的运行时断言：details 只从这里读取，字段逐个 typeof 校验。
  const source = details as EnrollmentConflictDetails;
  const revision = source.current_revision;
  if (typeof revision !== "number" || !Number.isInteger(revision) || revision < 0) {
    return null;
  }
  const enrollmentId =
    typeof source.enrollment_id === "string" && source.enrollment_id !== ""
      ? source.enrollment_id
      : fallbackEnrollmentId;
  return {
    enrollmentId,
    policyRevision: revision,
    notice:
      enrollmentId === null
        ? `自动化 enrollment 已更新到 rev ${revision}，请重新读取 enrollment 身份后再确认终止`
        : `自动化 enrollment 已更新到 rev ${revision}，请重新确认终止`,
  };
}

/**
 * 409 自愈：用服务端当前身份刷新确认面，让用户能再点一次确认。
 * 不自动提交终止（用户必须显式再确认），也不把错误吞成成功。
 *
 * details 缺 `current_revision`（旧服务端）时退回只读 GET enrollment 刷新
 * 身份；GET 也读不到才返回 null，由调用方给出「刷新页面后重试」的可操作报错。
 */
export async function refreshTerminationIdentityOnConflict(args: {
  details: unknown;
  projectId: string;
  issueId: string;
  fallbackEnrollmentId: string | null;
  openConfirm: (request: AutomationTerminationRequest) => void;
}): Promise<{ notice: string } | null> {
  const resolution = resolveTerminationRevisionConflict(
    args.details,
    args.fallbackEnrollmentId,
  );
  if (resolution !== null) {
    args.openConfirm({
      projectId: args.projectId,
      issueId: args.issueId,
      enrollmentId: resolution.enrollmentId,
      policyRevision: resolution.policyRevision,
    });
    return { notice: resolution.notice };
  }
  let enrollment: IssueAutomationEnrollment | null = null;
  try {
    enrollment = await getAutomationEnrollment(args.projectId, args.issueId);
  } catch {
    enrollment = null;
  }
  if (!enrollment) {
    return null;
  }
  args.openConfirm({
    projectId: args.projectId,
    issueId: args.issueId,
    enrollmentId: enrollment.enrollment_id,
    policyRevision: enrollment.policy_revision,
  });
  return {
    notice: `已刷新自动化 enrollment 身份（rev ${enrollment.policy_revision}），请重新确认终止`,
  };
}

/** 409 且身份无法自动刷新时的可操作兜底文案（不静默、不冒充失败结论之外的东西）。 */
export function conflictUnrecoverableMessage(message: string): string {
  const base = message.trim() === "" ? "终止请求失败" : message;
  return `${base}；自动化身份已过期且无法自动刷新，请刷新页面后重新发起终止`;
}

/**
 * 会话失效最小提示：WS 侧 attempt 已不存在（被删除/换代）时页面持有的身份
 * 必然陈旧——明确提示刷新，避免用户继续在旧投影上操作。
 */
export function AutomationSessionInvalidHint() {
  const sessionInvalidated = useCodingWorkspaceStore(
    (state) => state.sessionInvalidated,
  );
  if (!sessionInvalidated) {
    return null;
  }
  return (
    <div
      data-testid="automation-session-invalid"
      role="status"
      className="flex shrink-0 flex-wrap items-center gap-2 rounded-md border border-[var(--aria-danger)] bg-red-50 px-3 py-2 text-xs leading-4 text-[var(--aria-danger)]"
    >
      <span className="font-semibold">该会话已失效</span>
      <span>
        服务端已找不到该 coding 会话（可能已删除或换代），页面身份已陈旧，请刷新页面后再操作。
      </span>
      <button
        type="button"
        onClick={() => window.location.reload()}
        className={BTN_SECONDARY_CLASS}
      >
        刷新页面
      </button>
    </div>
  );
}
