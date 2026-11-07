// C5：enrollment target 展示文案的共享约定（单仓 / 单成员 LC / 多成员 LC）。
// 后端 EnrollmentTarget 仍有三变体，但**第三变体（single_repository）仅
// legacy 只读**：新投影只出逻辑代码库两变体；老单仓 enrollment 记录仍要能
// 显示（不退化为「target 未知」）。前端渲染必须覆盖全部变体；任何分支都
// 不得输出 undefined/null/[object Object]——缺字段一律用「—」占位。

/** 展示用占位：非字符串或空串一律显示「—」。 */
export function enrollmentTargetPlaceholder(value: unknown): string {
  return typeof value === "string" && value.trim() !== "" ? value : "—";
}

/** 多成员 LC 的成员数量文案：N 个成员 / 无成员 / 缺字段「—」。 */
export function enrollmentTargetMemberCountText(
  logicalRepositoryIds: unknown,
): string {
  if (!Array.isArray(logicalRepositoryIds)) {
    return "—";
  }
  const count = logicalRepositoryIds.length;
  return count === 0 ? "无成员" : `${count} 个成员`;
}
