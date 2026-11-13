# Tasks：explicit-sc-author-effort

> **本 change 只立项，尚未实施** —— 下列条目全部是**待办清单**（未勾选），不是已完成记录。
> owner 已批准决策三全部推荐（`cadence/designs-reviews/2026-11-08_决策_WIG六项owner裁决_v1.0.md` §3.1–§3.5）。
> 精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`；本文件只记**高层可验收工作包**，每个工作包映射到相关 requirement。
> **需求编号**：REQ-WSC-29（`work-item-plan-single-candidate`，ADDED）+ 「明确 fresh 只换 native 会话」（`workspace-session-resume-recovery`，MODIFIED）+ REQ-ENV-HOST-01（`session-policy-envelope`，MODIFIED）。WSC-26/27 已占用，WSC-28 归 `bound-sc-generation-teaching`，WSC-30/31 归 `isolate-plan-criterion-namespace`。

## 1. 冻结策略的服务端内部承载

映射 requirement：REQ-WSC-29（新的 Claude SC author 原运行冻结 high / 作用域收窄 / 存量保留旧继承模式并披露未知）

- [ ] 1.0 **【防退化 · 新增】「不注入时行为与现状逐字相同」的基线** —— 本 change 的**默认生效面**是「对新的 Claude SingleCandidate author 原运行注入 `CLAUDE_CODE_EFFORT_LEVEL=high`」，它**改变了现有行为**（现状是完全继承宿主）；因此必须钉住**它没有改变的那一半**：作用域外的一切（Story/Design 通用 author、Story/Design 修订、reviewer、coding、非 Claude provider、非 SingleCandidate 出口）**MUST NOT 出现任何 env 变化**，且**无冻结事实**的会话 MUST 与现状完全一致（继续继承宿主、不补造快照）。故须在**未改实现**的状态下记录这些出口的**逐字节 env map 基线**与**无冻结事实会话的继承行为**，改实现后逐条比对。验证：基线可在**未改实现**上复现；改后逐出口比对 env map 逐字节相同，且无冻结事实会话的继承行为不变 —— 任一出口出现额外变量即判为回归（对应 REQ-WSC-29 的「作用域外角色与 provider 档位不变」场景）

- [ ] 1.1 建立服务端内部冻结策略类型（三元组：策略版本、provider、author effort），作为 SC 会话的**可缺省**冻结字段落 durable record；确认它**不进入**公共 provider 配置面，公共快照、wire、TS 快照与 coding 转换链零改动
- [ ] 1.2 冻结策略的**建立规则**：仅由服务端**创建事实**决定新默认（新建 session record 的首次生成且为 SC 流程 + Claude provider ⇒ 冻结 `high`）；非 SC / 非 Claude / Story·Design / reviewer / coding 一律不写
- [ ] 1.3 冻结策略的**读取规则**：读旧 durable record 时缺省即为「无冻结事实」，**不写回、不补造历史快照**；旧 record（含其上的重臂）MUST NOT 被写入当前部署默认
- [ ] 1.4 作用域判别收口到**显式 SC author 出口**：共用 author 构造路径中的 legacy outline / PlanRevision / Draft / Batch **保持空 env**，只有 SC author 出口消费冻结策略；builder 内的基线注入（`prompts/baseline_teaching.rs` 的纯文本边界块）不受影响
- [ ] 1.5 作用域矩阵验收：Claude + SC author ⇒ 命中；非 Claude、legacy、Story/Design、reviewer、coding ⇒ 档位与变更前一致（env 不被注入）
- [ ] 1.6 策略版本不可识别时的行为：零 provider 启动 + typed 错误 + 保留原记录（fail-closed），不得回落当前默认或宿主继承 —— 映射 REQ-WSC-29 的**「冻结策略的 policy_version 不可识别时零启动」**场景

## 2. 冻结策略的注入与同原运行一致性

映射 requirement：REQ-WSC-29（新的 Claude SC 原运行冻结 high / 教学·resume·fresh 三路复用同一冻结策略 / 宿主更高档位不覆盖 / requested 与 effective 不得混称）

- [ ] 2.1 由独立小模块把冻结策略转成 caller env（`CLAUDE_CODE_EFFORT_LEVEL=high`），在 SC author 出口装配 provider 输入时写入；**不改** provider adapter 默认档位，caller env 覆盖层沿用既有透传路径（`gateway_start.rs:648-678` / `provider_gateway.rs:1998-2001`）
- [ ] 2.2 同原运行一致性：初轮、教学重驱、**SC 人工修订出口**（`sc_manual_revision.rs:356-366`）、native resume、明确 fresh **全部**使用同一冻结策略与同一 requested 档位；不得在任一路径重读宿主或当前部署默认
- [ ] 2.3 宿主仍设更高档位时，新运行的显式档位不被抬高或改写
- [ ] 2.4 requested 与 effective 分离：观测与日志记录 requested；effective 无 provider 支持能力 / 请求证据时记「未知」，不得冒称 `high`
- [ ] 2.5 离线 env 装配矩阵（不产生 provider 副作用）：normal / resume / fresh 三路 env 一致；作用域外出口 env 仍为空
- [ ] 2.6 **注入点事实钉死**：注入实现在 **SC author 出口模块**（新增 `prompts/sc_author_input.rs` + `prompts/sc_launch_policy.rs`）；**不是** `claude_code_provider/mod.rs`（adapter 硬编码路径，裁决 §3.3 已否决），**不是** `workspace_engine/prompts.rs:545`（那是共用 builder 的空 env 现状位置，是被分离的对象）

## 3. 恢复、接管与存量兼容

映射 requirement：「明确 fresh 只换 native 会话」（MODIFIED）+ REQ-WSC-29（存量运行保留旧继承模式并披露未知 / 兼容例外 / 客户端不得修改）

- [ ] 3.1 冻结事实在**创建 / 恢复 / 接管**三条路径上逐字段贯穿（durable → runtime → 重建 runtime），逐字段而非整体比较
- [ ] 3.2 **接管继承原策略**：auto→interactive 接管时 MUST **原样复制父会话的冻结字段**，父为 `None` 时保持 `None`，MUST NOT 走「创建时写新默认」的路径，MUST NOT 因新 session ID 换档 —— 映射「明确 fresh 只换 native 会话」的**「自动→交互接管沿用父冻结档位（含缺省）」**场景
- [ ] 3.3 恢复路径使用**服务端原运行**的冻结事实；既有客户端 provider 比较条件**保持不变**，另由服务端核验原运行绑定的策略引用 / 摘要与冻结配置一致，不一致 ⇒ typed 协议错误，不静默按当前默认继续；另映射 REQ-WSC-29 的**「冻结策略的 policy_version 不可识别时零启动」**场景（该场景挂在 REQ-WSC-29，不在恢复 MODIFIED 上）
- [ ] 3.4 **明确 fresh 只换 native 会话**：fresh 后内部执行策略不变、档位不变，且 **MUST NOT 新增授权来源**、**MUST NOT 重置**原运行的授权、预算、预留与授权层 attempt 事实；其真实启动照常按 REQ-WSC-25 记候选尝试流水与 provider_start_ledger 条目（本条的 attempt 指 gate/授权层 attempt，不指候选尝试流水与 provider_start_ledger 条目）
- [ ] 3.5 存量「无冻结事实」：保留旧继承语义 + 以「未知」披露，**不追溯改档**、不补造历史快照；只有下一次**真实新生成**（新建 session record 的首次生成）才用届时的新默认策略 —— 映射「同一 session 重臂仍属原运行」「新建 session 适用新默认」场景
- [ ] 3.6 **兼容例外披露落地**：对外表述 MUST NOT 宣称旧运行的 effective 已被追溯冻结；实际 effective 仍可能受宿主变化影响这一事实进入可披露口径
- [ ] 3.7 客户端在恢复 / 新生成 / 任何控制消息中表达 effort 或其它执行策略时一律忽略，以服务端冻结事实为准；**仅 effort/执行策略表达被忽略，provider 与权限配置变更仍按既有「恢复不换配置或身份」场景拒绝**
- [ ] 3.8 往返与恢复验收（离线）：新记录 roundtrip、进程重启、auto→interactive 接管、部署默认改变四种情形下，同一原运行的显式档位均不变；旧 None 记录只验证「继续不注入平台覆盖、不补造历史快照」
- [ ] 3.9 **冻结点一致性条款**：若创建时写入的 provider 与真实 SC 首次生成准入时的 author_provider 不一致，MUST NOT 静默沿用该档位，MUST 按「无冻结事实」处理（披露未知、不注入平台覆盖），MUST NOT 以创建时值覆盖准入时值 —— 映射 REQ-WSC-29 的**「创建时 provider 与准入时 author_provider 不一致按无冻结事实处理」**场景

## 4. 与主规格 REQ-ENV-HOST-01 的显式修订

映射 requirement：REQ-ENV-HOST-01（`session-policy-envelope`，MODIFIED）

- [ ] 4.1 delta 已新增 `specs/session-policy-envelope/spec.md`，以 `## MODIFIED Requirements` 追加**受信 provider 控制变量例外**（有界类：provider 官方声明的推理档位 / 工具输出上限 / 缓存 TTL 三类），并新增场景「受信 provider 控制变量只在覆盖层内设定」「受信 provider 控制变量清单与三类约束」「输出上限只收紧不放宽」「缓存 TTL 不得短于内置默认」「例外不涉及凭据隔离网络命名空间与宿主路径」「例外不涉及模型账号代理路由与遥测开关」；主规格原文与既有场景逐字保留（仅「运行环境与清理边界」场景做**最小收窄**：加「受信 provider 控制变量除外」括注并指向新场景名）
- [ ] 4.2 归档前核对：`openspec/specs/**` **零改动**（主规格由 delta 归档时自动同步）；例外 MUST NOT 构造或覆盖其余子进程环境，MUST NOT 涉及凭据、隔离、网络命名空间、宿主路径、模型与账号选择、代理与路由、遥测开关；推理档位类 MUST NOT 依模型标签或宿主设置重算，输出上限类 MUST 只收紧，缓存 TTL 类 MUST NOT 短于 provider 内置默认

