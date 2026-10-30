import { useCallback, useEffect, useRef, useState } from "react";
import type { RevokeFindingExclusionsOutcome } from "../../../state/workspace-finding-exclusions";
import type { FeedbackSenderWithOptions, ReviewFinding } from "../finding-list";

export type FindingExclusionRevokeFailure = { code: string; message: string };

/**
 * REQ-WSC-13/14（k3 P0-2C）门卡「排除/撤销排除」本地状态的单一事实源。
 *
 * - 排除（REQ-WSC-13）：出站走 HumanGateFeedback 的 excludedFindingIds（空串
 *   反馈），成功即本地置已排除——durable 回执由下一轮投影携带，本地态只作
 *   即时反馈；失败保持可点（fail-closed，不静默丢）。
 * - 撤销（REQ-WSC-14/k3 P0-2C）：走 REST `revoke_finding_exclusions`（会话级
 *   durable 事实）。成功后本地集合**整体对齐**回执的 `remaining_finding_keys`
 *   ——徽章/计数只认服务端权威清单，两边绝不漂移；失败不动徽章、原因如实
 *   上屏（不冒充成功）。
 * - 刷新恢复：挂载时发一次空 key 撤销请求（服务端幂等：无匹配 key 零写），
 *   回执的 remaining 即 durable 清单全量——页面刷新后「已排除」不再只在本地。
 *   owner 在本卡已有排除/撤销动作后，迟到的快照不回写（防覆盖新事实）。
 */
export function useGateFindingExclusions({
  cardId,
  sendFeedbackWithOptions,
  revokeSender,
  exclusionFrameRefused,
  markLocalAction,
  enabled,
}: {
  /** 门卡 id——durable 引导按卡一次性执行（重放/换卡不重读）。 */
  cardId: string;
  /** §6 契约：门面 feedback 的双参扩展签名（排除出站）。 */
  sendFeedbackWithOptions?: FeedbackSenderWithOptions;
  /** REST 撤销发送器（门面可选面）；缺省不提供撤销入口（fail-closed）。 */
  revokeSender?: (findingKeys: string[]) => Promise<RevokeFindingExclusionsOutcome>;
  /** REQ-WSC-15：排除被引擎拒收的证据在场（本地排除标记全部失效，宁可重点）。 */
  exclusionFrameRefused: boolean;
  /** 记录本卡最近一次本地门动作（协议错误归因用，ref 由门卡持有）。 */
  markLocalAction: (kind: "exclusion" | "feedback") => void;
  /** durable 引导放行判据：卡上确实有带 key 的 finding 且未收口（收口/无 findings 的卡不发只读 POST）。 */
  enabled: boolean;
}) {
  const [excludedFindingKeys, setExcludedFindingKeys] = useState<string[]>([]);
  const [revokingFindingKeys, setRevokingFindingKeys] = useState<string[]>([]);
  const [revokingAll, setRevokingAll] = useState(false);
  const [revokeFailure, setRevokeFailure] = useState<FindingExclusionRevokeFailure | null>(
    null,
  );
  const [revokeNotice, setRevokeNotice] = useState<string | null>(null);
  const localMutationRef = useRef(false);

  // 刷新后恢复 durable 已排除态：空 key 撤销 = 零写只读列表，remaining 即全量。
  useEffect(() => {
    if (!revokeSender || !enabled) {
      return;
    }
    let cancelled = false;
    void revokeSender([]).then((outcome) => {
      if (cancelled || !outcome.ok || localMutationRef.current) {
        return;
      }
      setExcludedFindingKeys(outcome.result.remaining_finding_keys);
    });
    return () => {
      cancelled = true;
    };
  }, [revokeSender, cardId, enabled]);

  const handleExcludeFinding = useCallback(
    (finding: ReviewFinding): boolean => {
      const findingKey = finding.findingKey;
      if (!sendFeedbackWithOptions || !findingKey) {
        return false;
      }
      markLocalAction("exclusion");
      const sent = sendFeedbackWithOptions("", { excludedFindingIds: [findingKey] });
      const excluded = sent === true;
      if (excluded) {
        localMutationRef.current = true;
        setRevokeFailure(null);
        setRevokeNotice(null);
        setExcludedFindingKeys((keys) =>
          keys.includes(findingKey) ? keys : [...keys, findingKey],
        );
      }
      return excluded;
    },
    [sendFeedbackWithOptions, markLocalAction],
  );

  const revoke = useCallback(
    async (findingKeys: string[], notice: string) => {
      if (!revokeSender || findingKeys.length === 0) {
        return;
      }
      setRevokingAll(findingKeys.length > 1);
      setRevokingFindingKeys((current) => [...new Set([...current, ...findingKeys])]);
      setRevokeFailure(null);
      setRevokeNotice(null);
      const outcome = await revokeSender(findingKeys);
      setRevokingAll(false);
      setRevokingFindingKeys((current) =>
        current.filter((key) => !findingKeys.includes(key)),
      );
      if (outcome.ok) {
        localMutationRef.current = true;
        // 服务端回执的 remaining 是权威清单——本地集合整体对齐。
        setExcludedFindingKeys(outcome.result.remaining_finding_keys);
        setRevokeNotice(notice);
        return;
      }
      // 失败不冒充成功：徽章/计数不动，code+message 如实上屏。
      setRevokeFailure({ code: outcome.code, message: outcome.message });
    },
    [revokeSender],
  );

  const handleRevokeExclusion = useCallback(
    (findingKey: string) => {
      void revoke([findingKey], "已撤销排除，这条下一轮会重新进入评审");
    },
    [revoke],
  );

  const handleRevokeAllExclusions = useCallback(
    (findingKeys: string[]) => {
      void revoke(
        findingKeys,
        `已撤销全部排除（${findingKeys.length} 项），下一轮会重新进入评审`,
      );
    },
    [revoke],
  );

  return {
    /** 本地已排除 key（未套用 fail-closed 抑制；门卡拒收注记判据用）。 */
    excludedFindingKeys,
    /** 展示用已排除 key：排除被引擎拒收的证据在场时全部失效（REQ-WSC-15）。 */
    effectiveExcludedFindingKeys: exclusionFrameRefused ? [] : excludedFindingKeys,
    revokingFindingKeys,
    revokingAll,
    revokeFailure,
    revokeNotice,
    handleExcludeFinding,
    handleRevokeExclusion,
    handleRevokeAllExclusions,
  };
}

