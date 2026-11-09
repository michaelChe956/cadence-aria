# 提案：Claude SC author 显式 effort 冻结策略

> 状态：**草案 —— owner 已批准决策三全部推荐**（见 `cadence/designs-reviews/2026-11-08_决策_WIG六项owner裁决_v1.0.md` §3.1–§3.5）。**本 change 只立项，不实施**：`tasks.md` 是待办清单，不是已完成记录。
> 依据：决策报告 §3（effort 默认 high、独立内部承载、恢复冻结）、主方案 `cadence/designs/2026-11-08_技术方案_WIG生成加速与收敛性加固_v1.0.md` 的 G 工作包。
> 需求编号：本 change 占用 **REQ-WSC-29**（WSC-26/27 已被归档 change `2026-11-08-gate-card-resolved-and-inflight-state` 占用；WSC-28 归 `bound-sc-generation-teaching`；WSC-30/31 归 `isolate-plan-criterion-namespace`），并 **MODIFIED `session-policy-envelope` 的 REQ-ENV-HOST-01**（新增受信执行策略变量例外，见「范围」）。

## Why

WIG（Work Item Generation）链路的 Claude SingleCandidate author 目前**没有显式推理档位**：SC/plan 流式输入构造器给 `env_vars` 传空 map（`src/product/workspace_engine/prompts.rs:545` 附近），子进程因此**继承宿主环境**，实际 effort 随宿主的 Claude 设置漂移（实测宿主为 `xhigh`）。这带来三个真实缺陷：

1. **行为不可归因**：同一仓库、同一提示，实际档位随机器/宿主 settings 变化，无法判断质量与耗时差异来自哪一侧，也无法在回归中复现。
2. **恢复语义未定义**：SC author 的 native 会话在指纹漂移后走「明确 fresh 只换 native 会话」路径；现有恢复比较（`src/web/workspace_ws_handler/socket.rs:1413-1427`）逐字段比较 author / reviewer / permission_modes，**不含 effort**。若未来把 effort 引入可写路径，部署升级或显式 fresh 都可能让同一原运行换档，而现行恢复合同禁止「恢复换 provider 配置」。
3. **无默认口径**：官方文档说明 `CLAUDE_CODE_EFFORT_LEVEL` 优先于会话选择、CLI flag 与 settings，但组织 cap 仍可能压低实际生效值；平台当前既不设默认、也不记录 requested 与 effective 的区别。

因此本期需要**一条被规格钉死的执行策略**：新的 Claude SC author 原运行由平台**冻结**显式 effort=high，同一原运行的教学、resume、明确 fresh 复用**同一冻结策略**；无冻结事实的存量运行保留旧继承语义并**披露未知**，不伪造原档位。

## What Changes

- **新增 SC author 执行策略契约**：平台 SHALL 为新的 Claude SingleCandidate author 原运行冻结显式 effort=high；同一原运行的教学重驱、native resume 与明确 fresh SHALL 使用相同冻结策略，MUST NOT 从宿主新 settings 或当前部署默认重算。
- **新增服务端内部冻结承载**：新增独立的服务端内部配置（`policy_version` / `provider` / `author_effort` 三元组），作为 SC 运行的可选冻结字段随原业务运行耐久保存；**不扩展公共 `ProviderConfigSnapshot`**（该 DTO 承载在 `StartGeneration.provider_config` 上，扩它会命中 Rust wire、两份 TS 快照、coding 转换、session 持久与恢复比较的公共配置面）。
- **冻结事实逐字段贯穿创建 / 恢复 / 接管**：新运行由**服务端创建事实**决定新默认；读取旧 durable record **不写入**任何补造值；自动→交互接管**继承原策略**，MUST NOT 以新 session ID 换档。
- **恢复合同扩展**：修改「明确 fresh 只换 native 会话」条款 —— fresh 只换 native 会话，**内部执行策略亦按原运行冻结**（不换 effort、不重置授权/预算）；存量「无冻结事实」的运行**不追溯改档**，下一次**真实新生成**才使用新默认。
- **兼容例外显式披露**：存量继承模式的**实际 effective effort 仍可能受宿主变化影响**；MUST NOT 宣称旧运行的 effective 已被追溯冻结。缺冻结事实时按「未知」披露。
- **作用域收窄**：只作用于 Claude + SingleCandidate author。reviewer、Story/Design、coding 及其它 provider 的既有档位**保持不变**（本期明确不改 `prompts.rs:419` 的其他 author 出口、`review_parts/single_candidate_input.inc.rs:315` 的 reviewer 出口、`coding_models/provider_config.rs:90-106` 的 coding 转换）。
- **requested 与 effective 分离**：日志与观测记录 requested 档位；MUST NOT 把组织 cap 压低后的 effective 冒称 high。
- **真实 provider 调用另行授权**：本期**未授权**任何真实 provider 调用或 effort 实验（Case A/B）；相关验收项只写「获授权后执行」。

## 非目标

