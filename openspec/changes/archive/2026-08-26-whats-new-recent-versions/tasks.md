## 1. 数据与窗口逻辑

- [x] 1.1 `changelog.ts`：新增 0.0.9 预备条目（2 条要点 + 日期占位注释"发布时替换为真实日期"），数组顶部加维护注释（发布升版本时裁剪至 4 条）；新增导出 `recentEntries(currentVersion: string, limit = 4): ChangelogEntry[]`（取不高于当前版本的前 4 条）；测试：数组新→旧有序、窗口 ≤4、高于当前版本的条目被排除、`CURRENT_VERSION=0.0.8` 时窗口为 0.0.8/0.0.7/0.0.6/0.0.5（0.0.9 被排除）
- [x] 1.2 `useWhatsNew.ts`：`entry` 改为 `entries = recentEntries(CURRENT_VERSION)`（触发/已读判定仍基于单当前版本，不变）；既有测试适配 + 新增断言（未读弹出时 entries 为 4 条且首条为当前版本）

## 2. 弹窗多版本渲染

- [x] 2.1 `WhatsNewDialog.tsx`：props 从 `entry: ChangelogEntry` 改为 `entries: ChangelogEntry[]`，按序渲染多版本区块（标题+日期+要点，区块间视觉分隔，沿用现有样式语言）；`app-shell.tsx` 传参适配；组件测试：4 个区块按新→旧渲染、标题/日期/要点正确、"知道了"关闭回调
- [x] 2.2 全量验证：`cd web && pnpm tsc -b && pnpm test` 全绿；`cargo fmt --check`、clippy、`cargo test --locked`（后端无改动，防意外回归）

## 3. 收尾

- [x] 3.1 提交 commit；`openspec validate whats-new-recent-versions` 通过；汇报（用户发布时需手动：升 `Cargo.toml`+`CURRENT_VERSION` 至 0.0.9、填真实日期、裁剪数组至 4 条）
