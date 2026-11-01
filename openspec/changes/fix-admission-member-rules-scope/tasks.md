# Tasks: fix-admission-member-rules-scope

全部工作包映射到 `logical-codebase-registration` 的 REQ-REG-12 delta；supersede、非目标及规格时序见 `proposal.md`，三路与禁止事项见 `design.md`。此清单仅记录高层工作包和定位锚，不代替获批后 `cadence/plans/` 中的精确实施 Plan。每项可独立提交/验收；所列文件为该项执行窗口的独占写面，重复文件必须串行移交，不能同时派工。锚点行号取自文档接地时，以符号为准。

工作包规模均为派工拆分用的 `[INFERENCE]`，单项不超过 4h，并非追加总工期承诺；总规模锚沿用 Max 双审/brief §6：实现与测试改造约 0.5–1 个工作日，统一验证、独立审查和裸仓页面复验另约半天，精确安排由获批 Plan 展开。

## 1. 先修有效根 fixture

- [x] 1.1 **先改真实有效根 fixture，不先改放行断言**（预计 1–2h，独占 `src/product/logical_codebase/provider_admission_preflight_tests.inc.rs`：`admission_fixture_with_policy` 285–438、`publish_root_policy_with_receipt` 1241–1292）。构造真实 canonical root 规则、AGENTS 原字节、真实已发布政策 locator 与匹配最终 receipt；明确区分普通有效根、故意缺根/缺 receipt、placeholder 兼容和 fresh-bootstrap fixture。**验收：**有效根 fixture 确有根 `language.md` 与可核验发布链，不靠补成员规则或 placeholder 证明根有效；负面/兼容 fixture 仍可单独表达原边界。

## 2. 路 A：普通根消费者的成员缺失仅审计

- [x] 2.1 **先失败矩阵与公共 admission 收窄一起交付**（预计 2–3h，独占 `src/product/logical_codebase/provider_admission_check.inc.rs`：`check` 184–265、289–354、469–490；`src/product/logical_codebase/provider_admission_preflight_tests.inc.rs`，依赖 1.1）。覆盖真实有效根＋全部/部分成员缺失继续；保留 `missing_member_rules` 非阻断审计，不读取不消费成员文件正文；只有这些缺失时 ready=true、allowed_actions=[]。保留无 recorded checkout 等真实阻断项，不清空整个 `missing_materials`。改写 `missing_member_language_rule_waits_before_provider_spawn`；更正 `ordinary_missing_root_session_zero_spawns`（1144–1208）名实不符的名称、注释与断言，并提供真正的普通已发布政策＋根缺失零启动对照，保持 cwd≠target 与 symlink alias 独立性。**验收：**上述正向矩阵经公共预检 ready、审计存在、无成员准备动作；真实阻断仍拒绝且零启动；根缺失/receipt 缺失或损坏/locator、AGENTS digest、root 身份漂移沿既有门拒绝；Normal＋桩政策边界不变、不扩大，成员文件树与 Git 状态不变。
- [x] 2.2 **复核规则引用的审计语义并修正旧注释**（预计 0.5–1h，独占 `src/product/logical_codebase/provider_admission_check.inc.rs` 的结果构造与旧注释；只读定义锚 `src/product/logical_codebase/provider_admission_credential.inc.rs::ProviderRuleReference` 33–37 及现有导出/引用面；依赖 2.1，同 owner 串行）。复核 `digest: None` 当时的全库使用面，保持缺失审计，纠正「任何相位都阻断」注释，不增加消费协议或伪造 digest。**验收：**报告列出已检查的引用/消费面；不存在下游把空 digest 或引用在场当作正文证明的未处置问题，审计记录仍可观察，生产语义与 REQ-REG-12 一致。

## 3. 路 B：有效 bootstrap 凭据与既有 RootRecipe 通道

