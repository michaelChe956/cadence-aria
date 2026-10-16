# Task 5 报告:Bug2-C 类型投影与前端回归

- 状态: DONE(批 1 代码任务全部完成,停等审查,过后 Task 6 全量门禁+部署)
- Commit: 见 git log 消息 `feat(web): CodingRoleRun origin 类型与关门投影一致性(Bug2-C)`(以该消息最新一条为准)
- 测试摘要: 红=tsc 类型缺口实录(TS2353/TS2339 `gate_origin does not exist in type 'CodingRoleRun'`)→ 绿:`pnpm exec vitest run useCodingWorkspaceWs coding-attention` 3 文件 36 用例全过;全量 `pnpm test` **230 文件 / 2361 用例全过**;`pnpm build`(tsc -b + vite)成功。

## 交付内容

1. **类型对齐**(web/src/api/types/coding.ts):新增 `CodingRoleRunGateOrigin{gate_id, generation, action_id}`;`CodingRoleRun.gate_origin?: CodingRoleRunGateOrigin | null`——镜像 Task 3 Rust `GateOrigin` serde 可选字段(旧数据缺省 null/缺失,按无关联解释不猜);generation 对应服务端 `resolution_generation`,重开递增后旧 origin 代次失配不误关新门。
2. **前端回归锁**(useCodingWorkspaceWs.actions.test.tsx 新 describe `coding-attention projection consistency after gate closure`,命中 brief 的 vitest 过滤词):
   - **snapshot 移除**:动作关闭成功→权威快照 pending_gates=[]→pendingGates 由快照重建为空;同快照 retry run 携带 gate_origin 原样透传(关联事实不丢失);
   - **重连不复活**:断连重放窗口(动作已发、确认快照未达即 close 1006)→重连 hello→服务端(Task 3/4 关门幂等收敛)重放权威快照→门不复活、origin run 在列(fake timers 全链路模拟);
   - **事项一致**(对照 coding_attention_info.rs:317-397:事项只由 open 门事实派生):门归档后随快照重建自然消失,UI 无特例按钮逻辑;旧 run 缺 origin 投影不变(`gate_origin ?? null` 为 null)。
3. **UI 无特例按钮逻辑**:确认无需任何按钮级隐藏特例——前端 pendingGates 恒等于最新服务端快照(mergeSnapshotPendingGates 只映射快照门,不做本地并集),Task 3/4 的后端持久化事实经快照自然投影。

## TDD 红→绿实录

- Step 1 先写三用例(vitest 运行时 16/16 绿——store 快照重建语义本就正确,这正是后端权威修复下的预期;这些用例是回归锁);**类型红以 `pnpm exec tsc -b` 实录**:TS2353(字面量含 gate_origin)/TS2339(读 gate_origin)两处报错。
- Step 2 实现 `CodingRoleRunGateOrigin`+可选字段后:tsc 0 错;brief 命令 `pnpm exec vitest run useCodingWorkspaceWs coding-attention` 36 用例全过(遵 Main 嘱用 pnpm exec,非 npm/npx)。

## 回归

- 全量前端:`pnpm test` 230 文件/2361 用例 0 败;`pnpm build` 成功(tsc -b 无错,vite 产物正常)。
- 后端零改动(本任务只动 coding.ts+actions.test.tsx 两文件,均在文件面内)。

## Concerns

1. vitest 默认不 typecheck(esbuild 剥离类型),类型门的日常保障是 `pnpm build` 的 tsc -b——本次已实跑通过;若未来把 `vitest --typecheck` 纳入门禁,新 describe 可直接复用。
2. roleRuns 中 gate_origin 仅作关联事实透传,前端无按钮逻辑消费它(消费方是 Task 4 服务端补偿);RoleRunHistoryPanel 等展示组件不特例渲染,符合「无特例按钮逻辑」约束。

## 修复轮 1(F1,P2,审查必修)

**问题**:`upsertRoleRunFromCompletedProviderEntry`(coding-workspace-store.ts:843-884)在 live provider 完成帧按字段逐项重建 CodingRoleRun 时,复制了 retry_metadata/retry_exhausted/supersede_*/reason_code 全部可选字段,唯缺 `gate_origin`——权威快照已携带的关联事实在该窗口丢失(store 中变 undefined),直到下一次权威快照。

**修复**(按 reviewer 指定,:875 补一行,与既有可选字段保留约定同形):
```ts
gate_origin: existing?.gate_origin ?? null,
```

**覆盖测试**(新增 `keeps gate origin when a completed provider entry rebuilds the role run`,TDD 红→绿):
- 红:run 带含 gate_origin 的权威快照入 store → provider 完成帧(`replacePendingEntry` provider_stream+role_run_id)重建 → 断言 gate_origin 保留 → 实录 1 failed(丢失为 undefined);
- 绿:修复后 run 转 completed 且 gate_origin 三元组原样保留。

**命令与输出**:
- `pnpm exec vitest run useCodingWorkspaceWs coding-workspace-store` → 4 文件 63 用例全过
- `pnpm exec tsc -b` → 0 错
- `pnpm test` 全量 → **230 文件 / 2362 用例全过**(较修复前 +1 用例)

**Commit**:单独提交 `fix(web): live 完成重建保留 gate_origin 投影`。
