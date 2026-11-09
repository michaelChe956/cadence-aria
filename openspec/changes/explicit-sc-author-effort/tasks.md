# Tasks：explicit-sc-author-effort

> **本 change 只立项，尚未实施** —— 下列条目全部是**待办清单**（未勾选），不是已完成记录。
> owner 已批准决策三全部推荐（`cadence/designs-reviews/2026-11-08_决策_WIG六项owner裁决_v1.0.md` §3.1–§3.5）。
> 精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`；本文件只记**高层可验收工作包**，每个工作包映射到相关 requirement。
> **需求编号**：REQ-WSC-29（`work-item-plan-single-candidate`，ADDED）+ 「明确 fresh 只换 native 会话」（`workspace-session-resume-recovery`，MODIFIED）。WSC-26/27 已占用，WSC-28 归 `bound-sc-generation-teaching`，WSC-30/31 归 `isolate-plan-criterion-namespace`。

## 1. 冻结策略的服务端内部承载

映射 requirement：REQ-WSC-29（新的 Claude SC 原运行冻结 high / 作用域收窄 / 存量保留旧继承模式并披露未知）

- [ ] 1.1 建立服务端内部冻结策略类型（三元组：策略版本、provider、author effort），作为 SC 会话的**可缺省**冻结字段落 durable record；确认它**不进入**公共 provider 配置面，公共快照、wire、TS 快照与 coding 转换链零改动
- [ ] 1.2 冻结策略的**建立规则**：仅由服务端创建事实决定新默认（SC 流程 + Claude provider ⇒ 冻结 `high`）；非 SC / 非 Claude / Story·Design / reviewer / coding 一律不写
- [ ] 1.3 冻结策略的**读取规则**：读旧 durable record 时缺省即为「无冻结事实」，**不写回、不补造历史快照**；序列化保持旧记录可反序列化
- [ ] 1.4 作用域判别收口到**显式 SC author 出口**：共用 author 构造路径中的 legacy outline / PlanRevision / Draft / Batch / 基线教学**保持空 env**，只有 SC author 出口消费冻结策略
- [ ] 1.5 作用域矩阵验收：Claude + SC author ⇒ 命中；非 Claude、legacy、Story/Design、reviewer、coding ⇒ 档位与变更前一致（env 不被注入）
- [ ] 1.6 策略版本不可识别时的行为：零 provider 启动 + typed 错误 + 保留原记录（fail-closed），不得回落当前默认或宿主继承

## 2. 冻结策略的注入与同原运行一致性

映射 requirement：REQ-WSC-29（新的 Claude SC 原运行冻结 high / 教学·resume·fresh 三路复用同一冻结策略 / 宿主更高档位不覆盖 / requested 与 effective 不得混称）

- [ ] 2.1 由独立小模块把冻结策略转成 caller env（`CLAUDE_CODE_EFFORT_LEVEL=high`），在 SC author 出口装配 provider 输入时写入；**不改** provider adapter 默认档位，caller env 覆盖层沿用既有透传路径
- [ ] 2.2 同原运行三路一致性：初轮、教学重驱、人工修订、native resume、明确 fresh **全部**使用同一冻结策略与同一 requested 档位；不得在任一路径重读宿主或当前部署默认
- [ ] 2.3 宿主仍设更高档位时，新运行的显式档位不被抬高或改写
- [ ] 2.4 requested 与 effective 分离：观测与日志记录 requested；effective 无 provider 支持能力 / 请求证据时记「未知」，不得冒称 `high`
- [ ] 2.5 离线 env 装配矩阵（不产生 provider 副作用）：normal / resume / fresh 三路 env 一致；作用域外出口 env 仍为空

## 3. 恢复、接管与存量兼容

映射 requirement：「明确 fresh 只换 native 会话」（MODIFIED）+ REQ-WSC-29（存量运行保留旧继承模式并披露未知 / 兼容例外 / 客户端不得修改）

- [ ] 3.1 冻结事实在**创建 / 恢复 / 接管**三条路径上逐字段贯穿（durable → runtime → 重建 runtime），逐字段而非整体比较
- [ ] 3.2 **接管继承原策略**：auto→interactive 接管时冻结策略随上下文复制，MUST NOT 因新 session ID 换档
- [ ] 3.3 恢复路径使用**服务端原运行**的冻结事实；既有客户端 provider 比较条件**保持不变**，另由服务端核验原运行绑定的策略引用 / 摘要与冻结配置一致，不一致 ⇒ typed 协议错误，不静默按当前默认继续
- [ ] 3.4 **明确 fresh 只换 native 会话**：fresh 后内部执行策略不变、档位不变，且**不重置**原运行的授权、预算、预留与 attempt 事实
- [ ] 3.5 存量「无冻结事实」：保留旧继承语义 + 以「未知」披露，**不追溯改档**、不补造历史快照；只有下一次真实新生成才用届时的新默认策略
- [ ] 3.6 **兼容例外披露落地**：对外表述 MUST NOT 宣称旧运行的 effective 已被追溯冻结；实际 effective 仍可能受宿主变化影响这一事实进入可披露口径
- [ ] 3.7 客户端在恢复 / 新生成 / 任何控制消息中表达 effort 时一律忽略，以服务端冻结事实为准；不得据此新建或改写原运行的冻结事实
- [ ] 3.8 往返与恢复验收（离线）：新记录 roundtrip、进程重启、auto→interactive 接管、部署默认改变四种情形下，同一原运行的显式档位均不变；旧 None 记录只验证「继续不注入平台覆盖、不补造历史快照」

## 4. 跨 change 共享面与排序登记

映射 requirement：REQ-WSC-29（同一原运行的冻结策略不得被当前默认覆盖）

- [ ] 4.1 与 `bound-sc-generation-teaching` 的共享面登记复核：其 SC generation window 的**冻结输入摘要**纳入 effort 策略摘要（provider + 策略版本 + author effort + digest），使教学重驱按原运行的冻结策略执行；该摘要的构造在其 change 目录内，**由集成 owner 统一接上**，本 change 不修改其文件
- [ ] 4.2 共享文件的**串行归属**确认（集成 owner 执行，本 change 不并发编辑）：`src/product/models/mod.rs`、`src/product/models/workspace.rs`（SC 字段区）、`src/product/lifecycle_store/workspace.rs`（创建 / 接管）、`src/product/workspace_engine/types.rs`（runtime / 重建）、`src/web/workspace_ws_handler/run/single_candidate.rs`、`src/product/workspace_engine/prompts.rs`、`src/web/workspace_ws_handler/socket.rs`
- [ ] 4.3 与**决策六（文件拆分）**的顺序：决策六的纯移动拆分先落，再由集成 owner 依次接 SC author 出口与教学编排；本 change MUST NOT 在拆分前抢先改共用 builder 的调用面
- [ ] 4.4 确认**未修改** `openspec/changes/mechanicalize-plan-compile-repair/**` 与 `openspec/changes/fix-author-rewrite-quota-pool/**`；REQ-WSC-09 的 typed IR / structure 归 `mechanicalize` owner
- [ ] 4.5 确认本期**未改**的三处既有承载：`prompts.rs:419`（其它 author 出口）、`prompts/review_parts/single_candidate_input.inc.rs:315`（reviewer 出口）、`coding_models/provider_config.rs:90-106`（coding 转换）

## 5. 验收与门禁

映射 requirement：REQ-WSC-29（全部）+ 「明确 fresh 只换 native 会话」（MODIFIED）

- [ ] 5.1 离线矩阵全绿：作用域矩阵（Claude + SC author 命中，其余不变）、三路 env 一致、宿主高档位不覆盖、往返 / 重启 / 接管 / 部署默认改变四种情形档位不变、旧 None 只验证不注入与不补造
- [ ] 5.2 相关既有测试保持绿（含行数门禁 `large_file_guard`）；公共快照 / wire / TS 快照 / coding 转换链的**零改动**断言成立
- [ ] 5.3 契约留痕核对：delta 与主 specs 措辞一致，REQ-WSC-29 与恢复 delta 的条款**均含**「存量 effective 未被追溯冻结」的兼容例外
- [ ] 5.4 **真实 provider 调用与质量 / 耗时对照：获 owner 另行授权后执行**（当前**未授权**，本期不执行、不得预填结论）；获授权后按项目 prompt 验证规则，涉及 Draft / Canonical 输出约束时执行获授权 Case A/B 各 10 次首次输出 10/10，不计自动修复
- [ ] 5.5 获授权后的对照口径：先 F 可用再对照时长、input/cache、IR 首次通过、真实语义缺陷与越权回归；**不以「输出完成」替代计划正确**；`low` 是否可成为默认另行立项，不在本次预批准

## 6. 契约与留痕

- [ ] 6.1 本 change 的 `proposal.md` / `design.md` / `specs/work-item-plan-single-candidate/spec.md` / `specs/workspace-session-resume-recovery/spec.md` / `tasks.md` 五件齐备且互相对齐
- [ ] 6.2 delta 为 **ADDED REQ-WSC-29** + **MODIFIED「明确 fresh 只换 native 会话」**（后者逐字复制主 specs 全文与既有 3 个场景，只增内部执行策略冻结相关正文与场景）
- [ ] 6.3 MUST NOT 改主 specs（`openspec/specs/**`）—— 归档时由 delta 自动同步
- [ ] 6.4 留痕：决策三 owner 裁决（2026-11-08）与本 change 的「不扩公共 DTO / 存量 effective 未冻结的兼容例外 / 真实调用需另行授权」三处边界记入交付说明