## 4-a. G-2 / G-3 / G-6：上下文膨胀与缓存 TTL（并入本 change 的 SC author 出口 env 策略）

映射 requirement：REQ-WSC-29（`work-item-plan-single-candidate`，ADDED 的「上下文与缓存控制变量同属冻结策略且只作用于 SC author 出口」「控制变量取值变更只对新生成授权生效」两场景）+ REQ-ENV-HOST-01（`session-policy-envelope`，MODIFIED 的三类约束场景）

- [ ] 4-a.1 在 `prompts/sc_launch_policy.rs` 的冻结策略转 env 处接入**两项输出上限控制变量**：`MAX_MCP_OUTPUT_TOKENS=4000`、`BASH_MAX_OUTPUT_LENGTH=8000`；取值**只**来自该原运行的冻结事实，与 effort 档位同源，**不**在每次启动时重读宿主或平台新默认（验证：初轮 / compile-IR-structure 教学 / native resume / fresh 四路装配出的 env **逐字节一致**）。**G-6 的 `CLAUDE_CODE_PROMPT_CACHE_TTL=1h` 本期不启用**（owner 2026-11-11 裁决）—— 其注入点与冻结语义随本组一并就位，**启用与否**由 G-6 的决策条件（方案 F 落地后 `cache_read`/`cache_creation` 真实计数可观测）触发；本期实现**不得**默认注入该变量
- [ ] 4-a.2 **显式排除**用例（每条一条）：`CLAUDE_CODE_FILE_READ_MAX_OUTPUT_TOKENS` **本期不设**（默认值与截断行为未确认；owner 2026-11-11 裁决让位给 G-5）；**`CLAUDE_CODE_PROMPT_CACHE_TTL` 本期不注入**（G-6 不启用）；`prompts.rs:419`（Story/Design 通用 author）、`prompts/revision.rs:96`（Story/Design 修订）、`prompts/review_parts/single_candidate_input.inc.rs:315`（reviewer）、`coding_models/provider_config.rs:90-106`（coding）**均不设**；`provider_drive/artifact_retry.rs:248-263` 的 retry **继承原出口**已装配的 env，MUST NOT 另行注入或清空（验证：以上各出口的 env 均为空 map 或与原出口逐字节相同）
- [ ] 4-a.3 **拒绝**用例：把输出上限类变量设为**高于** provider 内置默认 → 被拒；把缓存 TTL 类设为**短于** provider 内置默认 → 被拒；把凭据 / 隔离 / 网络命名空间 / 宿主路径 / 模型与账号选择 / 代理与路由 / 遥测开关类变量经该例外设定 → 被拒（验证：每次拒绝都有明确原因，**不静默忽略**）
- [ ] 4-a.4 **取值变更边界**：调整输出上限或缓存 TTL 的取值只对**新的合法生成授权**（新建 session record）生效；在途运行的既有取值 MUST NOT 被改写，MUST NOT 借此重置预算或清空预留（验证：一条在途运行 + 一次取值变更 ⇒ 在途 env 不变；一条新建 session ⇒ 取新值）
- [ ] 4-a.5 **不确定性落纸**（说明项）：TTL 1h 的 cache write 单价高于 5m，且本次 `cache_creation_input_tokens` **恒为 0**（该 provider 未透传 cache write 计数）⇒ **价差无法从本地数据核算，净收益需实测**；G-2/G-3 的字符数（MCP 36,589 → ~16,000；Bash 18,121 → ~8,000）为**基线对照值**，实施后须复测，MUST NOT 写成已测结论

