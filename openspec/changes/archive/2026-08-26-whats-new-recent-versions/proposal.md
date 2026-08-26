# Proposal: whats-new-recent-versions

## Why

更新动态弹窗当前只展示当前版本单个条目，历史条目虽在数据中保留但永不可见；用户希望更新动态成为"最近版本变更"的滚动视图，并在其中预先准备下一版条目（发布时自动生效）。

## What Changes

- 更新动态弹窗从单条目改为**滚动窗口**：展示"版本不高于当前版本（`CURRENT_VERSION`）"的最新至多 4 个条目，新→旧排列，当前版本区块在最上。
- 触发与已读语义不变：仍只在当前版本未读时弹出，关闭标记当前版本已读。
- `CHANGELOG` 追加 `0.0.9` 预备条目（2 条要点：图片文件存储改造、kimi provider 修复）；因 `CURRENT_VERSION` 仍为 `0.0.8`，该条目在发布前不展示；日期字段为发布时填写的占位值。
- 数组维护约定：发布升版本时人工裁剪数组保持 4 条；新增测试锁定"展示窗口 ≤ 4"与"数组新→旧有序"。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `whats-new-dialog`: 弹窗内容由"当前版本单条"改为"最近 ≤4 个版本（不高于当前版本）"；其余 Requirement（已读标记、手写中文要点、localStorage 降级）不变。

## Impact

- 前端：`web/src/whats-new/changelog.ts`（新条目+维护注释）、`useWhatsNew.ts`（窗口选择逻辑）、`components/whats-new/WhatsNewDialog.tsx`（多区块渲染）、对应测试。
- 不改：触发时机、localStorage key 与降级行为、后端、版本号常量（保持 0.0.8）。
