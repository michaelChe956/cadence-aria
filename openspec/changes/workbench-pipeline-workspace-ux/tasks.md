## 1. 派生层与选择语义（对应 Requirement: 下一步动作分组与过滤 / 统一选择语义与持续归属高亮）

- [ ] 1.1 `deriveIssueQueue` 纯函数（分组、mini-graph 阶段状态、文本过滤、渲染上限 50）+ 表驱动单测覆盖全部分组边界。验证：新单测全绿。
- [ ] 1.2 统一选择语义：队列行高亮改由 `focusedIssueId` 驱动 + `aria-current`；`handleSelectCard` 非 issue 分支同步 `focusedIssueId`；`selectedCardKey` 仅工作区内生效。验证：点 Story/Design 后 Issue 行高亮保持的回归测试通过。

## 2. 紧凑队列与折叠外壳（对应 Requirement: 紧凑 Issue 队列与阶段 mini-graph / 队列滚动边界与渲染上限 / 队列折叠与监督/专注双密度）

- [ ] 2.1 `IssueQueueRow`（单行密度 + mini-graph + hover 操作）与 `IssueQueue`（吸顶过滤条 + 分组折叠 + 显示更多）组件及测试。验证：单行密度、按需操作、分组折叠、过滤零网络请求测试通过。
- [ ] 2.2 外壳改 `100dvh` + 队列/工作区独立滚动 + 队列折叠（按 Project 记忆，localStorage 持久化）。验证：长列表不撑高整页、折叠/展开不改焦点与网络请求、折叠状态记忆测试通过。

## 3. 阶段标签工作区（对应 Requirement: 阶段标签工作区）

- [ ] 3.1 吸顶 Issue 头（保留 `selected-issue-preview` 契约）+ `StageStepper`（计数+状态色+可点击）。验证：区域名与 testid 兼容测试通过。
- [ ] 3.2 阶段 tab（Story/Design/Work Item 单阶段全宽）+ 默认阶段规则 + Work Item 仓库分组全宽迁移 + 「生成下一阶段」常驻。验证：默认阶段、空阶段不占宽、生成动作可达测试通过。

## 4. 运维面板摘要条（对应 Requirement: 逻辑代码库运维面板降级）

- [ ] 4.1 `LogicalCodebaseSummaryBar`（默认一行摘要、异常警示、展开呈现现有面板、按 Project 记忆）。验证：摘要态/异常态/展开后能力不变测试通过。

## 5. 轮询上下文冻结与视觉规范（对应 Requirement: 轮询刷新保持上下文 / 视觉与交互规范 / 既有契约兼容）

- [ ] 5.1 轮询上下文冻结回归测试（焦点/滚动/折叠/过滤不变）+ 深链 `scrollIntoView`。验证：回归测试通过。
- [ ] 5.2 视觉打磨走查（胶囊 chip、hover 无位移、150–300ms 过渡、`prefers-reduced-motion`、focus 可见、对比度 AA、无 emoji 图标）+ `pnpm test` + `pnpm tsc -b` + `openspec validate --strict` 全绿。验证：命令输出留证。

## Deferred（最终评审裁定，非本 change 验收阻塞）

- 深链 scrollIntoView（「轮询刷新保持上下文」requirement 后半句）：当前深链路径 focusEntityKey 仅打开抽屉、不设置 focusedIssueId，无「焦点行」可滚动，该 SHALL 前置条件未激活；补齐需 focusEntityKey → focusedIssueId 映射与过滤/折叠/截断下的滚动策略，另开 change 处理。
