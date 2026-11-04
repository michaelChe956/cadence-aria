// add-provider-revalidate-probe Task 3.2：LC provider capability 面板——
// 只读 durable 投影（GET capabilities）渲染四家 provider 的
// launch/write_boundary/resume 三行聚合状态，按 provider 提供「核验」
// 按钮显式触发真实探针（POST capability-revalidate，首次分钟级同步等待），
// already_confirmed/revalidated/probing/failed 四态如实回显；成功后重读
// 同一 GET 面刷新状态（不乐观改 durable 事实）。
//
// owner 2026-11-04 裁定：去掉 2×2 方格卡片，只保留行级列表 ⇒ 四家 provider
// 各占一行（名字 / 状态 / 三个能力维度 / 版本 / 版本落后 / 安装更新入口 /
// 核验按钮）；「一键核验全部」按钮仍在标题行右侧、作用域说明仍在按钮下方；
// 批量核验作业的逐项状态并入同一行（不再另起第二组列表）。能力核验的
// 行为、接口调用与状态语义均未改动。
import { useCallback, useEffect, useState } from "react";
import { ApiRequestError } from "../../api/client";
import { getInstallTargets } from "../../api/provider-install";
import {
  getLogicalCodebaseCapabilities,
  postLogicalCodebaseCapabilityRevalidate,
} from "../../api/provider-capability";
import type {
  InstallTargetView,
  ProviderCapabilityProviderSnapshotDto,
  ProviderCapabilitiesResponse,
} from "../../api/types";
import {
  capabilityBatchStatusLabel,
  ProviderCapabilityBatchScope,
  ProviderCapabilityBatchStartButton,
  useProviderCapabilityBatch,
} from "./ProviderCapabilityBatch";
import { ProviderInstallActionButton } from "./ProviderInstallActionButton";

const PROVIDER_LABELS: Record<string, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
  pi: "Pi",
  kimi_code: "Kimi Code",
};

const CAPABILITY_ASPECTS = ["launch", "write_boundary", "resume"] as const;
type CapabilityAspect = (typeof CAPABILITY_ASPECTS)[number];

const ASPECT_LABELS: Record<CapabilityAspect, string> = {
  launch: "启动",
  write_boundary: "写边界",
  resume: "恢复",
};

const ASPECT_STATUS_LABELS: Record<string, string> = {
  confirmed: "已核验",
  unknown: "未核验",
  denied: "拒绝",
};

/** provider 级状态文案（行级列表的一列）：三个能力维度全 Confirmed 才
 *  「已确认」（与 admission fail-closed 同口径，不夸大）；否则「未核验」。 */
const PROVIDER_STATUS_LABELS = {
  confirmed: "已确认",
  unknown: "未核验",
} as const;

/** 单 aspect 聚合：三 action 行全 Confirmed 才已核验（与 admission
 * fail-closed 同口径，不夸大）；任一 Unknown 仍未核验（重验证可补）；
 * 全已知且含 Denied → 拒绝（真实探针如实拒绝的证据）。 */
function aspectStatus(
  provider: ProviderCapabilityProviderSnapshotDto,
  aspect: CapabilityAspect,
): "confirmed" | "unknown" | "denied" {
  const cells = provider.rows.map((row) => row[aspect]);
  if (cells.includes("unknown")) {
    return "unknown";
  }
  return cells.includes("denied") ? "denied" : "confirmed";
}

type RevalidateFeedback =
  | { state: "probing" }
  | { state: "already_confirmed"; version: string }
  | { state: "revalidated"; version: string; importedActions: string[] }
  | { state: "failed"; code: string; message: string; detail: string | null };

type ProviderCapabilityCardProps = {
  projectId: string | null;
  logicalCodebaseId: string | null;
};

