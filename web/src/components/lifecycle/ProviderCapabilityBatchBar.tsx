// 票3「一键核验全部」批量条（REQ-PCR-03/04/05/06 的前端面）。
//
// - 「一键核验全部」按钮 → POST 发起异步作业（立即拿到 job_id），按钮变
//   「核验中（N/4）」并禁用；2.5s 轮询 durable 作业，逐项渲染
//   已确认 / 核验中 / 核验失败 / 跳过（CLI 不可用）/ 超时；
// - 失败行【重试】走既有单 provider 端点（不重跑整批）；
// - 跳过行显示【安装】入口 —— 复用块2 的 `ProviderInstallActionButton`
//   （本组件不自建安装实现、不自判版本、不写安装文案）；
// - 终态停轮询；job_id 记在 localStorage，刷新后按 job_id 继续（durable）。
import { useCallback, useEffect, useState } from "react";
import { ApiRequestError } from "../../api/client";
import {
  getCapabilityRevalidateBatchJob,
  startCapabilityRevalidateBatch,
} from "../../api/capability-revalidate-batch";
import { postLogicalCodebaseCapabilityRevalidate } from "../../api/provider-capability";
import { getInstallTargets } from "../../api/provider-install";
import type {
  CapabilityRevalidateBatchItemDto,
  CapabilityRevalidateBatchJobDto,
} from "../../api/types/capability-revalidate-batch";
import type { ProviderCapabilityType } from "../../api/types/provider-capability";
import type { InstallTargetView } from "../../api/types/provider";
import { ProviderInstallActionButton } from "./ProviderInstallActionButton";

const POLL_INTERVAL_MS = 2_500;
const BATCH_SIZE = 4;

const PROVIDER_LABELS: Record<ProviderCapabilityType, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
  pi: "Pi",
  kimi_code: "Kimi Code",
};

const STATUS_LABELS: Record<CapabilityRevalidateBatchItemDto["status"], string> = {
  running: "核验中",
  confirmed: "已确认",
  failed: "核验失败",
  // 跳过 ≠ 核验失败：CLI 不在场时引导安装，不谎报失败也不谎报已确认。
  skipped: "跳过（CLI 不可用）",
  timeout: "超时（该项未完成，未确认）",
};

function storageKey(projectId: string, logicalCodebaseId: string): string {
  return `aria:capability-batch:${projectId}:${logicalCodebaseId}`;
}

function readStoredJobId(
  projectId: string,
  logicalCodebaseId: string,
): string | null {
  try {
    return window.localStorage.getItem(storageKey(projectId, logicalCodebaseId));
  } catch {
    return null;
  }
}

function storeJobId(
  projectId: string,
  logicalCodebaseId: string,
  jobId: string | null,
): void {
  try {
    const key = storageKey(projectId, logicalCodebaseId);
    if (jobId === null) {
      window.localStorage.removeItem(key);
    } else {
      window.localStorage.setItem(key, jobId);
    }
  } catch {
    // localStorage 不可用只影响「刷新续轮询」，不影响核验本身。
  }
}

function errorMessage(error: unknown): string {
  if (error instanceof ApiRequestError) {
    return `${error.code}：${error.message}`;
  }
  return error instanceof Error ? error.message : String(error);
}

/** 逐项安装面入口。版本判定归块2（`install-targets.action`，H3：本组件不自判
 *  版本、不自写安装文案）：`install`=CLI 不可用→【安装】；`update`=版本落后
 *  →【更新】+ 当前/最新版本；`verify`=已安装未核验→【核验】（接既有单 provider
 *  端点，不留一个点不动的按钮）；`none`/未知→无入口。 */
type InstallEntryKind = "install" | "update" | "verify";

