// 从 CockpitInbox.tsx 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// LC 恢复等待卡动作面（显式 fresh 出口 + provider/断连/终态守卫）。
import { useEffect, useState } from "react";
import { getProviderOption } from "../../../state/provider-options";
import { useProviderAvailabilityStore } from "../../../state/provider-availability-store";
import type { LcResumeRecoveryInboxProjection } from "../../../state/workspace-cockpit-projection";
import { useWorkspaceStore } from "../../../state/workspace-ws-store";
import { BTN_PRIMARY_CLASS } from "../gate-visual-tokens";

/** 设计 §6：missing map（full 指纹不可用）的稳定判别码——只呈现事实，
 * 不猜漂移维度（provider_gateway::RESUME_FINGERPRINT_UNAVAILABLE 同源）。 */
const LC_RESUME_MAP_MISSING_REASON = "resume_fingerprint_unavailable";

/**
 * D（Task 7.1，设计 §6）：LC 恢复等待卡动作面——仅 waiting 态出
 * 「启动新会话继续修订」（显式 fresh：一次用户动作一个稳定 operation_id，
 * 由页面 newCommandId 生成并经 C6.2 sendLcResumeFresh 发送；本卡片零
 * 乐观本地变更，受理/回放/失败一律以 typed 结果帧＋SessionState 对账，
 * unknown 不伪成功也不自动换新 ID 重发）。inflight 禁双击：本地快照点击
 * 时的 durable updated_at，事实推进（受理离场或 prepare 失败回 waiting
 * 都会更新 updated_at）即解除禁用。observer（非 actionable）只显示事实
 * 不出写动作；断连/provider 不可用/终态 fail-closed 禁用并保留原因
 *（provider 守卫与 ChatInputBar REQ-PPS-03 同源：快照未加载如实按未知
 * 放行；终态为 fix-loop r1 P2 补位——fresh 链后端绕过 engine 终态拒收）。
 * fresh_reserved/fresh_running 无动作面——投影标题已呈
 * 「正在启动新原生会话…」。
 */
export function LcResumeRecoveryCard({
  info,
  actionable,
  onStartLcResumeFresh,
  connectionBlocked,
  terminalBlockReason,
}: {
  info: LcResumeRecoveryInboxProjection;
  actionable: boolean;
  onStartLcResumeFresh?: (recoveryId: string) => boolean;
  connectionBlocked: boolean;
  terminalBlockReason: string | null;
}) {
  const [requestedAtUpdatedAt, setRequestedAtUpdatedAt] = useState<string | null>(null);
  const providers = useWorkspaceStore((state) => state.providers);
  const providerSnapshot = useProviderAvailabilityStore((state) => state.snapshot);
  const authorProvider = providers?.author ?? "claude_code";
  const authorOption = providerSnapshot
    ? getProviderOption(providerSnapshot, authorProvider)
    : null;
  const providerBlockReason =
    authorOption && !authorOption.available
      ? `${authorOption.label} 当前不可用：${authorOption.reason ?? "原因未知"}`
      : null;

  useEffect(() => {
    // durable 事实推进（updated_at 变化）= 本次动作已有 typed 结局：受理
    // 离场（reserved/running，动作面随之消失）或失败回 waiting（复原可再
    // 次显式发起，属新用户动作＝新 operation_id）。
    if (requestedAtUpdatedAt !== null && info.updatedAt !== requestedAtUpdatedAt) {
      setRequestedAtUpdatedAt(null);
    }
  }, [info.updatedAt, requestedAtUpdatedAt]);

  if (info.state !== "waiting") {
    return null;
  }
  const inflight = requestedAtUpdatedAt !== null;
  const operationPending =
    info.lastOperation !== null &&
    (info.lastOperation.state === "reserved" || info.lastOperation.state === "running");
  const failedReason =
    info.lastOperation !== null && info.lastOperation.state === "failed"
      ? info.lastOperation.reason ?? "未知原因"
      : null;
  const disabled =
    inflight ||
    operationPending ||
    connectionBlocked ||
    providerBlockReason !== null ||
    terminalBlockReason !== null;
  const handleStart = () => {
    if (disabled || onStartLcResumeFresh === undefined) {
      return;
    }
    if (onStartLcResumeFresh(info.recoveryId)) {
      setRequestedAtUpdatedAt(info.updatedAt);
    }
  };
  return (
    <div className="mt-2" data-testid="lc-resume-recovery-actions">
      <p className="text-xs text-slate-600">
        {info.reasonCode === LC_RESUME_MAP_MISSING_REASON
          ? "无法验证旧会话冻结信息"
          : "运行环境已变化，不能继续旧原生会话。当前工作区、产物、反馈与评审意见均保留。"}
      </p>
      {failedReason ? (
        <p className="mt-1 text-xs text-[var(--aria-danger)]">
          上次启动未成功：{failedReason}
        </p>
      ) : null}
      {onStartLcResumeFresh !== undefined && actionable ? (
        <>
          <p className="mt-1 text-xs text-slate-500">
            只更换 AI 原生会话，将重新校验权限并继续当前任务
          </p>
          <div className="mt-2 flex flex-wrap gap-2">
            <button
              type="button"
              disabled={disabled}
              onClick={handleStart}
              className={`${BTN_PRIMARY_CLASS} disabled:cursor-not-allowed disabled:opacity-60`}
            >
              启动新会话继续修订
            </button>
          </div>
          {connectionBlocked ? (
            <p className="mt-1 text-xs text-slate-500">连接已断开，恢复连接后可继续</p>
          ) : null}
          {terminalBlockReason !== null ? (
            <p className="mt-1 text-xs text-slate-500">{terminalBlockReason}</p>
          ) : null}
          {providerBlockReason !== null ? (
            <p className="mt-1 text-xs text-[var(--aria-danger)]">{providerBlockReason}</p>
          ) : null}
        </>
      ) : null}
    </div>
  );
}
