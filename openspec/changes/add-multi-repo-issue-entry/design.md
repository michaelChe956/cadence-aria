# Design: add-multi-repo-issue-entry

## 决策记录(用户裁决,2026-10-10)

1. plan 形态=**一个多仓 plan**(items 各带 target_repository_id),不做每仓一 plan(用户选 A)
2. 钉定=**上界**:勾选集为硬上界,AI 在界内收敛 involved(可少于勾选),越界走修订反馈(用户确认)
3. MTG 一期红线(人工逐 target 启动)维持,自动 fan-out 维持 defer(C7 既定)
4. 验收=单测/集成+页面 E2E 终验,不跑 provider 矩阵(能力与 provider 无关)

## 架构

```
CreateLifecycleIssueDialog(复选) ──写入──▶ selection.focus_repository_ids(后端字段已在)
                                                    │ focus⊆include 校验(已在)
story(不变) → design ──钉定上界注入 prompt──▶ involved ⊆ focus(AI 收敛)
                                                    │
plan preflight:恰好一仓 → involved ⊆ focus 且非空(本 change 核心改动)
                                                    │
SC 事务/split/组编码/cross_target 门:零改动(items.target 分流=现状)
```

## 组件与文件

| 组件 | 文件 | 改动 |
|---|---|---|
| 入口 UI | `web/src/components/lifecycle/CreateLifecycleIssueDialog.tsx`(+测试) | 成员列表复选;提交写 `focus_repository_ids`;勾 1 个=现单选等价 |
| preflight | `src/web/handlers/lifecycle/preflight.rs` + `plan_preparation.rs` | `preflight_single_repository_candidate` 的恰一仓判定 → `involved ⊆ focus 且非空`(读 selection);单成员时语义等价 |
| design 钉定 | `src/product/workspace_engine/prompts.rs`+`entity.rs`/`builder.rs`(e78a97c6 同面) | 钉定块从"单仓绝对路径"扩为"勾选集上界+原样输出 involved(可子集)+界外即拒" |
| 校验守卫 | design write-back 面(aggregate_writeback) | sentinel involved 越界(⊄focus)时拒绝回写+可见诊断(单仓钉定的既有纪律推广) |
| spec | SC spec 场景 delta(本 change specs/) | 「多仓必拒」→「界内放行/界外拒」 |

## 关键设计点

1. **单仓零回归**:focus=1 时 preflight 新逻辑与旧恰一仓等价(数学上 `involved ⊆ {x} 且非空` ⇔ `involved == {x}`),现有单仓测试(含矩阵 40+ live 的单仓链)不受影响;以既有测试全绿为准绳。
2. **钉定纪律沿用矩阵先例**:e78a97c6 已证"钉定彻底消除 AI 自决覆写";本 change 把同一纪律参数化(单仓=上界 1)。
3. **change_order ⊆ involved**:依赖序仍由 AI 定,但仅限界内成员。
4. **不改 MTG/组编码**:items 的 target_repository_id 分流是已验收现状;人工逐 target 启动=REQ-MTG-03 一期合规。

## 错误处理

- 勾选集空/仅 LC 无成员:沿用现表单校验
- involved 空集/含界外:deterministic preflight 失败(SC 终态收敛,含原因)——与现单仓拒绝形态同构
- sentinel 越界回写:拒绝+诊断(钉定纪律)

## 测试策略

- 单测:preflight 新语义(⊆/非空/界外/单成员等价四象限)、write-back 越界拒、钉定 prompt 内容
- 集成:dialog 复选写入 focus(lcg 系)、单仓零回归既有面全绿
- 终验:页面 E2E 四仓流程(独立执行,不在本 change tasks 内)