## 5. 跨 change 共享面与排序登记（**说明项 / 协调项** —— 不作为本 change 的可验收产出，不阻归档）

> 本节只登记写面归属与串行次序，**不映射到 requirement 的可验收产出**；实际接线由集成 owner 在各 change 落地时串行执行。

- [ ] 5.1 与 `bound-sc-generation-teaching` 的共享面登记复核：其 SC generation window 的**冻结输入摘要**（**该字段由 bound 的 REQ-WSC-28B / D1 提供，次序 bound → effort**）纳入 effort 策略摘要（provider + 策略版本 + author effort + digest），使教学重驱按原运行的冻结策略执行；**本 change 只负责填值**，不修改 bound 目录的文件
- [ ] 5.2 共享文件的**串行归属**确认（集成 owner 执行，本 change 不并发编辑）：`src/product/models/mod.rs`、`src/product/models/workspace.rs`（SC 字段区）、`src/product/lifecycle_store/workspace.rs`（创建 / 接管）、`src/product/workspace_engine/types.rs`（runtime / 重建）、`src/web/workspace_ws_handler/run/single_candidate.rs`、`src/product/workspace_engine/prompts.rs`、`src/web/workspace_ws_handler/socket.rs`
- [ ] 5.3 与 **isolate** 的共享面登记：`lifecycle_store/workspace.rs` 的创建 / 接管段，次序 **effort → isolate**（isolate 改 `run_history` 创建点与代际，MUST NOT 触碰冻结字段）
- [ ] 5.4 与 **close** 的共享面登记：**无直接共享写面**（close 的写面为 `work_item_plan_policy/classify.rs`、`review/routing*.rs` / `review/closure_policy.rs` 与 REQ-WSC-17 / REQ-TOP-07 delta）
- [ ] 5.5 与**决策六（文件拆分）**的顺序：决策六的纯移动拆分先落，再由集成 owner 依次接 SC author 出口与教学编排；本 change MUST NOT 在拆分前抢先改共用 builder 的调用面；与 mechanicalize / quota-pool **经决策六拆分后**不共享写面
- [ ] 5.6 确认**未修改** `openspec/changes/mechanicalize-plan-compile-repair/**` 与 `openspec/changes/fix-author-rewrite-quota-pool/**`；REQ-WSC-09 的 typed IR / structure 归 `mechanicalize` owner
- [ ] 5.7 确认本期**未改**的既有承载：`prompts.rs:419`（其它 author 出口）、`prompts/review_parts/single_candidate_input.inc.rs:315`（reviewer 出口）、`prompts/revision.rs:96`（Story/Design 修订出口）、`coding_models/provider_config.rs:90-106`（coding 转换）；`provider_drive/artifact_retry.rs:248-263` 的 retry 以 `base_input.clone()` 为基底，**继承原出口**已装配的 env，MUST NOT 另行注入或清空