/**
 * 门卡「已排除」计数注记 + 撤销反馈条：计数随撤销同步减一/归零，「全部撤销」
 * 一次清掉本卡全部已排除项；撤销失败的 code/message 与成功即时反馈就地相邻
 * 呈现（owner 不需要去别处找原因）。全部隐藏条件：无已排除、无反馈、无失败。
 */
export function GateExclusionStatusBar({
  excludedCount,
  revokeKeys,
  revokeAvailable,
  revokingAll,
  onRevokeAll,
  failure,
  notice,
}: {
  /** 本卡已排除 finding 数（durable 清单 ∩ 本卡 findings）。 */
  excludedCount: number;
  /** 本卡已排除项的 key 全集（「全部撤销」按它出站）。 */
  revokeKeys: string[];
  /** 门面是否提供撤销面（可选面，未接线不渲染撤销按钮）。 */
  revokeAvailable: boolean;
  revokingAll: boolean;
  onRevokeAll: (findingKeys: string[]) => void;
  failure: FindingExclusionRevokeFailure | null;
  notice: string | null;
}) {
  if (excludedCount === 0 && failure === null && notice === null) {
    return null;
  }
  return (
    <div className="space-y-1">
      {excludedCount > 0 ? (
        <p
          data-testid="gate-finding-excluded-note"
          className="flex flex-wrap items-center gap-2 text-xs font-medium text-emerald-700"
        >
          <span>
            已排除 {excludedCount} 项：owner 已处置的事实，下一轮评审不再报为需处理
          </span>
          {revokeAvailable && revokeKeys.length > 1 ? (
            <button
              type="button"
              data-testid="gate-revoke-all-exclusions"
              disabled={revokingAll}
              onClick={() => onRevokeAll(revokeKeys)}
              className="font-medium text-slate-600 underline underline-offset-2 transition-colors hover:text-slate-900 disabled:pointer-events-none disabled:opacity-50"
            >
              {revokingAll ? "全部撤销中…" : "全部撤销"}
            </button>
          ) : null}
        </p>
      ) : null}
      {notice !== null ? (
        <p
          data-testid="gate-finding-revoke-notice"
          className="text-xs font-medium text-slate-700"
        >
          {notice}
        </p>
      ) : null}
      {failure !== null ? (
        <p
          data-testid="gate-finding-revoke-failed"
          className="text-xs font-medium text-red-700"
        >
          撤销排除未生效（{failure.code}）：{failure.message}
        </p>
      ) : null}
    </div>
  );
}
