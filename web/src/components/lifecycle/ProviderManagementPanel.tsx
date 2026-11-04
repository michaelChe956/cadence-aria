// Provider 管理面板（独立于 LC 管理面板）：
// owner 2026-11-04 裁定——provider 的探针/核验/更新/安装/版本是【项目级全局】
// 能力（能力核验已全局化），不再寄居在「逻辑代码库」抽屉 ⇒ 整个
// ProviderCapabilityCard（只读能力投影 + 一键核验全部 + 逐家核验/更新/安装 +
// 版本落后提示 + 核验回显）整体搬到这里。
// 本组件只做容器与作用域说明，不新增统计口径、不复制能力卡的判定逻辑。
import { ProviderCapabilityCard } from "./ProviderCapabilityCard";

type ProviderManagementPanelProps = {
  projectId: string | null;
  /** 能力面 GET/POST 端点仍按逻辑代码库定位（承载 404 守卫）；内容与逻辑代码库
   *  无关（项目级全局）⇒ 传项目下任一已存在的逻辑代码库 id 即可。 */
  logicalCodebaseId: string | null;
};

export function ProviderManagementPanel({
  projectId,
  logicalCodebaseId,
}: ProviderManagementPanelProps) {
  return (
    <div data-testid="provider-management-panel" className="px-3 py-2">
      <p
        data-testid="provider-management-scope"
        className="mb-2 text-[11px] text-[var(--aria-ink-muted)]"
      >
        Provider 能力核验、安装与版本管理属项目级全局能力，与逻辑代码库无关。
      </p>
      <ProviderCapabilityCard
        projectId={projectId}
        logicalCodebaseId={logicalCodebaseId}
      />
    </div>
  );
}