- [x] 3.1 **先失败完整 bootstrap admission→gateway 用例，再按已重验 phase 接线**（预计 2–3h，独占 `src/product/logical_codebase/provider_admission_check.inc.rs`：`check` 140–168、356–379；`src/product/logical_codebase/provider_admission_preflight_tests.inc.rs`；依赖 2.1，公共文件同 owner 串行；只读锚 `src/product/logical_codebase/provider_gateway.rs::validate_root_recipe_request` 1061–1074）。有效 durable Running credential＋根/全部成员规则无＋最终 receipt 无时进入合法 recipe，gateway 复用现有 RootRecipe 能力链，不先走 Normal 矩阵拒绝。改写 `bootstrap_phase_only_waives_missing_root_rules` 与 `lcg_t03_member_language_rule_missing_is_not_root_phase_exempt` 的旧全相位成员门断言/名称；扩展 `lcg_t02_unknown_normal_matrix_keeps_existing_root_recipe_contract`，不能只测 capability source。**验收：**Normal Unknown＋合法已交付 recipe 证据＋有效凭据可通过完整 bootstrap 预检；无/失效凭据、operation 非 Running、LC/root/step/input digest 漂移、recipe 证据版本漂移均零启动；普通会话仍因 Normal capability 不满足拒绝，不伪造 Confirmed，不改变 authority/policy/trust/cwd/target/availability/D4/#8 门，成员树/Git 状态零写。

## 4. 路 C：SC 实际路径、同一正文与 waiting

- [x] 4.1 **SC 消费范围先失败测试与同源读取一起交付**（预计 2–3h，独占 `src/web/workspace_ws_handler/run/single_candidate.rs`：`preflight_lc_member_rules_before_author_run` 498–556、`run_single_candidate_author` 558–744；`src/web/workspace_ws_handler/tests/single_candidate_lc_admission.rs`；依赖 2.1）。LC 规则读取本身成为消费前门，成功读取的同一份 `String` 直接进入 prompt；使用已解析 `repository.path`，不另选首个 Main/任意 checkout、不枚举全部 active 成员作为 SC 文件门。只读来源锚：`src/product/workspace_repository.rs::plan_session_repository_target` 393–406、`aggregate_root_view` 313–331；`src/product/repository_store_parts/resolve.inc.rs::resolve_logical_repository_in_lc` 112–167。**验收：**聚焦成员有规则而其他成员缺失可启动，prompt 为实际聚焦路径原文；focus 缺失停等且零启动；多 target、focus 非唯一时读聚合根并不等待成员文件；primary checkout 与首个 Main 不同仍消费已解析来源，不从其他 worktree 兜底。保留根/policy/trust/capability 等其他既有门。
- [x] 4.2 **实际读取错误与既有 SC/非 LC 合同回归**（预计 1–2h，独占 4.1 的同一两个文件，依赖 4.1，串行移交）。先失败覆盖 LC 实际消费文件的 NotFound、权限、目录与非法 UTF-8；detail 区分错误和路径，统一落 `member_language_rules_missing` 语义、Prepare/Retry、durable Prepare/Open，不成为首次 Failed；文件恢复后同一会话继续。保留 `lc_missing_member_language_rules_converts_to_waiting_prepare_face`、`lc_waiting_face_resumes_after_member_rules_restored`、`lc_run_with_member_rules_present_starts_author_directly` 的核心臂。**验收：**四类读取错误均 provider 零启动，恢复后同来源正文进入 prompt；`src/product/work_item_split_engine/prompts.rs` 41–44 的拒绝启动合同、`src/product/workspace_engine/conversational_gate_parts/sc_revision_delivery.inc.rs` 71–78 的 revision 注入、`src/product/workspace_engine/types.rs` 254–264 的 durable 分流未改；非 LC `src/web/workspace_ws_handler/tests/single_candidate_provider_run.rs::single_candidate_author_rejects_missing_language_rules_before_provider_start`（946 行起）仍保持原失败行为，不新增 SC phase。

## 5. 聚合初始化集成时序

- [x] 5.1 **bootstrap repair 集成随行为调整，而非只删错误码断言**（预计 1–2h，独占 `tests/it_web/web_lc_bootstrap_repair.rs`：`a03_new_lc_reaches_planning_ready_without_manual_seed_or_duplicate_provider_turn` 537 行起、`a04_failed_identity_and_missing_rules_require_product_repair_before_read_switch` 770 行起/1010–1076；依赖 3.1）。a03 不再预写 543–544 行的成员规则，让根材料由 recipe 生成；a04 去掉「缺成员规则第一次阻断、补成员后第二次才消费注入 pre_check 中断」的旧前提，按第一次即进入真实 recipe 链调整整个重试时序、turn 数和断言。**验收：**真实裸成员可推进 planning-ready；故意注入的中断仍有正确失败事实与显式重试，根材料仍由 recipe 生成、成员文件树/Git 状态零写、无重复 provider turn；初始化既有失败/Retry 投影不被误改成 SC Prepare/Open。

## 6. 回滚违规注入、保留 SC 自备材料

