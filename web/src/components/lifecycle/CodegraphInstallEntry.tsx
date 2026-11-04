// 块2 11.10：codegraph 缺失的失败面上的【安装】入口。
//
// 复用 provider-cli-installation 的同一套安装机制——`ProviderInstallActionButton`
// ＋ `GET /api/providers/install-targets`（后端 source_table 判定本地可用性与
// 版本），与登记向导面（LogicalCodebaseRegistrationWizard）、能力卡面
// （ProviderCapabilityCard）同款；不新造安装通道、不新造安装文案。
//
// 装好提示的文案取后端原文：安装来源用安装目标面的 `install_source`
// （后端 source_table 渲染，如「npm @colbymchenry/codegraph」）；失败面
// 「怎么做」用后端 failure copy 的 action_zh（BootstrapFailureLine 已渲染）。
// `ProviderHealthEntry.install_hint` 只覆盖四家真实 provider（后端
// `ProviderName` 枚举里没有 codegraph）⇒ codegraph 没有该字段，不在此编造。
//
// 【安装】与【重新检测】是两个独立按钮、语义不重叠：这里只管「把 codegraph
// 装上」，装完后的复核由失败面既有的「重新检测」（POST /api/providers/recheck）
// 承担。
import { useCallback, useEffect, useState } from "react";
import { getInstallTargets } from "../../api/provider-install";
import type { InstallTargetView } from "../../api/types";
import { ProviderInstallActionButton } from "./ProviderInstallActionButton";

/**
 * codegraph 环境阻断码（后端 `aggregate_index/codegraph_cli.rs` 的错误码，
 * `bootstrap_projector.inc.rs` 按它落成 notice / operation error）。
 */
export const CODEGRAPH_INSTALL_CODES = [
  "codegraph_missing",
  "codegraph_version_mismatch",
] as const;

/** 该失败/通知码是否属于「codegraph 环境前置被阻断」⇒ 需要安装入口。 */
export function isCodegraphInstallCode(reasonCode: string | null | undefined): boolean {
  return CODEGRAPH_INSTALL_CODES.some((code) => code === reasonCode);
}

/**
 * codegraph 的安装入口（未装 →【安装】；版本落后 →【更新】）。安装目标读不
 * 到就不渲染入口（不凭空断言本机未安装——与向导面同一纪律）。
 */
export function CodegraphInstallEntry({ className }: { className?: string }) {
  const [target, setTarget] = useState<InstallTargetView | null>(null);

  const refresh = useCallback(async () => {
    try {
      const views = await getInstallTargets();
      setTarget(views.find((view) => view.target === "codegraph") ?? null);
    } catch {
      setTarget(null);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  if (target === null) {
    return null;
  }

  return (
    <div data-testid="lc-codegraph-install-entry" className={className}>
      <div className="flex flex-wrap items-baseline justify-between gap-x-2">
        <span className="text-xs font-semibold text-[var(--aria-ink)]">codegraph</span>
        <span className="text-xs text-[var(--aria-ink-muted)]">
          {target.local_version ?? "未探测"}
        </span>
      </div>
      <p
        className="mt-0.5 text-xs text-[var(--aria-ink-muted)]"
        data-testid="lc-codegraph-install-source"
      >
        安装来源：{target.install_source}
      </p>
      <ProviderInstallActionButton
        target="codegraph"
        cliAvailable={target.cli_available}
        localVersion={target.local_version}
        latestVersion={target.latest_version}
        capabilityConfirmed
        onSettled={() => void refresh()}
      />
    </div>
  );
}
