// provider-cli-installation（块2 片7B）：能力探针处每家的安装 / 更新入口。
//
// 显示规则（REQ-PCI-02，由探针结果 + 本地状态共同决定）：
//   未安装（命令缺失）          → 【安装】+ 手动命令
//   已安装但未核验（探针未通过）→ 【核验】（不显示安装；已核验才会显示更新）
//   已安装已核验但版本落后      → 【更新】+ 当前版本 vs 最新版本
//   已安装且已是最新            → 不渲染任何按钮（返回 null）
// 已安装（任意版本，安装 / 更新后皆可）还能点【暴露到终端 PATH】（§五
// 2.11）：把 CLI 软链到登录 shell PATH 上的可写目录，用户终端里敲命令才
// 真的有反应；只在用户明确点击时调用，后端不写 shell profile、不改 PATH。
// Kimi Code 走远程安装脚本：点击安装前必须告知「将执行来自 code.kimi.com 的
// 安装脚本」，确认后才发 POST（acknowledge_remote_script=true）；且 Kimi 的
// 版本来源是 npm 包 `@moonshot-ai/kimi-code` ⇒ 同样显示「更新」（REQ-PCI-01 / REQ-PCI-07）。
// 安装是异步作业：POST 立即返回 job_id，组件自带轮询（durable），终态后回调
// onSettled 让父级重读 health（安装 ≠ 核验，REQ-PCI-05）。
import { useEffect, useRef, useState } from "react";
import { ApiRequestError } from "../../api/client";
import {
  compareVersions,
  exposeToTerminalPath,
  getInstallJob,
  manualInstallCommand,
  startInstall,
} from "../../api/provider-install";
import type {
  ExposeTerminalPathResult,
  InstallFailureKind,
  InstallJob,
  InstallTargetWire,
} from "../../api/types";

const POLL_INTERVAL_MS = 2_000;

const FAILURE_KIND_LABELS: Record<InstallFailureKind, string> = {
  permission: "权限不足",
  network: "网络问题",
  package_not_found: "找不到这个包",
  other: "安装失败",
};

export interface ProviderInstallActionButtonProps {
  target: InstallTargetWire;
  cliAvailable: boolean;
  localVersion?: string | null;
  latestVersion?: string | null; // Kimi 恒 null → 永不显示「更新」
  capabilityConfirmed: boolean;
  /** 是否渲染「暴露到终端 PATH」按钮（默认 true ⇒ 既有宿主行为不变）。
   *  false ⇒ 本组件**不**渲染该按钮（暴露入口由父级自己按需渲染，避免同一
   *  动作出现两个同义入口）。 */
  exposeEnabled?: boolean;
  /** 作业进入终态（completed/failed）后回调：父级据此重读 health 投影。 */
  onSettled?: () => void;
  /** 可选扩展：父级把既有「核验」通道（POST capability-revalidate）接进来。
   *  未提供时「核验」按钮禁用并提示到能力核验卡操作。 */
  onVerify?: () => void;
}