- **不扩展公共 `ProviderConfigSnapshot` / 不新增客户端可写参数**：本期不做可编辑的 effort/model 配置面，不动 Rust wire 与两份 TS 快照。
- **不在 provider adapter 内硬编码 effort**：adapter 层保持角色无关，避免影响 reviewer、coding 与所有 Claude 使用者。
- **不改变 reviewer / Story / Design / coding / 其它 provider 的档位**：作用域限定在 Claude SC author。
- **不把 low 预设为默认、不承诺提速**：high 的性能收益尚无实测；low 是否可成为默认、high 实际提速与跨环境质量均需另行实测与授权（决策报告 §3.3 / §9）。
- **不改恢复比较中的其它遗漏项**：例如当前 fresh 比较未包含 `review_rounds`，本期不顺手修。
- **不做全载体治理**：coding/review/legacy 所有物理窗口治理、其它恢复配置字段的公共化，均不属本期。
- **不伪造历史事实**：MUST NOT 为存量运行补写 effort 快照、伪造历史档位或据此宣称可追溯冻结。

## 范围

| 能力 | 变更 |
|---|---|
| `work-item-plan-single-candidate` | **ADDED** REQ-WSC-29（SC author 显式 effort 冻结策略：默认 high、作用域收窄、冻结贯穿教学/resume/fresh、存量未知披露、requested/effective 分离；含「原运行 vs 真实新生成」界定与 policy_version 不可识别时的零启动 fail-closed） |
| `workspace-session-resume-recovery` | **MODIFIED**「明确 fresh 只换 native 会话」（内部执行策略亦按原运行冻结；fresh 不换 effort、不重置授权/预算；存量无冻结事实不追溯改档，下一次真实新生成才用新默认；兼容例外写入） |
| `session-policy-envelope` | **MODIFIED** REQ-ENV-HOST-01（追加**受信执行策略变量例外**：provider 官方声明的单一档位变量 MAY 在**调用方显式 env map 覆盖层**内设定为受控值；其余环境、凭据、隔离与网络命名空间不受影响。主规格原文与既有场景逐字保留） |

## 注入点事实（供主方案对齐）

- 注入点是 **SC author 出口模块**：新增 `prompts/sc_author_input.rs` + `prompts/sc_launch_policy.rs`（冻结策略 → caller env `CLAUDE_CODE_EFFORT_LEVEL`）。
- **不是** `claude_code_provider/mod.rs`（adapter 硬编码路径已被裁决 §3.3 否决），**不是** `workspace_engine/prompts.rs:545`（那是共用 builder 的空 env **现状位置**，是本 change 要被分离的对象，不是注入实现位置）。

## OpenSpec 阈值判定

**命中**（决策报告 §3.4 明文）。即使不扩展 DTO，默认推理档位、内部 durable 字段与恢复行为都是**新行为 / 新数据契约**，MUST NOT 当作一行 env 小修绕过规格门禁。

## 风险与取舍

| 风险 | 处置 |
|---|---|
| `high` 的质量收益与耗时收益**均未实测** | 本期只冻结口径，不承诺时长/token 降幅；真实质量与耗时对照需另行授权后实测（决策报告 §3.2 / §3.5） |
| 组织 cap 可能把实际生效档位压到 `high` 之下 | requested 与 effective 分离记录；MUST NOT 把 effective 冒称 high |
| 存量运行无法证明原 effective 档位 | 保留旧继承语义并披露未知；MUST NOT 追溯冻结、不得补造历史快照（兼容例外显式写入 delta） |
| 未来把 effort 做成可编辑配置时需要重新设计 | 本期只做服务端内部冻结字段；公共配置面留待另立完整设计（公共快照/TS/durable/resume） |
| 显式冻结可能与宿主/组织策略冲突 | 平台层只设 requested 档位，不承诺绕开组织 cap |

## 跨 change 登记

- 与 `bound-sc-generation-teaching` 共享面（次序 **bound → effort**）：SC 生成教学链的**运行绑定摘要**需把冻结策略纳入冻结输入，MUST NOT 用当前默认覆盖旧绑定 —— **该冻结输入摘要字段由 bound 的 REQ-WSC-28B / D1 提供，本 change 只负责填值**；完整共享面与串行归属见本 change `design.md` 的「共享面与排序」。
- 与 `isolate-plan-criterion-namespace` 共享面：仅 `lifecycle_store/workspace.rs` 的**创建 / 接管段**（次序 **effort → isolate**；isolate 改 `run_history` 创建点与代际）。该 change 另占用 REQ-WSC-30/31 与 `work-item-typed-outcome-policy`。
- 与 `close-contract-findings-before-pass` **无直接共享写面**（close 的写面为 `work_item_plan_policy/classify.rs`、`review/routing*.rs` / `review/closure_policy.rs` 与 REQ-WSC-17 / REQ-TOP-07 delta）。
- 与 `mechanicalize-plan-compile-repair` / `fix-author-rewrite-quota-pool` **经决策六拆分后**不共享写面（mechanicalize 的代码面含 `run/single_candidate.rs`，须先落决策六的纯移动拆分）；本 change MUST NOT 修改这两个 change 目录的任何文件。