- [x] 6.1 **四模板源回滚/缺席验收**（预计 0.5–1h，独占历史注入目标 `e2e/fixtures/repos/busi/.claude/rules/language.md`、`e2e/fixtures/repos/api/.claude/rules/language.md`、`e2e/fixtures/repos/gateway/.claude/rules/language.md`、`e2e/fixtures/repos/frontend/.claude/rules/language.md`；只读 `e2e/lib/assemble-core.ts::instantiateMemberRepo` 97–106）。处理 controller 为绕过准入手工补入的模板规则；若已回滚则只验收不存在，不重建。**验收：**确认注入归属后四源均不存在，重新装配不把 language.md 复制/提交进成员基线；不删除用户规则、不重置成员仓。文档接地时这四源已不存在，不能把此次文档交付报告为实际执行了删除。
- [ ] 6.2 **当前四验收副本回滚，由 controller 执行**（预计 0.5–1h，独占 controller 已确认的当前 `busi`、`api`、`gateway`、`frontend` 验收副本中手工注入的 `.claude/rules/language.md`；不发明运行目录或擅改其他副本）。依 brief §3.3 核实并撤销手工补成员规则的 workaround。**验收：**四副本恢复无任何规则文件的骨架形态，回滚范围/归属可审计，不覆盖用户修改、不 reset/revert 成员仓；已不存在的副本文件不重建。
- [ ] 6.3 **保留真实 SC seed，只纠正全相位阻断文案**（预计 0.5h，独占 `tests/it_web/web_lc_gateway_multi_provider/harness.rs`：3694–3700、7575–7585）。`seed_member_rule_material` 调用和实现均保留为 SC 测试自备材料；相关注释改为实际消费者边界。**验收：**seed 未被删除或作为产品自动生成路径，有 seed 的 provider 矩阵不被当作 owner 裸仓通过证据，文案不再宣称任何 phase 都必须有全部成员规则。

## 7. 跨路径验证与 owner 裸仓终验

- [ ] 7.1 **由 Main 汇合后统一定向验证及独立审查**（预计 2–3h，无生产文件写面；依赖 2–6 组）。覆盖公共 admission、SC admission、bootstrap repair、已改 root-authority/receipt/capability 失败门，以及无 checkout、manifest/cwd/target/policy 等既有拒绝；审查核对消费者范围、空 digest 审计、同源 String、凭据到 RootRecipe 完整通道和禁止事项。**验收：**Main 留存实际命令/输出和红→绿证据，范围内用例通过；没有用新增根校验、TTL、成员生成、伪造能力或删历史门换取绿灯。worker 不自行并发跑 build/test/lint/formatter；精确命令由已确认 Plan 展开，本工作包不要求本次文档作者执行产品检查。
- [ ] 7.2 **controller 真实页面复验四个裸成员仓**（预计 1–2h，无仓内源码写面；依赖 7.1、6.1、6.2）。装配后明确断言四仓无任何规则文件，再点「启动聚合初始化」。**验收：**不再出现 `member_language_rules_missing` 阻断，合法 recipe 产出根材料、成员树/Git 状态零写、五阶段后续可继续；留存真实页面与操作结果，不用有 SC seed 的矩阵绿替代。4317 服务仅 controller 管理，实施者不启停/改配置。
- [ ] 7.3 **规格基线与交付契约收口**（预计 0.5–1h，独占本 change 的 `proposal.md`、`design.md`、`specs/logical-codebase-registration/spec.md`、`tasks.md`；只读旧 change/主规格）。核对 lcg A9/Task 3a supersede 声明及 `lc-root-initialization` 是否先归档，保证 delta 针对正确现行 REQ-REG-12；按真实完成项勾选。**验收：**三路、回滚和全部边界与冻结 brief 一致，OpenSpec strict 校验通过，未实施项不伪勾；新场景显式替换旧「bootstrap phase 只豁免根规则」身份，不保留错误旧语义。若旧 change 先归档，按时序条款替换归档后主规格的旧身份/内容并以新基线 strict 校验；该同步动作不在本次文档生成任务中执行。

## Workflow follow-up

- 书面契约获批后才调用 writing-plans，将精确文件步骤、验证命令和提交边界写入 `cadence/plans/`；Plan 只能展开此清单。
- 完成实施、Main 验证与独立审查后，再按既有 OpenSpec sync/archive 流程处理；若 `lc-root-initialization` 先归档，针对归档后主规格同步本 correction，避免旧 delta 覆盖新契约。
