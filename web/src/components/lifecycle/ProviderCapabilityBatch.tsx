// 票3「一键核验全部」批量作业（REQ-PCR-03/04/05/06 的前端面）。
//
// - 「一键核验全部」按钮挂在**能力卡头部**（块3 design 决策 6.6「前端：能力卡
//   头部『一键核验全部』按钮」；主 spec REQ-PCR-06「能力卡 SHALL 提供『一键
//   核验全部』按钮」）。owner 2026-11-04 裁定：按钮放在上面、不要放在下面
//   ⇒ 作业状态由本模块的 hook 持有，按钮与逐项结果拆成两个可分别摆放的组件；
// - 按钮 → POST 发起异步作业（立即拿到 job_id），按钮变「核验中（N/4）」并
//   禁用；2.5s 轮询 durable 作业，逐项渲染
//   已确认 / 核验中 / 核验失败 / 跳过（CLI 不可用）/ 超时；
// - 失败行【重试】走既有单 provider 端点（不重跑整批）；
// - 跳过行显示【安装】入口 —— 复用块2 的 `ProviderInstallActionButton`
//   （本模块不自建安装实现、不自判版本、不写安装文案）；
// - 终态停轮询；job_id 记在 localStorage，刷新后按 job_id 继续（durable）。
import { useCallback, useEffect, useState } from "react";
import { ApiRequestError } from "../../api/client";
import {
  getCapabilityRevalidateBatchJob,
  startCapabilityRevalidateBatch,
} from "../../api/capability-revalidate-batch";
import { postLogicalCodebaseCapabilityRevalidate } from "../../api/provider-capability";
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

/** 逐项状态文案（能力面板的行级列表复用同一份标签，不两处各写一份）。 */
export function capabilityBatchStatusLabel(
  status: CapabilityRevalidateBatchItemDto["status"],
): string {
  return STATUS_LABELS[status];
}

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

/** 逐项安装面入口。版本判定归块2（`install-targets.action`，H3：本模块不自判
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

/** 批量作业状态（作业面 + 触发/重试动作）。能力卡按需摆放按钮与结果列表。 */
export type ProviderCapabilityBatchState = {
  /** 逐项结果（作业未发起时为 null）。 */
  job: CapabilityRevalidateBatchJobDto | null;
  /** 作业进行中（含刚发起、还没拿到首帧结果的窗口）。 */
  running: boolean;
  /** 已出结果的家数（按钮上的 N/4）。 */
  done: number;
  /** 发起请求在途（按钮禁用）。 */
  starting: boolean;
  /** 某家单项重试在途。 */
  retrying: ProviderCapabilityType | null;
  /** 作业面错误（发起/轮询/重试失败如实呈现）。 */
  error: string | null;
  /** 发起整批作业。 */
  start: () => Promise<void>;
  /** 失败项单独重试（走单 provider 端点，不重跑整批）。 */
  retry: (providerType: ProviderCapabilityType) => Promise<void>;
  /** 重读当前作业（安装/更新完成后刷新逐项结果）。 */
  refresh: () => Promise<void>;
};

/**
 * 批量核验作业的持有者：job_id 持久化在 localStorage，2.5s 轮询 durable 作业，
 * 终态停轮询。行为语义与原批量条一致（本模块只是把状态从「条」里拆出来，
 * 让按钮能放在能力卡头部、结果列表仍留在能力卡下方）。
 */
export function useProviderCapabilityBatch(
  projectId: string | null,
  logicalCodebaseId: string | null,
): ProviderCapabilityBatchState {
  const [jobId, setJobId] = useState<string | null>(null);
  const [job, setJob] = useState<CapabilityRevalidateBatchJobDto | null>(null);
  const [starting, setStarting] = useState(false);
  const [retrying, setRetrying] = useState<ProviderCapabilityType | null>(null);
  const [error, setError] = useState<string | null>(null);

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

  const refreshById = useCallback(
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
      void refreshById(jobId);
    }, delay);
    return () => clearTimeout(timer);
  }, [running, jobId, job, refreshById]);

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
          await refreshById(jobId);
        }
      } catch (caught) {
        setError(errorMessage(caught));
      } finally {
        setRetrying(null);
      }
    },
    [projectId, logicalCodebaseId, jobId, refreshById],
  );

  const refresh = useCallback(async (): Promise<void> => {
    if (jobId === null) {
      return;
    }
    await refreshById(jobId);
  }, [jobId, refreshById]);

  return {
    job,
    running,
    done,
    starting,
    retrying,
    error,
    start,
    retry,
    refresh,
  };
}

/**
 * 「一键核验全部」按钮（能力卡头部）。进行中禁用并显示「核验中（N/4）」。
 */
export function ProviderCapabilityBatchStartButton({
  state,
}: {
  state: ProviderCapabilityBatchState;
}) {
  return (
    <button
      type="button"
      data-testid="capability-batch-start"
      disabled={state.running || state.starting}
      onClick={() => {
        void state.start();
      }}
      className="rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-1.5 text-xs font-semibold text-white disabled:opacity-60"
    >
      {state.running ? `核验中（${state.done}/${BATCH_SIZE}）` : "一键核验全部"}
    </button>
  );
}

/**
 * 按钮区的作用域与成本说明（REQ-PCR-06「按钮区域 SHALL 说明：逐逻辑代码库 /
 * 真实探测 / 消耗额度 / 可能几分钟」）。紧贴按钮渲染，不藏进折叠区。
 */
export function ProviderCapabilityBatchScope() {
  return (
    <p
      data-testid="capability-batch-scope"
      className="mt-1 text-[11px] text-[var(--aria-ink-muted)]"
    >
      逐 LC 核验：会真实跑一次探测，可能几分钟，消耗 provider 额度。
    </p>
  );
}

/**
 * 批量作业的逐项结果区（能力卡下方）：作业面错误 + 逐项状态 / 单项重试 /
 * 安装更新入口。
 */
export function ProviderCapabilityBatchResults({
  state,
  installTargets,
  onSettled,
}: {
  state: ProviderCapabilityBatchState;
  /** 安装目标面（由能力卡读取块2 `install-targets` 后传入；本组件不自取）。 */
  installTargets: Record<string, InstallTargetView>;
  /** 安装/更新完成后的回调（能力卡据此重读安装目标面）。 */
  onSettled?: () => void;
}) {
  return (
    <div data-testid="provider-capability-batch-results" className="mt-2">
      {state.error !== null ? (
        <p role="alert" className="mt-1 text-[11px] text-[var(--aria-danger)]">
          {state.error}
        </p>
      ) : null}
      {state.job !== null ? (
        <ul className="mt-2 space-y-1">
          {state.job.results.map((item) => {
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
                    disabled={state.retrying === item.provider_type}
                    onClick={() => {
                      // 单项重试走单 provider 端点（不重跑整批）；完成后刷新
                      // 安装目标面（本地版本可能已变）。
                      void state.retry(item.provider_type).then(() => {
                        onSettled?.();
                      });
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
                      onSettled?.();
                      void state.refresh();
                    }}
                    onVerify={
                      entry === "verify"
                        ? () => {
                            void state.retry(item.provider_type);
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
    </div>
  );
}
