# pi-light rep1e 探索跑证据（不入格）——2026-09-07 补验定性

> 定性（oracle+controller 共识，2026-09-07 晚）：**levels 语料 legacy_group 通道全链证据
> （advance 轴缺失，admission_kind=legacy_group，产品路径不同，不可补发）**。
> 原交接文档称之为「pi×轻 rep1」的前提已被 digest 级证据推翻，本跑**不计入任何矩阵格**。

## 四轴失配（vs 3.5 canonical 轻格基准=val-summary.json）

| 轴 | 3.5 canonical（val-pi 10/10） | rep1e 实测 | 证据 |
|---|---|---|---|
| 语料 | minimal（08-minimal-hello-api.md，digest `ca554062…`） | levels（07-fullstack-levels.md，digest `116d4a15…`） | `run-artifacts/pi/rep1/result.json` description_digest |
| advance 脚本 | `request-change:…;confirm;advance`（每跑 send+completed） | `ARIA_HUMAN_SCRIPT='confirm'`（advance_actions=[]，ws.jsonl 0 次 advance） | result.json advance_actions + ws.jsonl |
| admission_kind | sc_advance（12/12） | **legacy_group**（coding driver 自建 attempt 走 group.rs 默认） | `durable-copies/issue_work_item_plan_0001.json` |
| 单元数 | 1 unit | 3 units | handoff.json work_item_ids |

## 产品路径差异（为何不可补发 advance）

- 未发 advance → 无 sc_advance journal → coding driver `POST .../work-item-plans/{plan}/coding-attempts` 走 `group.rs` 新建分支 → `prepare_group_initialization` 默认 `LegacyGroup`（`group_initialization.rs:110`）
- `group_dependency_gate.rs:22-24`：依赖门**仅对 ScAdvance 生效**；`group_initialization.rs:124`：单元拓扑排序**仅对 ScAdvance 执行**——rep1e 的 3 单元在无依赖门/无拓扑排序的 legacy 通道跑完
- `advance.rs:420`：存在非 ScAdvance group journal 时 advance 直接 `Err("existing group initialization is bound to another advance identity")`——issue_0140 该计划**永久无法再 advance**

## 真实证据价值（显式入档）

1. **F5 有效性正面证据**：flash（glm-5.3-flash）在 levels 语料 27.6min/1 auto repair/2 轮 review 达 Confirmed——直接推翻 3.5「pi×levels hard_timeout×2」记录，是 pi×重 格可行性的实测依据
2. **多单元 legacy_group 通道全链首通**：3 units → coding completed → push（head=c791225b，naruto 远端实测一致）；非 C1 判据通道，供 legacy 通道回归参考
3. **7 次 coding 尝试失败谱系**：上游 503×2+预算不足×2+blocked 语义×2+driver 撞存活 runner×1（后者已修=commit 4e466030 codingProtocolErrorPlan 容忍）
4. push 层闭环物证：`durable-copies/git-operation.json`（kind=review_request, phase=completed, push_status=pushed, commit_sha=c791225b…）

## 根因与防复发

- 根因=交接文档 §4 SOP 命令缺 `ARIA_FIXTURE_SET`（默认 levels）与 `ARIA_HUMAN_SCRIPT`（interactive 必填，未设时退化裸 confirm），两处偏离均不报错——**操作漏配 env，非 driver/产品缺陷，不进 F7 族**（交接 §5.1 的「疑 driver 缺陷」结论已改）
- SOP 已修正：补三个必需 env（ARIA_FIXTURE_SET/ARIA_HUMAN_SCRIPT/ARIA_WORKITEM_HARD_TIMEOUT_MS）
