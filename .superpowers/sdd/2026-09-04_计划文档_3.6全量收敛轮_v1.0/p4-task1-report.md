# Phase 4 Task 1：P3 留档四项顺修报告

## 完成内容

- `RevisionDiffView`：单 artifact 轮次直接返回，避免冗余 markdown GET；版本号使用组件级 in-flight 集合去重，并记录成功读取版本，避免 sibling cache 更新引起 effect 重跑后重复读取。
- `PlanRepairReadOnlyPanel`：`resumeTarget === null` 时显示 `—`，不保留空 `<dd>`。
- `ContractChecklistView`：走查真实 `review_verdict.metadata.findings[]` 后确认该结构不包含 `contract_field`；无跳转目标时降级为“无关联 finding（以门禁 finding 为准）”。
- `selectFindingTargetEntryId`：新增 review verdict 缺 `contract_field` 的反例，证明返回 `null`，不凭 severity、message 等字段虚构跳转目标。

## 红灯证据

命令：

```sh
cd web && pnpm test src/components/chat-workspace/cockpit/RevisionDiffView.test.tsx src/components/chat-workspace/cockpit/PlanRepairReadOnlyPanel.test.tsx src/components/chat-workspace/cockpit/ContractChecklistView.test.tsx
```

结果：预期失败。单轮仍调用 `load`；sibling cache 更新后 v2 重复读取；空 `resumeTarget` 渲染为空；旧 L2 文案未按真实 review finding 的字段边界降级。

## 绿灯证据

定向命令：

```sh
cd web && pnpm test src/components/chat-workspace/cockpit/RevisionDiffView.test.tsx src/components/chat-workspace/cockpit/PlanRepairReadOnlyPanel.test.tsx src/components/chat-workspace/cockpit/ContractChecklistView.test.tsx src/state/plan-approval-projection.test.ts
```

结果：4 个测试文件、32 个测试通过。

全量命令：

```sh
cd web && pnpm tsc -b && pnpm test
```

结果：类型检查通过；156 个测试文件、1334 个测试通过。测试运行仍输出既有 jsdom navigation 未实现 stderr，命令退出码为 0。

边界核查：`git diff --stat -- src/` 无输出；`git diff -- web/package.json` 无输出。

## Commit 列表

`633090d5 fix: 顺修 Phase3 留档请求与呈现边界`

## 自我审查发现

- `ChatCockpitPage` 以 `key={selectedSessionId}` 重挂载 `PlanApprovalPanel`，因此已成功读取版本的组件内集合不会跨会话抑制新会话的读取；同一会话内 retained 集合仅消除 effect 重跑造成的重复成功 GET。
- 未修改引擎、前端依赖或 ImageCreate 页面。

## Fix round 1：F.4 字段存在分支回退

- 已按引擎 wire 契约与既有 projection 用例确认 `review_verdict.metadata.findings[].contract_field` 为可选字段；恢复 `ContractChecklistView` 的两段式原文案“无关联 finding（当前对话流中没有指向该契约的门禁 / 评审 finding）”。
- 恢复 `ContractChecklistView.test.tsx` 的 `getByText("无关联 finding")` 既有断言，删除错误分支新增的同值降级文案用例；保留 `plan-approval-projection.test.ts` 对缺少 `contract_field` 时不得虚构跳转目标的反例。
- 定向：`cd web && pnpm vitest --run src/components/chat-workspace/cockpit/ContractChecklistView.test.tsx src/state/plan-approval-projection.test.ts`，2 个测试文件、17 个测试通过。
- 全量：`cd web && pnpm tsc -b && pnpm test`，类型检查通过；156 个测试文件、1333 个测试通过。测试 stderr 仍有既有 jsdom navigation 未实现输出，命令退出码为 0。
- `loadedVersionsRef` 不能删除：定向移除后，`RevisionDiffView` single-flight 回归测试出现第三次请求；成功写入 `localCache` 与父级缓存的 effect 时序之间仍可触发一次 `missingKey` 重跑，该集合避免重复 GET。
- Commit：`fix: 回退 F.4 降级文案分支至字段存在分支`。