export function ProviderCapabilityCard({
  projectId,
  logicalCodebaseId,
}: ProviderCapabilityCardProps) {
  const [projection, setProjection] = useState<ProviderCapabilitiesResponse | null>(
    null,
  );
  const [loadError, setLoadError] = useState<string | null>(null);
  const [installTargets, setInstallTargets] = useState<
    Record<string, InstallTargetView>
  >({});
  const [installTargetsError, setInstallTargetsError] = useState<string | null>(
    null,
  );
  const [busyProvider, setBusyProvider] = useState<string | null>(null);
  const [feedbackByProvider, setFeedbackByProvider] = useState<
    Record<string, RevalidateFeedback>
  >({});
  // 「一键核验全部」的作业状态（REQ-PCR-06：按钮由能力卡提供 ⇒ 状态挂在卡上）。
  const batch = useProviderCapabilityBatch(projectId, logicalCodebaseId);

  const reload = useCallback(async (): Promise<void> => {
    if (projectId === null || logicalCodebaseId === null) {
      return;
    }
    try {
      const response = await getLogicalCodebaseCapabilities(
        projectId,
        logicalCodebaseId,
      );
      if (!Array.isArray(response?.providers)) {
        // 负载不合规（无 providers 数组）如实报错，不静默空白、不崩溃。
        setProjection(null);
        setLoadError("capability 状态面负载不合规（缺少 providers 数组）");
        return;
      }
      setProjection(response);
      setLoadError(null);
    } catch (error) {
      setProjection(null);
      setLoadError(error instanceof Error ? error.message : "状态读取失败");
    }
  }, [projectId, logicalCodebaseId]);

  /** 安装目标面（本地可用性 / 版本 / 安装源）：与能力投影并行读取。
   *  读取失败只降级为「不提供安装入口 + 提示」，不影响能力三行的展示。 */
  const reloadInstallTargets = useCallback(async (): Promise<void> => {
    if (projectId === null || logicalCodebaseId === null) {
      return;
    }
    try {
      const views = await getInstallTargets();
      const byTarget: Record<string, InstallTargetView> = {};
      for (const view of views) {
        byTarget[view.target] = view;
      }
      setInstallTargets(byTarget);
      setInstallTargetsError(null);
    } catch (error) {
      setInstallTargets({});
      setInstallTargetsError(
        error instanceof Error ? error.message : "安装状态读取失败",
      );
    }
  }, [projectId, logicalCodebaseId]);

  useEffect(() => {
    if (projectId === null || logicalCodebaseId === null) {
      return;
    }
    setProjection(null);
    setInstallTargets({});
    setFeedbackByProvider({});
    void reload();
    void reloadInstallTargets();
  }, [projectId, logicalCodebaseId, reload, reloadInstallTargets]);

  if (projectId === null || logicalCodebaseId === null) {
    return null;
  }

  async function handleRevalidate(providerType: string): Promise<void> {
    setBusyProvider(providerType);
    setFeedbackByProvider((current) => ({
      ...current,
      [providerType]: { state: "probing" },
    }));
    try {
      const result = await postLogicalCodebaseCapabilityRevalidate(
        projectId!,
        logicalCodebaseId!,
        providerType as ProviderCapabilityProviderSnapshotDto["provider_type"],
      );
      setFeedbackByProvider((current) => ({
        ...current,
        [providerType]:
          result.outcome === "already_confirmed"
            ? { state: "already_confirmed", version: result.version }
            : {
                state: "revalidated",
                version: result.version,
                importedActions: [...result.imported_actions],
              },
      }));
      // 成功后重读同一 GET 面：三行状态以 durable 事实刷新，不乐观改。
      await reload();
      await reloadInstallTargets();
    } catch (error) {
      if (error instanceof ApiRequestError) {
        const rawDetail = (error.details as Record<string, unknown> | undefined)
          ?.detail;
        setFeedbackByProvider((current) => ({
          ...current,
          [providerType]: {
            state: "failed",
            code: error.code,
            message: error.message,
            detail: typeof rawDetail === "string" ? rawDetail : null,
          },
        }));
      } else {
        setFeedbackByProvider((current) => ({
          ...current,
          [providerType]: {
            state: "failed",
            code: "web_client_error",
            message: error instanceof Error ? error.message : "核验失败",
            detail: null,
          },
        }));
      }
    } finally {
      setBusyProvider(null);
    }
  }

  return (
    <div
      data-testid="provider-capability-card"
      className="border-b border-[var(--aria-line)] px-3 py-2"
    >
      {/* 头部右侧 = 「一键核验全部」入口（REQ-PCR-06：能力卡 SHALL 提供该按钮；
          owner 2026-11-04 裁定：按钮放在上面、不要放在下面）。 */}
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-xs font-semibold text-[var(--aria-ink)]">
          Provider 能力核验
        </h3>
        <ProviderCapabilityBatchStartButton state={batch} />
      </div>
      <ProviderCapabilityBatchScope />
      {loadError ? (
        <p
          data-testid="provider-capability-load-error"
          className="mt-1 text-xs text-[var(--aria-danger)]"
        >
          {loadError}
        </p>
      ) : null}
      {installTargetsError ? (
        <p
          data-testid="provider-install-targets-error"
          className="mt-1 text-xs text-[var(--aria-danger)]"
        >
          安装状态读取失败：{installTargetsError}
        </p>
      ) : null}
      {batch.error !== null ? (
        <p role="alert" className="mt-1 text-[11px] text-[var(--aria-danger)]">
          {batch.error}
        </p>
      ) : null}
      {projection === null && loadError === null ? (
        <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">载入中…</p>
      ) : null}
      {/* 行级列表（owner 2026-11-04：删掉 2×2 方格卡片，只保留这一组行）：
          四家 provider 各一行；批量核验作业的逐项状态并入同一行。 */}
      {projection ? (
        <ul
          data-testid="provider-capability-list"
          className="mt-2 divide-y divide-[var(--aria-line)]"
        >
          {projection.providers.map((provider) => {
            const providerType = provider.provider_type;
            const label = PROVIDER_LABELS[providerType] ?? providerType;
            const feedback = feedbackByProvider[providerType] ?? null;
            const installView = installTargets[providerType] ?? null;
            // 批量作业的逐项结果（作业未发起时为 null）：并入本行展示。
            const batchItem =
              batch.job?.results.find(
                (item) => item.provider_type === providerType,
              ) ?? null;
            // 三行聚合全 Confirmed 才算已核验（与 admission fail-closed 同口径）。
            const providerConfirmed = CAPABILITY_ASPECTS.every(
              (aspect) => aspectStatus(provider, aspect) === "confirmed",
            );
            // 已核验 = durable 投影的聚合结论，或批量作业已确认（任一成立；
            // 批量作业的逐项结论与投影同源，作业成功时投影可能尚未刷新）。
            const confirmed = providerConfirmed || batchItem?.status === "confirmed";
            // 版本落后由安装目标面判定（块2 `install-targets.action`）。
            const outdated = installView?.action === "update";
            return (
              <li
                key={providerType}
                data-testid={`provider-capability-provider-${providerType}`}
                className="flex flex-wrap items-center gap-x-2 gap-y-1 py-1 text-xs"
              >
                <span className="font-semibold text-[var(--aria-ink)]">
                  {label}
                </span>
                <span
                  data-testid={`provider-capability-status-${providerType}`}
                  className="text-[var(--aria-ink-muted)]"
                >
                  {providerConfirmed
                    ? PROVIDER_STATUS_LABELS.confirmed
                    : PROVIDER_STATUS_LABELS.unknown}
                </span>
                {batchItem !== null ? (
                  <span
                    data-testid={`capability-batch-status-${providerType}`}
                    className={
                      batchItem.status === "failed"
                        ? "text-[var(--aria-danger)]"
                        : "text-[var(--aria-ink-muted)]"
                    }
                  >
                    批量：{capabilityBatchStatusLabel(batchItem.status)}
                  </span>
                ) : null}
                {outdated ? (
                  <span
                    data-testid={`provider-capability-outdated-${providerType}`}
                    className="text-[var(--aria-primary)]"
                  >
                    版本落后
                  </span>
                ) : null}
                <span
                  data-testid={`provider-capability-${providerType}-version`}
                  className="text-[var(--aria-ink-muted)]"
                >
                  {provider.version ?? "未探测"}
                </span>
                {/* 三个能力维度（启动 / 写边界 / 恢复）：原方格卡片里的聚合
                    状态，删卡片后并入本行（不丢信息）。 */}
                {CAPABILITY_ASPECTS.map((aspect) => (
                  <span
                    key={aspect}
                    className="flex items-baseline gap-x-1 text-[var(--aria-ink-muted)]"
                  >
                    <span className="font-semibold text-[var(--aria-ink)]">
                      {ASPECT_LABELS[aspect]}
                    </span>
                    <span
                      data-testid={`provider-capability-${providerType}-${aspect}`}
                    >
                      {ASPECT_STATUS_LABELS[aspectStatus(provider, aspect)]}
                    </span>
                  </span>
                ))}
                <button
                  type="button"
                  disabled={busyProvider !== null}
                  onClick={() => void handleRevalidate(providerType)}
                  className="min-h-9 rounded-md border border-[var(--aria-primary)] bg-white px-2.5 py-1 text-xs font-semibold text-[var(--aria-primary)] transition-colors duration-200 hover:bg-[var(--aria-panel-muted)] disabled:cursor-not-allowed disabled:opacity-60"
                  data-testid={`provider-capability-revalidate-${providerType}`}
                >
                  核验 {label}
                </button>
                {/* 安装 / 更新入口（REQ-PCI-02）：未安装 → 【安装】+ 手动命令；
                    已安装且已核验但版本落后 → 【更新】。已安装但未核验时由本行
                    既有「核验」按钮负责（不显示安装，也不重复渲染一个核验）。 */}
                {installView &&
                (!installView.cli_available || confirmed) ? (
                  <ProviderInstallActionButton
                    target={installView.target}
                    cliAvailable={installView.cli_available}
                    localVersion={installView.local_version ?? provider.version}
                    latestVersion={installView.latest_version}
                    capabilityConfirmed={confirmed}
                    onSettled={() => {
                      void reload();
                      void reloadInstallTargets();
                    }}
                  />
                ) : null}
                {/* 批量作业里的失败项：单项重试走单 provider 端点（不重跑整批）。 */}
                {batchItem !== null && batchItem.status === "failed" ? (
                  <button
                    type="button"
                    data-testid={`capability-batch-retry-${providerType}`}
                    disabled={batch.retrying === providerType}
                    onClick={() => {
                      void batch.retry(providerType).then(() => {
                        void reload();
                        void reloadInstallTargets();
                      });
                    }}
                    className="rounded-md border border-[var(--aria-primary)] px-2 py-0.5 text-[11px] font-semibold text-[var(--aria-primary)] disabled:opacity-60"
                  >
                    重试
                  </button>
                ) : null}
                {feedback ? (
                  <p
                    data-testid={`provider-capability-result-${providerType}`}
                    className={
                      feedback.state === "failed"
                        ? "text-xs text-[var(--aria-danger)]"
                        : "text-xs text-[var(--aria-ink-muted)]"
                    }
                  >
                    {feedback.state === "probing"
                      ? "核验中：真实探针运行中（首次可能需要数分钟）…"
                      : feedback.state === "already_confirmed"
                        ? `already_confirmed（版本 ${feedback.version}，无需重跑探针）`
                        : feedback.state === "revalidated"
                          ? `revalidated（已导入 ${feedback.importedActions.join("、")}，版本 ${feedback.version}）`
                          : `失败 ${feedback.code}：${feedback.message}${feedback.detail ? `；详情 ${feedback.detail}` : ""}`}
                  </p>
                ) : null}
              </li>
            );
          })}
        </ul>
      ) : null}
    </div>
  );
}
