// Provider capability 面板（2026-11-05 机器级化，REQ-PCR-08：能力面不再按
// 项目/逻辑代码库定位 ⇒ 本组件不接收任何 id）——只读 durable 投影
// （GET capabilities）渲染四家 provider 的
// launch/write_boundary/resume 三行聚合状态，按 provider 提供「核验」
// 按钮显式触发真实探针（POST capability-revalidate，首次分钟级同步等待），
// already_confirmed/revalidated/probing/failed 四态如实回显；成功后重读
// 同一 GET 面刷新状态（不乐观改 durable 事实）。
//
// owner 2026-11-04 裁定（视觉重排）：去掉 2×2 方格卡片，也不做「一整行文字
// 流」⇒ 每家 provider 一个**块**，块之间用 1px 分割线（--aria-line）分隔
// （不套圆角边框卡片壳），块内固定三行并用 grid 对齐：
//   行1 provider 名（600 字重）· 右对齐版本（次要色 + 等宽字体）
//   行2 状态胶囊们（已确认 / 批量逐项状态 / 版本落后）
//   行3 左：三个能力维度胶囊（启动 / 写边界 / 恢复）· 右：操作按钮（右对齐）
// 窄屏（<640px）行3 允许上下堆叠，但用 gap 排布，绝不退化成连续文字流。
// 视觉契约以 design-system/MASTER.md 为准：胶囊 rounded-full（§6）、次按钮
// btn-secondary（§5）、1px 分割线（§3）、hover 只变颜色且 150-300ms（§5/§7）。
// 「一键核验全部」按钮、作用域说明与 codegraph 不出现等既有行为全部保持；
// 接口调用、参数、状态语义与回显文案均未改动。
import { useCallback, useEffect, useState } from "react";
import { ApiRequestError } from "../../api/client";
import {
  exposeToTerminalPath,
  getInstallTargets,
} from "../../api/provider-install";
import {
  getProviderCapabilities,
  postProviderCapabilityRevalidate,
} from "../../api/provider-capability";
import type {
  InstallTargetView,
  InstallTargetWire,
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
 * 「已确认」（与 admission fail-closed 同口径，不夸大）；否则「未核验」。 */
const PROVIDER_STATUS_LABELS = {
  confirmed: "已确认",
  unknown: "未核验",
} as const;

/** 胶囊基座（MASTER §6）：rounded-full + 1px 细边或无边框浅底；紧凑尺寸
 * （padding 2px 10px、字号 12px）。禁止方角 badge。 */
const CHIP_CLASS =
  "inline-flex items-center gap-1 rounded-full border px-2.5 py-0.5 text-xs font-medium";

/** 能力维度胶囊配色：已核验 = success-soft 底 + success 字；未核验 = 无边框
 * 弱化墨色；拒绝 = danger-soft 底 + danger 字（真实探针如实拒绝的证据）。 */
const ASPECT_CHIP_CLASSES: Record<
  "confirmed" | "unknown" | "denied",
  string
> = {
  confirmed:
    "border-[var(--aria-success)] bg-[var(--aria-success-soft)] text-[var(--aria-success)]",
  unknown:
    "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)]",
  denied:
    "border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] text-[var(--aria-danger)]",
};

/** provider 级状态胶囊配色：已确认 = success-soft 底 + success 字；未核验 =
 * 无边框 + 弱化墨色（未探测不谎报）。 */
const PROVIDER_STATUS_CHIP_CLASSES: Record<
  keyof typeof PROVIDER_STATUS_LABELS,
  string
> = {
  confirmed:
    "border-[var(--aria-success)] bg-[var(--aria-success-soft)] text-[var(--aria-success)]",
  unknown:
    "border-transparent bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)]",
};

/** 批量作业逐项状态胶囊配色：已确认 = success、核验中 = primary、失败 =
 * danger、跳过 / 超时 = warning（跳过 ≠ 失败，不谎报也不谎报已确认）。 */
const BATCH_STATUS_CHIP_CLASSES: Record<
  Parameters<typeof capabilityBatchStatusLabel>[0],
  string
> = {
  confirmed:
    "border-[var(--aria-success)] bg-[var(--aria-success-soft)] text-[var(--aria-success)]",
  running:
    "border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] text-[var(--aria-primary)]",
  failed:
    "border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] text-[var(--aria-danger)]",
  skipped:
    "border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] text-[var(--aria-warning)]",
  timeout:
    "border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] text-[var(--aria-warning)]",
};

/** 次按钮（MASTER §5）：淡蓝紫底 + 蓝紫字 + 无边框 + rounded-xl。仓内
 * `.btn-secondary` 已实现该契约（hover 只出现 1px 细边、transition 200ms），
 * 这里只覆盖本面板的紧凑密度与焦点可见性。 */
const SECONDARY_BUTTON_CLASS =
  "btn-secondary min-h-9 cursor-pointer px-2.5 py-1 text-xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-1 disabled:cursor-not-allowed disabled:opacity-60";

/** 安装 / 更新按钮由 `ProviderInstallActionButton`（另一个文件，不在本任务可改
 * 范围）渲染 ⇒ 在本组件内用后代选择器把它统一成本面板的次按钮外观
 * （MASTER §5）。只命中安装动作容器里的操作按钮，不动 Kimi 远程脚本确认框里
 * 的【确认并安装】/【取消】（那是主/次语义按钮，不是行级操作）。 */
const INSTALL_ACTION_CLASS =
  "min-w-0 max-w-full [&>div]:mt-0 [&>div>div>button]:rounded-xl [&>div>div>button]:border-transparent [&>div>div>button]:bg-[var(--aria-primary-soft)] [&>div>div>button]:text-[var(--aria-primary)] [&>div>div>button]:hover:border-[var(--aria-line-strong)] [&>div>div>button]:hover:bg-[var(--aria-primary-soft)]";

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

/** 「暴露到终端 PATH」的瞬态回显（§五 2.11）：进行中 / 成功（软链落在哪个
 *  目录、新终端可运行哪个命令）/ 失败（如实给出后端原因，不谎报成功）。 */
type ExposeFeedback =
  | { state: "busy" }
  | { state: "done"; dir: string; command: string }
  | { state: "failed"; message: string };

export function ProviderCapabilityCard() {
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
  // 「暴露到终端 PATH」的逐行回显（暴露不是安装动作 ⇒ 行内独立入口）。
  const [exposeFeedbackByProvider, setExposeFeedbackByProvider] = useState<
    Record<string, ExposeFeedback>
  >({});
  // 「一键核验全部」的作业状态（REQ-PCR-06：按钮由能力卡提供 ⇒ 状态挂在卡上）。
  const batch = useProviderCapabilityBatch();

  const reload = useCallback(async (): Promise<void> => {
    try {
      const response = await getProviderCapabilities();
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
  }, []);

  /** 安装目标面（本地可用性 / 版本 / 安装源）：与能力投影并行读取。
   *  读取失败只降级为「不提供安装入口 + 提示」，不影响能力三行的展示。 */
  const reloadInstallTargets = useCallback(async (): Promise<void> => {
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
  }, []);

  useEffect(() => {
    setProjection(null);
    setInstallTargets({});
    setFeedbackByProvider({});
    setExposeFeedbackByProvider({});
    void reload();
    void reloadInstallTargets();
  }, [reload, reloadInstallTargets]);

  async function handleRevalidate(providerType: string): Promise<void> {
    setBusyProvider(providerType);
    setFeedbackByProvider((current) => ({
      ...current,
      [providerType]: { state: "probing" },
    }));
    try {
      const result = await postProviderCapabilityRevalidate(
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

  /** §五 2.11：用户明确点击才把已装 CLI 软链到登录 shell PATH 上的可写目录
   *  （不改 shell profile、不改 PATH 环境变量）；成功后重读安装目标面 ⇒
   *  off_path 翻转时徽标与入口随之消失。失败如实回显后端原因。 */
  async function handleExpose(
    providerType: string,
    target: InstallTargetWire,
  ): Promise<void> {
    setExposeFeedbackByProvider((current) => ({
      ...current,
      [providerType]: { state: "busy" },
    }));
    try {
      const result = await exposeToTerminalPath(target);
      setExposeFeedbackByProvider((current) => ({
        ...current,
        [providerType]: {
          state: "done",
          dir: result.dir,
          command: result.command,
        },
      }));
      await reloadInstallTargets();
    } catch (error) {
      setExposeFeedbackByProvider((current) => ({
        ...current,
        [providerType]: {
          state: "failed",
          message:
            error instanceof ApiRequestError
              ? `暴露失败 ${error.code}：${error.message}`
              : error instanceof Error
                ? `暴露失败：${error.message}`
                : "暴露失败",
        },
      }));
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
      {/* 行级列表（owner 2026-11-04：删掉 2×2 方格卡片，也不做一整行文字流）：
          每家 provider 一个块，块间 1px 分割线（--aria-line），块内三行
          grid 对齐（名/版本 · 状态胶囊 · 维度胶囊 + 操作按钮）。 */}
      {projection ? (
        <ul
          data-testid="provider-capability-list"
          className="mt-2 divide-y divide-[var(--aria-line)]"
        >
          {projection.providers.map((provider) => {
            const providerType = provider.provider_type;
            const label = PROVIDER_LABELS[providerType] ?? providerType;
            const feedback = feedbackByProvider[providerType] ?? null;
            const exposeFeedback = exposeFeedbackByProvider[providerType] ?? null;
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
            const confirmed =
              providerConfirmed || batchItem?.status === "confirmed";
            // 版本落后由安装目标面判定（块2 `install-targets.action`）。
            const outdated = installView?.action === "update";
            const latestVersion = installView?.latest_version ?? null;
            return (
              <li
                key={providerType}
                data-testid={`provider-capability-provider-${providerType}`}
                className="grid gap-1.5 py-2"
              >
                {/* 行1：provider 名（600 字重）· 右对齐版本（次要色 + 等宽）。 */}
                <div
                  data-testid={`provider-capability-head-${providerType}`}
                  className="grid grid-cols-[1fr_auto] items-baseline gap-x-3"
                >
                  <span
                    data-testid={`provider-capability-name-${providerType}`}
                    className="truncate text-sm font-semibold text-[var(--aria-ink)]"
                  >
                    {label}
                  </span>
                  <span
                    data-testid={`provider-capability-${providerType}-version`}
                    className="aria-mono text-right text-xs text-[var(--aria-ink-muted)]"
                  >
                    {/* 版本取**最新**的一份事实：安装目标面（provider health）
                        在安装/更新后立刻刷新，能力投影要等下次「核验」才刷新
                        —— 用旧的投影版本会让用户以为更新没生效（2026-11-09）。 */}
                    {installView?.local_version ?? provider.version ?? "未探测"}
                  </span>
                </div>
                {/* 行2：状态胶囊（provider 级状态 / 批量逐项状态 / 版本落后）。 */}
                <div
                  data-testid={`provider-capability-status-row-${providerType}`}
                  className="flex flex-wrap items-center gap-x-2 gap-y-1"
                >
                  <span
                    data-testid={`provider-capability-status-${providerType}`}
                    className={`${CHIP_CLASS} ${
                      providerConfirmed
                        ? PROVIDER_STATUS_CHIP_CLASSES.confirmed
                        : PROVIDER_STATUS_CHIP_CLASSES.unknown
                    }`}
                  >
                    <span
                      aria-hidden="true"
                      className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                        providerConfirmed
                          ? "bg-[var(--aria-success)]"
                          : "bg-[var(--aria-line-strong)]"
                      }`}
                    />
                    {providerConfirmed
                      ? PROVIDER_STATUS_LABELS.confirmed
                      : PROVIDER_STATUS_LABELS.unknown}
                  </span>
                  {batchItem !== null ? (
                    <span
                      data-testid={`capability-batch-status-${providerType}`}
                      className={`${CHIP_CLASS} ${BATCH_STATUS_CHIP_CLASSES[batchItem.status]}`}
                    >
                      批量：{capabilityBatchStatusLabel(batchItem.status)}
                    </span>
                  ) : null}
                  {outdated ? (
                    <span
                      data-testid={`provider-capability-outdated-${providerType}`}
                      className={`${CHIP_CLASS} border-[var(--aria-cta)] bg-[var(--aria-cta-soft)] text-[var(--aria-cta)]`}
                    >
                      版本落后{latestVersion === null ? "" : ` → ${latestVersion}`}
                    </span>
                  ) : null}
                  {/* 已安装但不在登录 shell PATH（REQ-PCI-08）：仅提示徽标，
                      行动入口是既有「暴露到终端 PATH」按钮（不重复实现）。 */}
                  {installView?.off_path ? (
                    <span
                      data-testid={`provider-capability-off-path-${providerType}`}
                      className={`${CHIP_CLASS} border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] text-[var(--aria-warning)]`}
                    >
                      已安装，但不在你的终端 PATH 上
                    </span>
                  ) : null}
                </div>
                {/* 行3：左 = 三个能力维度胶囊；右 = 操作按钮（右对齐）。
                    空间不够时操作区整体换行（flex-wrap + gap），维度胶囊仍保持
                    一行，绝不退化成连续文字流。 */}
                <div
                  data-testid={`provider-capability-actions-row-${providerType}`}
                  className="flex flex-wrap items-center gap-x-3 gap-y-2"
                >
                  <div
                    data-testid={`provider-capability-aspects-${providerType}`}
                    className="flex flex-wrap items-center gap-x-2 gap-y-1"
                  >
                    {CAPABILITY_ASPECTS.map((aspect) => {
                      const status = aspectStatus(provider, aspect);
                      return (
                        <span
                          key={aspect}
                          data-testid={`provider-capability-aspect-chip-${providerType}-${aspect}`}
                          className={`${CHIP_CLASS} ${ASPECT_CHIP_CLASSES[status]}`}
                        >
                          <span className="text-[var(--aria-ink)]">
                            {ASPECT_LABELS[aspect]}
                          </span>
                          <span
                            data-testid={`provider-capability-${providerType}-${aspect}`}
                          >
                            {ASPECT_STATUS_LABELS[status]}
                          </span>
                        </span>
                      );
                    })}
                  </div>
                  <div
                    data-testid={`provider-capability-operations-${providerType}`}
                    className="ml-auto flex min-w-0 flex-wrap items-start justify-end gap-2"
                  >
                    <button
                      type="button"
                      disabled={busyProvider !== null}
                      onClick={() => void handleRevalidate(providerType)}
                      className={SECONDARY_BUTTON_CLASS}
                      data-testid={`provider-capability-revalidate-${providerType}`}
                    >
                      核验 {label}
                    </button>
                    {/* 安装 / 更新入口（REQ-PCI-02）：未安装 → 【安装】+ 手动命令；
                        已安装且已核验但版本落后 → 【更新】。已安装但未核验时由本行
                        既有「核验」按钮负责（不显示安装，也不重复渲染一个核验）。 */}
                    {installView &&
                    (!installView.cli_available || confirmed) ? (
                      <div className={INSTALL_ACTION_CLASS}>
                        <ProviderInstallActionButton
                          target={installView.target}
                          cliAvailable={installView.cli_available}
                          localVersion={
                            installView.local_version ?? provider.version
                          }
                          latestVersion={installView.latest_version}
                          capabilityConfirmed={confirmed}
                          exposeEnabled={false}
                          onSettled={() => {
                            void reload();
                            void reloadInstallTargets();
                          }}
                        />
                      </div>
                    ) : null}
                    {/* 「暴露到终端 PATH」不是安装动作（是「让用户终端能直接跑
                        CLI」的动作）⇒ 不挂在安装块里（已安装但**未核验**的行走
                        不到安装块，入口随之不可达）。这里按行独立渲染入口，
                        口径统一为「已安装 + 不在终端 PATH 上」（off_path=true，
                        无论是否已核验）；已在 PATH 上（off_path=false）时用户
                        终端本来就能跑，不加噪音；未安装没有可暴露的东西。
                        安装块内的同义按钮已由 exposeEnabled={false} 关掉，
                        同一动作只留一个入口。 */}
                    {installView &&
                    installView.cli_available &&
                    installView.off_path ? (
                      <button
                        type="button"
                        data-testid={`provider-capability-expose-${providerType}`}
                        disabled={exposeFeedback?.state === "busy"}
                        onClick={() =>
                          void handleExpose(providerType, installView.target)
                        }
                        className={SECONDARY_BUTTON_CLASS}
                      >
                        暴露到终端 PATH
                      </button>
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
                        className={SECONDARY_BUTTON_CLASS}
                      >
                        重试
                      </button>
                    ) : null}
                  </div>
                </div>
                {/* 核验回显（瞬态第四行：核验中 / already_confirmed / revalidated /
                    失败码 + detail，如实呈现，不乐观改状态）。 */}
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
                {/* 暴露回显（瞬态行：暴露中 / 成功（落在哪个目录、新终端可运行
                    哪个命令）/ 失败原因，如实呈现，不谎报已暴露）。 */}
                {exposeFeedback ? (
                  <p
                    data-testid={
                      exposeFeedback.state === "busy"
                        ? `provider-capability-expose-status-${providerType}`
                        : exposeFeedback.state === "done"
                          ? `provider-capability-expose-result-${providerType}`
                          : `provider-capability-expose-error-${providerType}`
                    }
                    className={
                      exposeFeedback.state === "failed"
                        ? "text-xs text-[var(--aria-danger)]"
                        : "text-xs text-[var(--aria-ink-muted)]"
                    }
                  >
                    {exposeFeedback.state === "busy"
                      ? "暴露中：正在把 CLI 软链到终端 PATH 上的可写目录…"
                      : exposeFeedback.state === "done"
                        ? `已暴露到 ${exposeFeedback.dir}，新终端可直接运行 ${exposeFeedback.command}`
                        : exposeFeedback.message}
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