export function ProviderInstallActionButton({
  target,
  cliAvailable,
  localVersion,
  latestVersion,
  capabilityConfirmed,
  exposeEnabled = true,
  onSettled,
  onVerify,
}: ProviderInstallActionButtonProps): React.ReactElement | null {
  const [jobId, setJobId] = useState<string | null>(null);
  const [job, setJob] = useState<InstallJob | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [scriptNoticeVisible, setScriptNoticeVisible] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // §五 2.11：暴露到终端 PATH 的进行中 / 结果 / 失败文案。
  const [exposing, setExposing] = useState(false);
  const [exposeResult, setExposeResult] =
    useState<ExposeTerminalPathResult | null>(null);
  const [exposeError, setExposeError] = useState<string | null>(null);

  // 回调放进 ref：父级每次 render 换新的函数体不应重启轮询。
  const onSettledRef = useRef(onSettled);
  useEffect(() => {
    onSettledRef.current = onSettled;
  }, [onSettled]);
  const settledJobIdRef = useRef<string | null>(null);

  const running = submitting || job?.status === "running";

  // 轮询：作业 durable，刷新页面后仍会继续查；终态即停（timer 随依赖变化清理）。
  useEffect(() => {
    if (jobId === null || job === null || job.status !== "running") {
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const next = await getInstallJob(jobId);
        if (!cancelled) {
          setJob(next);
        }
      } catch (pollError) {
        if (!cancelled) {
          setJob(null);
          setJobId(null);
          setError(
            pollError instanceof ApiRequestError
              ? `作业状态读取失败 ${pollError.code}：${pollError.message}`
              : pollError instanceof Error
                ? `作业状态读取失败：${pollError.message}`
                : "作业状态读取失败",
          );
        }
      }
    }, POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [jobId, job]);

  // 终态只回调一次（同一个 job 不重复触发父级刷新）。
  useEffect(() => {
    if (job === null || job.status === "running") {
      return;
    }
    if (settledJobIdRef.current === job.job_id) {
      return;
    }
    settledJobIdRef.current = job.job_id;
    onSettledRef.current?.();
  }, [job]);

  async function beginInstall(acknowledgeRemoteScript: boolean): Promise<void> {
    setScriptNoticeVisible(false);
    setError(null);
    setSubmitting(true);
    try {
      const started = await startInstall(
        target,
        acknowledgeRemoteScript ? { acknowledgeRemoteScript: true } : undefined,
      );
      setJobId(started.job_id);
      setJob({
        job_id: started.job_id,
        target,
        status: "running",
        created_at: "",
        updated_at: "",
        outcome: null,
        failure: null,
      });
    } catch (startError) {
      setError(
        startError instanceof ApiRequestError
          ? `发起安装失败 ${startError.code}：${startError.message}`
          : startError instanceof Error
            ? `发起安装失败：${startError.message}`
            : "发起安装失败",
      );
    } finally {
      setSubmitting(false);
    }
  }

  function handleInstallClick(): void {
    if (target === "kimi_code") {
      // 远程脚本：先告知来源，用户确认后才真正发起（REQ-PCI-01）。
      setScriptNoticeVisible(true);
      return;
    }
    void beginInstall(false);
  }

  // §五 2.11：用户明确点击才把已装 CLI 软链到登录 shell PATH 上的可写
  // 目录；成功 ⇒ 刷新提示（隐藏「请把 … 加入 PATH」旧句），失败 ⇒ 如实
  // 展示后端给出的可行动原因（候选目录与各自不合格原因）。
  async function handleExpose(): Promise<void> {
    setExposeError(null);
    setExposing(true);
    try {
      setExposeResult(await exposeToTerminalPath(target));
    } catch (exposeFailure) {
      setExposeResult(null);
      setExposeError(
        exposeFailure instanceof ApiRequestError
          ? `暴露失败 ${exposeFailure.code}：${exposeFailure.message}`
          : exposeFailure instanceof Error
            ? `暴露失败：${exposeFailure.message}`
            : "暴露失败",
      );
    } finally {
      setExposing(false);
    }
  }

  const versionGap =
    localVersion != null && latestVersion != null
      ? compareVersions(localVersion, latestVersion)
      : Number.NaN;
  // 版本来源与安装来源分离（REQ-PCI-07）：Kimi Code 的版本来源是 npm 包
  // `@moonshot-ai/kimi-code` ⇒ 同样参与「有新版」判定、同样显示【更新】；
  // 它的**安装源**仍是官方脚本（点击后先告知来源并确认，REQ-PCI-01）。
  const showUpdate = capabilityConfirmed && versionGap < 0;

  const showInstall = !cliAvailable;
  const showVerify = cliAvailable && !capabilityConfirmed;

  if (!showInstall && !showVerify && !showUpdate && !cliAvailable) {
    return null;
  }

  const buttonClass =
    "min-h-9 rounded-md border px-2.5 py-1 text-xs font-semibold transition-colors duration-200 disabled:cursor-not-allowed disabled:opacity-60";

  return (
    <div
      data-testid={`provider-install-action-${target}`}
      className="mt-1 min-w-0 max-w-full space-y-1"
    >
      <div className="flex flex-wrap items-center gap-2">
        {showInstall ? (
          <button
            type="button"
            data-testid={`provider-install-start-${target}`}
            disabled={running}
            onClick={handleInstallClick}
            className={`${buttonClass} border-[var(--aria-primary)] bg-white text-[var(--aria-primary)] hover:bg-[var(--aria-panel-muted)]`}
          >
            安装
          </button>
        ) : null}
        {showVerify ? (
          <button
            type="button"
            data-testid={`provider-install-verify-${target}`}
            disabled={onVerify === undefined}
            title={
              onVerify === undefined
                ? "请在下方能力核验卡中核验该 provider"
                : undefined
            }
            onClick={() => onVerify?.()}
            className={`${buttonClass} border-[var(--aria-primary)] bg-white text-[var(--aria-primary)] hover:bg-[var(--aria-panel-muted)]`}
          >
            核验
          </button>
        ) : null}
        {showUpdate ? (
          <button
            type="button"
            data-testid={`provider-install-start-${target}`}
            disabled={running}
            onClick={handleInstallClick}
            className={`${buttonClass} border-[var(--aria-primary)] bg-white text-[var(--aria-primary)] hover:bg-[var(--aria-panel-muted)]`}
          >
            更新
          </button>
        ) : null}
        {showUpdate ? (
          <span
            data-testid={`provider-install-versions-${target}`}
            className="text-xs text-[var(--aria-ink-muted)]"
          >
            当前 {localVersion} → 最新 {latestVersion}
          </span>
        ) : null}
        {cliAvailable && exposeEnabled ? (
          <button
            type="button"
            data-testid={`provider-install-expose-${target}`}
            disabled={exposing}
            onClick={() => void handleExpose()}
            className={`${buttonClass} border-[var(--aria-line)] bg-white text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)]`}
          >
            暴露到终端 PATH
          </button>
        ) : null}
      </div>

      {showInstall ? (
        <p
          data-testid={`provider-install-manual-command-${target}`}
          className="break-words text-xs text-[var(--aria-ink-muted)]"
        >
          手动安装：
          <code className="ml-1 break-all rounded bg-[var(--aria-panel-muted)] px-1 py-0.5">
            {manualInstallCommand(target)}
          </code>
        </p>
      ) : null}

      {scriptNoticeVisible ? (
        <div
          data-testid={`provider-install-kimi-script-notice-${target}`}
          className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-1.5"
        >
          <p className="text-xs text-[var(--aria-ink)]">
            将执行来自 code.kimi.com 的安装脚本（远程 shell 脚本）。确认后才发起{showUpdate ? "更新" : "安装"}。
          </p>
          <div className="mt-1 flex flex-wrap gap-2">
            <button
              type="button"
              data-testid={`provider-install-confirm-${target}`}
              onClick={() => void beginInstall(true)}
              className={`${buttonClass} border-[var(--aria-primary)] bg-[var(--aria-primary)] text-white`}
            >
              确认并{showUpdate ? "更新" : "安装"}
            </button>
            <button
              type="button"
              data-testid={`provider-install-cancel-${target}`}
              onClick={() => setScriptNoticeVisible(false)}
              className={`${buttonClass} border-[var(--aria-line)] bg-white text-[var(--aria-ink)]`}
            >
              取消
            </button>
          </div>
        </div>
      ) : null}

      {running ? (
        <p
          data-testid={`provider-install-status-${target}`}
          className="break-words text-xs text-[var(--aria-ink-muted)]"
        >
          安装中：正在执行安装命令，可能需要数分钟；作业状态是 durable 的，刷新页面不会丢。
        </p>
      ) : null}

      {!running && job?.status === "completed" ? (
        <p
          data-testid={`provider-install-status-${target}`}
          className="break-words text-xs text-[var(--aria-ink-muted)]"
        >
          已安装
          {job.outcome?.installed_version
            ? `：${job.outcome.installed_version}`
            : ""}
          {job.outcome?.path_hint && !exposeResult ? (
            // 后端 path_hint 本身是完整句子（2026-11-13 契约）：原样渲染，
            // 前端不再二次包裹（否则用户看到破碎双句，误判「安装没效果」）。
            <span
              data-testid={`provider-install-path-hint-${target}`}
              className="mt-1 block break-words"
            >
              {job.outcome.path_hint}
            </span>
          ) : null}
        </p>
      ) : null}

      {exposeResult ? (
        <p
          data-testid={`provider-install-expose-result-${target}`}
          className="break-words text-xs text-[var(--aria-ink-muted)]"
        >
          已暴露到 {exposeResult.dir}，新终端可直接运行 {exposeResult.command}
        </p>
      ) : null}

      {exposeError ? (
        <p
          data-testid={`provider-install-expose-error-${target}`}
          className="break-words text-xs text-[var(--aria-danger)]"
        >
          {exposeError}
        </p>
      ) : null}

      {!running && job?.status === "failed" ? (
        <div
          data-testid={`provider-install-status-${target}`}
          className="break-words text-xs text-[var(--aria-danger)]"
        >
          <p>
            {job.failure
              ? `装不上（${FAILURE_KIND_LABELS[job.failure.kind]}）：${job.failure.message}`
              : "安装失败"}
          </p>
          {job.failure?.manual_command ? (
            <p className="mt-1">
              手动安装：
              <code className="ml-1 break-all rounded bg-[var(--aria-panel-muted)] px-1 py-0.5">
                {job.failure.manual_command}
              </code>
            </p>
          ) : null}
        </div>
      ) : null}

      {error ? (
        <p
          data-testid={`provider-install-error-${target}`}
          className="break-words text-xs text-[var(--aria-danger)]"
        >
          {error}
        </p>
      ) : null}
    </div>
  );
}