function installEntryKind(
  item: CapabilityRevalidateBatchItemDto,
  target: InstallTargetView | undefined,
): InstallEntryKind | null {
  // 「CLI 不可用」是逐项结果的硬事实：优先引导安装，不谎报版本落后。
  if (item.status === "skipped") {
    return "install";
  }
  if (target === undefined) {
    return null;
  }
  if (target.action === "install") {
    return "install";
  }
  if (target.action === "update") {
    return "update";
  }
  if (target.action === "verify" && item.status !== "confirmed") {
    return "verify";
  }
  return null;
}

type ProviderCapabilityBatchBarProps = {
  projectId: string | null;
  logicalCodebaseId: string | null;
};

export function ProviderCapabilityBatchBar({
  projectId,
  logicalCodebaseId,
}: ProviderCapabilityBatchBarProps) {
  const [jobId, setJobId] = useState<string | null>(null);
  const [job, setJob] = useState<CapabilityRevalidateBatchJobDto | null>(null);
  const [starting, setStarting] = useState(false);
  const [retrying, setRetrying] = useState<ProviderCapabilityType | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [installTargets, setInstallTargets] = useState<
    Partial<Record<ProviderCapabilityType, InstallTargetView>>
  >({});

  // 本地/最新版本与「安装/更新/核验」动作一律取块2 的 install-targets
  // （REQ-PCI-02 判定归块2）；取不到时退化为「仅 CLI 不可用 → 安装」。
  const refreshInstallTargets = useCallback(async (): Promise<void> => {
    try {
      const targets = await getInstallTargets();
      const byTarget: Partial<
        Record<ProviderCapabilityType, InstallTargetView>
      > = {};
      for (const target of targets) {
        if (target.target !== "codegraph") {
          byTarget[target.target] = target;
        }
      }
      setInstallTargets(byTarget);
    } catch {
      setInstallTargets({});
    }
  }, []);

  useEffect(() => {
    void refreshInstallTargets();
  }, [refreshInstallTargets]);

  // 切换 LC / 首次挂载：按 localStorage 的 job_id 续读 durable 作业
  // （刷新不丢进度）。
  useEffect(() => {
    setJob(null);
    setError(null);
    setJobId(
      projectId !== null && logicalCodebaseId !== null
        ? readStoredJobId(projectId, logicalCodebaseId)
        : null,
    );
  }, [projectId, logicalCodebaseId]);

  const refresh = useCallback(
    async (id: string): Promise<void> => {
      if (projectId === null || logicalCodebaseId === null) {
        return;
      }
      try {
        const next = await getCapabilityRevalidateBatchJob(
          projectId,
          logicalCodebaseId,
          id,
        );
        setJob(next);
        setError(null);
      } catch (caught) {
        setError(errorMessage(caught));
        // 未知 / 过期 job_id：停止续轮询（不无限打 404）。
        storeJobId(projectId, logicalCodebaseId, null);
        setJobId(null);
      }
    },
    [projectId, logicalCodebaseId],
  );

  const running = jobId !== null && (job === null || job.status === "running");
  const done = job === null
    ? 0
    : job.results.filter((item) => item.status !== "running").length;

  // 2.5s 轮询；终态后不再排下一拍（不无限轮询）。
  useEffect(() => {
    if (!running || jobId === null) {
      return;
    }
    const delay = job === null ? 0 : POLL_INTERVAL_MS;
    const timer = setTimeout(() => {
      void refresh(jobId);
    }, delay);
    return () => clearTimeout(timer);
  }, [running, jobId, job, refresh]);

  const start = useCallback(async (): Promise<void> => {
    if (projectId === null || logicalCodebaseId === null) {
      return;
    }
    setStarting(true);
    setError(null);
    try {
      const started = await startCapabilityRevalidateBatch(
        projectId,
        logicalCodebaseId,
      );
      storeJobId(projectId, logicalCodebaseId, started.job_id);
      setJob(null);
      setJobId(started.job_id);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setStarting(false);
    }
  }, [projectId, logicalCodebaseId]);

  // 失败项单独重试：走既有单 provider 端点（不重跑整批），随后刷新作业。
  const retry = useCallback(
    async (providerType: ProviderCapabilityType): Promise<void> => {
      if (projectId === null || logicalCodebaseId === null) {
        return;
      }
      setRetrying(providerType);
      setError(null);
      try {
        await postLogicalCodebaseCapabilityRevalidate(
          projectId,
          logicalCodebaseId,
          providerType,
        );
        if (jobId !== null) {
          await refresh(jobId);
        }
        await refreshInstallTargets();
      } catch (caught) {
        setError(errorMessage(caught));
      } finally {
        setRetrying(null);
      }
    },
    [projectId, logicalCodebaseId, jobId, refresh, refreshInstallTargets],
  );

  if (projectId === null || logicalCodebaseId === null) {
    return null;
  }

  return (
    <section
      data-testid="provider-capability-batch-bar"
      className="border-b border-[var(--aria-line)] px-3 py-2"
    >
      <div className="flex items-center justify-between gap-3">
        <h3 className="text-xs font-semibold text-[var(--aria-ink)]">
          批量核验
        </h3>
        <button
          type="button"
          data-testid="capability-batch-start"
          disabled={running || starting}
          onClick={() => {
            void start();
          }}
          className="rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-1.5 text-xs font-semibold text-white disabled:opacity-60"
        >
          {running ? `核验中（${done}/${BATCH_SIZE}）` : "一键核验全部"}
        </button>
      </div>
      <p
        data-testid="capability-batch-scope"
        className="mt-1 text-[11px] text-[var(--aria-ink-muted)]"
      >
        逐 LC 核验：会真实跑一次探测，可能几分钟，消耗 provider 额度。
      </p>
      {error !== null ? (
        <p role="alert" className="mt-1 text-[11px] text-[var(--aria-danger)]">
          {error}
        </p>
      ) : null}
      {job !== null ? (
        <ul className="mt-2 space-y-1">
          {job.results.map((item) => {
            const target = installTargets[item.provider_type];
            const entry = installEntryKind(item, target);
            return (
              <li
                key={item.provider_type}
                data-testid={`capability-batch-row-${item.provider_type}`}
                className="flex flex-wrap items-center gap-2 text-[11px]"
              >
                <span className="font-semibold text-[var(--aria-ink)]">
                  {PROVIDER_LABELS[item.provider_type]}
                </span>
                <span
                  data-testid={`capability-batch-status-${item.provider_type}`}
                  className={
                    item.status === "failed"
                      ? "text-[var(--aria-danger)]"
                      : "text-[var(--aria-ink-muted)]"
                  }
                >
                  {STATUS_LABELS[item.status]}
                </span>
                {entry === "update" ? (
                  <span
                    data-testid={`capability-batch-outdated-${item.provider_type}`}
                    className="text-[var(--aria-primary)]"
                  >
                    版本落后
                  </span>
                ) : null}
                {item.status === "failed" ? (
                  <button
                    type="button"
                    data-testid={`capability-batch-retry-${item.provider_type}`}
                    disabled={retrying === item.provider_type}
                    onClick={() => {
                      void retry(item.provider_type);
                    }}
                    className="rounded-md border border-[var(--aria-primary)] px-2 py-0.5 text-[11px] font-semibold text-[var(--aria-primary)] disabled:opacity-60"
                  >
                    重试
                  </button>
                ) : null}
                {entry !== null ? (
                  <ProviderInstallActionButton
                    target={item.provider_type}
                    cliAvailable={entry !== "install"}
                    capabilityConfirmed={entry !== "verify"}
                    localVersion={target?.local_version ?? null}
                    latestVersion={target?.latest_version ?? null}
                    onSettled={() => {
                      void refreshInstallTargets();
                      if (jobId !== null) {
                        void refresh(jobId);
                      }
                    }}
                    onVerify={
                      entry === "verify"
                        ? () => {
                            void retry(item.provider_type);
                          }
                        : undefined
                    }
                  />
                ) : null}
              </li>
            );
          })}
        </ul>
      ) : null}
    </section>
  );
}