## 6. 验收与门禁

映射 requirement：REQ-WSC-29（全部）+ 「明确 fresh 只换 native 会话」（MODIFIED）+ REQ-ENV-HOST-01（MODIFIED）

- [ ] 6.1 离线矩阵全绿：作用域矩阵（Claude + SC author 命中，其余不变）、三路 env 一致、宿主高档位不覆盖、往返 / 重启 / 接管 / 部署默认改变四种情形档位不变、旧 None 只验证不注入与不补造
- [ ] 6.2 相关既有测试保持绿（含行数门禁 `large_file_guard`）；公共快照 / wire / TS 快照 / coding 转换链的**零改动**断言成立
- [ ] 6.3 契约留痕核对：delta 与主 specs 措辞一致，REQ-WSC-29、恢复 delta 与 REQ-ENV-HOST-01 delta **均含**「存量 effective 未被追溯冻结」的兼容例外
- [ ] 6.4 回退验收：新生成策略可经受控版本改回原档位 / 旧继承；新旧运行各按原冻结策略结束，**不热切在途 provider、不清预算**；`WorkspaceSessionRecord`（无 `deny_unknown_fields`）新增字段导致旧二进制会静默忽略该字段 ⇒ **旧二进制不得直接回滚**
- [ ] 6.5 变更留痕（**说明项**）：本 change 的 `proposal.md` / `design.md` / `specs/work-item-plan-single-candidate/spec.md` / `specs/workspace-session-resume-recovery/spec.md` / `specs/session-policy-envelope/spec.md` / `tasks.md` 六件齐备且互相对齐
- [ ] 6.6 delta 形态（**说明项**）：**ADDED REQ-WSC-29** + **MODIFIED「明确 fresh 只换 native 会话」**（后者逐字复制主 specs 全文与既有 3 个场景，只增内部执行策略冻结相关正文与场景）+ **MODIFIED REQ-ENV-HOST-01**（逐字复制主 specs 全文与既有 3 个场景，只增受信执行策略变量例外与 1 条场景）
- [ ] 6.7 主 specs 零改动（**说明项**）：MUST NOT 改主 specs（`openspec/specs/**`）—— 归档时由 delta 自动同步

