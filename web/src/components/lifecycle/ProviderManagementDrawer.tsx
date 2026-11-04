// Provider 管理抽屉：独立于「逻辑代码库管理」抽屉的第二个管理浮层。
// owner 2026-11-04 裁定——provider 的探针/核验/更新/安装/版本属【项目级全局】
// 能力（能力核验已全局化，不再是 per-LC），不该寄居在 LC 仓抽屉里 ⇒ 独立入口 +
// 独立面板。外形与交互沿用 LC 管理抽屉同一套浮层规范（shared/ManagementDrawer），
// 但开关与 testid 完全独立（provider-management-*），两者互不干扰。
import type { ReactNode } from "react";
import { ManagementDrawer } from "../shared/ManagementDrawer";

type ProviderManagementDrawerProps = {
  open: boolean;
  onClose: () => void;
  children: ReactNode;
};

export function ProviderManagementDrawer({
  open,
  onClose,
  children,
}: ProviderManagementDrawerProps) {
  return (
    <ManagementDrawer
      open={open}
      onClose={onClose}
      label="Provider 管理"
      testIdPrefix="provider-management"
    >
      {children}
    </ManagementDrawer>
  );
}
