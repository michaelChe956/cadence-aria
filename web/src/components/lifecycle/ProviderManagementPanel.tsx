// Provider 管理面板（独立于 LC 管理面板）：
// 2026-11-05 机器级化（owner 裁定 + REQ-PCR-08）——provider 的探针/核验/
// 更新/安装/版本是【机器级】能力（核验只核验一次，机器一份事实），与项目、
// 逻辑代码库都无关 ⇒ 整个 ProviderCapabilityCard（只读能力投影 + 一键核验
// 全部 + 逐家核验/更新/安装 + 版本落后提示 + 核验回显）在这里渲染，不接收
// 任何项目/逻辑代码库标识。
// 本组件只做容器与作用域说明，不新增统计口径、不复制能力卡的判定逻辑。
import { ProviderCapabilityCard } from "./ProviderCapabilityCard";

export function ProviderManagementPanel() {
  return (
    <div data-testid="provider-management-panel" className="px-3 py-2">
      <p
        data-testid="provider-management-scope"
        className="mb-2 text-[11px] text-[var(--aria-ink-muted)]"
      >
        Provider 能力核验、安装与版本管理属机器级能力，与项目、逻辑代码库无关。
      </p>
      <ProviderCapabilityCard />
    </div>
  );
}
