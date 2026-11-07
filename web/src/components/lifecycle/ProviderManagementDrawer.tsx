// Provider 管理抽屉：独立于「逻辑代码库管理」抽屉的第二个管理浮层。
// owner 2026-11-04 裁定 + 2026-11-05 机器级化（REQ-PCR-08）——provider 的
// 探针/核验/更新/安装/版本属【机器级】能力（核验只核验一次，机器一份事实，
// 与项目、逻辑代码库无关），不该寄居在 LC 仓抽屉里 ⇒ 独立入口 + 独立面板。
// 外形与交互沿用 LC 管理抽屉同一套浮层规范（shared/ManagementDrawer），
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
