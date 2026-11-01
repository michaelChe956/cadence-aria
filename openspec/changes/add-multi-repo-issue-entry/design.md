# Design: add-multi-repo-issue-entry

## 决策记录(用户裁决,2026-10-10)

1. plan 形态=**一个多仓 plan**(items 各带 target_repository_id),不做每仓一 plan(用户选 A)
2. 钉定=**上界**:勾选集为硬上界,AI 在界内收敛 involved(可少于勾选),越界走修订反馈(用户确认)
3. MTG 一期红线(人工逐 target 启动)维持,自动 fan-out 维持 defer(C7 既定)
4. 验收=单测/集成+页面 E2E 终验,不跑 provider 矩阵(能力与 provider 无关)

## 架构

```
CreateLifecycleIssueDialog(复选) ──写入──▶ 前后端 DTO(focus_repository_ids) ──▶ create handler 写 explicit selection
                                                    │ focus⊆include 校验(已在)
story(不变) → design ──钉定上界注入 prompt──▶ involved ⊆ 上界(AI 收敛)
              └ generate 入口出生值校验:请求 involved ⊄ 上界 → 拒
                                                    │
plan preflight:恰一仓 → involved ⊆ 上界(involved 空回退恰一仓口径)
                                                    │
SC 事务/split/组编码/cross_target 门:零改动(items.target 分流=现状)
```

## 组件与文件

| 组件 | 文件 | 改动 |
|---|---|---|
| 入口 UI | `web/src/components/lifecycle/CreateLifecycleIssueDialog.tsx`(+测试) | 成员列表复选;提交带 `focus_repository_ids`;勾 1 个=现单选等价 |
| **创建链(k3 P1-3 补)** | `web/src/api/types/common.ts`(CreateProjectIssueRequest)+`src/web/types.rs`(CreateProjectIssueRequest)+`src/web/handlers/…/product_resources.rs`(create_logical_codebase_issue) | 前后端 DTO 增 focus 复数字段;handler 由固定 `all_members` 改为 explicit policy:请求带 focus=写勾选集,不带=维持 all_members(存量兼容) |
| preflight | `src/web/handlers/lifecycle/preflight.rs` + `plan_preparation.rs` | 恰一仓 → `involved ⊆ resolved 上界`;**involved 空时回退「上界恰一仓」判定**(缺陷#7 旧口径等价保留:单成员旧过新过,多成员确定性拒) |
| design 钉定 | `src/product/workspace_engine/prompts.rs`+`entity.rs`/`builder.rs`(e78a97c6 同面) | 钉定块扩为"上界集合+原样输出 involved(可子集)+界外即拒";**上界=resolved 集合**(见关键设计点 0) |
| 出生值守卫(k3 P2-5 补) | generate 入口(`GenerateDesignSpecsRequest` 消费面/types.rs:834-841) | 请求 involved_repository_ids ⊄ resolved 上界 → 拒(出生值面,收敛早于 preflight) |
| 校验守卫 | design write-back 面(aggregate_writeback) | sentinel involved 越界(⊄ resolved 上界)时拒绝回写+可见诊断 |
| spec | SC spec 场景 delta(本 change specs/) | 「多仓必拒」→「界内放行/界外拒」(义务文本保留) |

## 关键设计点

0. **上界=resolved 集合(k3 P1-1 补)**:`focus_repository_ids` 为空(存量 AllMembers selection,现网全部既有 LC issue)时,上界= `resolve_effective_members()`(include−exclude,AllMembers 历史语义)——**绝不以空集为上界**(否则任何 involved 都越界,存量链整体回归);非空时上界=focus 原集。preflight/钉定/出生值/write-back 四面**同源引用同一 resolved bound**。
1. **单仓零回归**:focus=1(或 resolved 单成员)时新逻辑与旧恰一仓等价;**involved 空 → 回退「上界恰一仓」**(k3 P1-2 补:缺陷#7 的单成员回退口径在单成员上界下旧行为通过,多成员上界下确定性拒——矩阵 40+ live 依赖的链不断);现有单仓测试全绿为准绳。
2. **钉定纪律沿用矩阵先例**:e78a97c6 已证钉定消除 AI 自决覆写;本 change 参数化(单仓=上界 1)。
3. **change_order ⊆ involved**:依赖序 AI 定,仅限界内。
4. **不改 MTG/组编码**:items 分流已验收;人工逐 target 启动=REQ-MTG-03 一期合规;**|T|==0 且 focus 多值时启动维持现状 fail-closed(TargetMissing,k3 P3-6 盘点:非一期阻断,不扩展)**。
5. **story 面不动**:story involved 仅线索;闸在 design involved(出生值/preflight/write-back 三环)。

## 错误处理

- 勾选集空/仅 LC 无成员:现表单校验
- involved 空集:回退恰一仓口径(点 1);含界外:出生值拒→write-back 拒→preflight 拒三环收敛
- sentinel 越界回写:拒绝+诊断

## 测试策略

- 单测:preflight **六象限**(界内含子集过/空集回退恰一仓/focus=∅ 走 resolved 上界/界外拒/单成员等价/多成员空 involved 拒)、write-back 越界拒(resolved 同源)、出生值校验、钉定 prompt 内容
- 集成:dialog 复选写入 focus、**create 链 explicit selection durable 校验**、单仓零回归既有面全绿
- 终验:页面 E2E 四仓流程(独立执行)
