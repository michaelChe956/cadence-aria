// LC 管理面板抽屉化：右侧滑入轻量抽屉外壳（自研，无第三方依赖）。
// 职责仅外壳：半透明遮罩 + 右侧面板 + 进出场过渡 + Esc/遮罩/关闭按钮三路关闭
// + 打开时焦点移入、卸载时归还 + Tab 循环圈闭 + body 滚动锁定；
// 面板内容（LogicalCodebaseManagementPanel）经 children 注入，外壳不改其结构。
// 外壳实现已上提为共享组件（shared/ManagementDrawer）：LC 管理抽屉与
// Provider 管理抽屉共用同一套浮层规范（单一约定，不各写一份）。
// 视觉规范：w-[min(560px,92vw)]、200ms transform/opacity 过渡、motion-reduce 无过渡直切、
// hover 仅变色不位移、层级沿用项目浮层惯例 z-50（普通弹窗同级、系统守卫 z-[100] 之下）。
import type { ReactNode } from "react";
import { ManagementDrawer } from "../shared/ManagementDrawer";

type LogicalCodebaseManagementDrawerProps = {
  open: boolean;
  onClose: () => void;
  children: ReactNode;
};

export function LogicalCodebaseManagementDrawer({
  open,
  onClose,
  children,
}: LogicalCodebaseManagementDrawerProps) {
  return (
    <ManagementDrawer
      open={open}
      onClose={onClose}
      label="逻辑代码库管理"
      testIdPrefix="lc-management"
    >
      {children}
    </ManagementDrawer>
  );
}