## Workflow follow-up / 需真实 provider 调用（**owner 已授权，范围见 W.1；不阻归档**）

> 本节条目**不阻塞本 change 归档**。**owner 2026-11-11 已授权**真实 provider 调用做质量 / 耗时对照（范围与判据见 W.1）；本节仍**不得预填结论**，实测结果以实测记录为准。

- [ ] W.1 **真实 provider 调用与质量对照（owner 2026-11-11 已授权）**：范围 = **1 个真实多仓需求**，跑 `high` 与**基线档**各 **10 次首次输出**（10/10，不计自动修复），按项目 prompt 验证规则执行。**测的是「high 是否达标」，不是「high 比 low 快多少」** —— `low` 已由裁决 §3.3 否决，测提速量级无决策价值。**判据（owner 2026-11-11 定）**：主判据 = **IR 首次通过率**（基线现值：run1 **15 条 `unknown_done_when_ref`，cross-WI 15/15** ⇒ 首次通过 0；A 包落地后应降到 0 条）+ **真实语义缺陷** + **越权回归**三项均不劣化；**时长与 input/cache 只作观测量，不作判据**。**前置依赖**：`fix-usage-and-timing-metrics` 先行落地（否则 input/cache 与首次通过率**测不准** —— 主方案 §10.2「一切 token 收益当前测不准，方案 F 修完后才能量化」）。实测**不得**写成「44 分钟 → 10–15 分钟」或任何量化提速结论（裁决 §3.2）
- [ ] W.1b **【实测已完成 · 2026-11-09】装置级对照 n=10/10**（同一真实 author prompt / 同一多仓 fixture / 唯一变量 = 档位；报告见 `cadence/analysis-docs/2026-11-09_WIG_effort档位实测对照_v1.0.md`）：**性能方向显著** —— 墙钟中位 415.6s → 249.5s（**-40.0%**，秩和置换检验 p=0.0192，几何均值比 0.533）、output 中位 -22.8%、成本中位 -19.4%（p=0.0242）；**各档去掉最慢 1 次离群后仍全部显著**。**质量方向无差异** —— `unknown_done_when_ref` 越界率按「每次运行」为观测单位，xhigh 中位 5.6% vs high 中位 5.0%，**置换检验 p=0.804**，且**越界与 thinking 深度无关**（rho≈0）、两档逐次分布几乎一致 ⇒ **effort 是性能杠杆，不是质量杠杆**；越界的根治靠方案 A（命名域）。⚠️ 装置级 ≠ 平台全流程，且本次**未启用 codegraph**（fixture trust 门），绝对数字不可与原现场比。⇒ **据此不改默认档位、不改契约**；「是否降到 high」的决策仍须以平台全流程 + 方案 F 计量就绪后的数据为准 |
- [ ] W.2 对照口径细则：先 F 可用再对照；**不以「输出完成」替代计划正确**；跨模型与跨环境的**复现性**须在结论中显式标注为未验证；`low` 是否可成为默认**另行立项**，不在本次授权范围内
