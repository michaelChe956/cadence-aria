// add-provider-revalidate-probe Task 3.2：LC provider capability 卡——
// 只读 durable 投影（GET capabilities）渲染四家 provider 的
// launch/write_boundary/resume 三行聚合状态，按 provider 提供「核验」
// 按钮显式触发真实探针（POST capability-revalidate，首次分钟级同步等待），
// already_confirmed/revalidated/probing/failed 四态如实回显；成功后重读
// 同一 GET 面刷新状态（不乐观改 durable 事实）。
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
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-xs font-semibold text-[var(--aria-ink)]">
          Provider 能力核验
        </h3>
        <span className="text-xs text-[var(--aria-ink-muted)]">
        </span>
      </div>
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
      {projection === null && loadError === null ? (
        <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">载入中…</p>
      ) : null}
      {projection ? (
        <ul className="mt-2 grid grid-cols-1 gap-2 sm:grid-cols-2">
          {projection.providers.map((provider) => {
            const feedback = feedbackByProvider[provider.provider_type] ?? null;
            const installView = installTargets[provider.provider_type] ?? null;
            // 三行聚合全 Confirmed 才算已核验（与 admission fail-closed 同口径）。
            const providerConfirmed = CAPABILITY_ASPECTS.every(
              (aspect) => aspectStatus(provider, aspect) === "confirmed",
            );
            return (
              <li
                key={provider.provider_type}
                data-testid={`provider-capability-provider-${provider.provider_type}`}
                className="rounded border border-[var(--aria-line)] px-2 py-1.5"
              >
                <div className="flex flex-wrap items-baseline justify-between gap-x-2">
                  <span className="text-xs font-semibold text-[var(--aria-ink)]">
                    {PROVIDER_LABELS[provider.provider_type] ??
                      provider.provider_type}
                  </span>
                  <span className="flex items-baseline gap-2">
                    <span
                      data-testid={`provider-capability-${provider.provider_type}-version`}
                      className="text-xs text-[var(--aria-ink-muted)]"
                    >
                      {provider.version ?? "未探测"}
                    </span>
                    <button
                      type="button"
                      disabled={busyProvider !== null}
                      onClick={() =>
                        void handleRevalidate(provider.provider_type)
                      }
                      className="min-h-9 rounded-md border border-[var(--aria-primary)] bg-white px-2.5 py-1 text-xs font-semibold text-[var(--aria-primary)] transition-colors duration-200 hover:bg-[var(--aria-panel-muted)] disabled:cursor-not-allowed disabled:opacity-60"
                      data-testid={`provider-capability-revalidate-${provider.provider_type}`}
                    >
                      核验{" "}
                      {PROVIDER_LABELS[provider.provider_type] ??
                        provider.provider_type}
                    </button>
                  </span>
                </div>
                <ul className="mt-1 space-y-1">
                  {CAPABILITY_ASPECTS.map((aspect) => (
                    <li
                      key={aspect}
                      className="flex flex-wrap items-baseline gap-x-2 text-xs text-[var(--aria-ink-muted)]"
                    >
                      <span className="font-semibold text-[var(--aria-ink)]">
                        {ASPECT_LABELS[aspect]}
                      </span>
                      <span
                        data-testid={`provider-capability-${provider.provider_type}-${aspect}`}
                      >
                        {
                          ASPECT_STATUS_LABELS[
                            aspectStatus(provider, aspect)
                          ]
                        }
                      </span>
                    </li>
                  ))}
                </ul>
                {/* 安装 / 更新入口（REQ-PCI-02）：未安装 → 【安装】+ 手动命令；
                    已安装且已核验但版本落后 → 【更新】。已安装但未核验时由本行
                    既有「核验」按钮负责（不显示安装，也不重复渲染一个核验）。 */}
                {installView &&
                (!installView.cli_available || providerConfirmed) ? (
                  <ProviderInstallActionButton
                    target={installView.target}
                    cliAvailable={installView.cli_available}
                    localVersion={installView.local_version ?? provider.version}
                    latestVersion={installView.latest_version}
                    capabilityConfirmed={providerConfirmed}
                    onSettled={() => {
                      void reload();
                      void reloadInstallTargets();
                    }}
                  />
                ) : null}
                {feedback ? (
                  <p
                    data-testid={`provider-capability-result-${provider.provider_type}`}
                    className={
                      feedback.state === "failed"
                        ? "mt-1 text-xs text-[var(--aria-danger)]"
                        : "mt-1 text-xs text-[var(--aria-ink-muted)]"
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
      {/* codegraph 纳入同一安装机制（11.10）：它没有 capability 探针，只按
          本地是否可用给【安装】（+ 手动命令）或【更新】。 */}
      {installTargets.codegraph ? (
        <div
          data-testid="provider-install-target-codegraph"
          className="mt-2 rounded border border-[var(--aria-line)] px-2 py-1.5"
        >
          <div className="flex flex-wrap items-baseline justify-between gap-x-2">
            <span className="text-xs font-semibold text-[var(--aria-ink)]">
              codegraph
            </span>
            <span className="text-xs text-[var(--aria-ink-muted)]">
              {installTargets.codegraph.local_version ?? "未探测"}
            </span>
          </div>
          <ProviderInstallActionButton
            target="codegraph"
            cliAvailable={installTargets.codegraph.cli_available}
            localVersion={installTargets.codegraph.local_version}
            latestVersion={installTargets.codegraph.latest_version}
            capabilityConfirmed
            onSettled={() => {
              void reload();
              void reloadInstallTargets();
            }}
          />
        </div>
      ) : null}
    </div>
  );
}
