# Design: fix-admission-member-rules-scope

## Context

动机、supersede 声明和非目标见 `proposal.md`；行为合同统一归 REQ-REG-12 delta。本设计仅转录已冻结 brief 与 Oracle/Max 双审结论，不新增能力或重新选择方案。

### 已接地的当前落点

| 文件与符号/区间（相对 worktree） | 当前事实与本次边界 |
|---|---|
| `src/product/logical_codebase/provider_admission_check.inc.rs`：`LogicalCodebaseProviderAdmissionPreflight::check`，184–265、289–379、469–490 | 当前遍历 active 成员，选首个 Main/任意 checkout，读取规则后把错误归入 `missing_member_rules`，一律加入阻断材料；`missing_materials` 还含「无 recorded checkout」，不能整体清空。bootstrap 凭据先重验，但 gateway 仍无条件走 Normal `validate`。 |
| `src/product/logical_codebase/provider_gateway.rs`：`validate_root_recipe_request`，1061–1074 | 已有 RootRecipe 通道，以 `require_root_recipe_supported` 校验真实 recipe capability 并冻结凭据；不携带 Normal Confirmed 事实，应复用而非新造通道。 |
| `src/web/workspace_ws_handler/run/single_candidate.rs`：`preflight_lc_member_rules_before_author_run`、`run_single_candidate_author`，498–647、712–744 | 先经公共预检，再从已解析的 `repository.path` 读取 UTF-8 全文；当前第二次读取失败会写 Failed，成功正文进入 prompt。这是需要收口的真实消费点。 |
| `src/product/workspace_repository.rs`：`aggregate_root_view`，313–331；`plan_session_repository_target`，393–406 | 多 target、focus 非唯一时得到 AggregateRoot；其 `repository.path` 是聚合根，SC 并非总是成员规则消费者。 |
| `src/product/repository_store_parts/resolve.inc.rs`：`resolve_logical_repository_in_lc`，112–167 | 既有身份解析使用 identity registry primary checkout，并保留既有受约束的身份恢复分支；本次复用解析结果，不再按列表顺序另选文件来源、不扩展该解析规则。 |
| `src/product/workspace_engine/types.rs`：`work_item_plan_author_for_durable_flow`，254–264 | durable flow 已分到 `WorkItemPlanSingleCandidateAuthor`；不能再造 SC phase 或由 action 猜入口。 |
| `src/product/work_item_split_engine/prompts.rs`：`WorkItemPlanMarkdownAuthorContext`，41–44；`src/product/workspace_engine/conversational_gate_parts/sc_revision_delivery.inc.rs`：`build_sc_manual_revision_prompt_for_turn`，71–78 | SC 必需正文缺失/不可读拒绝启动以及 revision 的语言规则注入均保持不动。 |
| `src/product/cadence_skills/routing_reference.rs`：`logical_cadence_routing_rules_reference`，47–62 | 政策权威在 envelope 的根 locator；明确禁止在 target worktree 猜同名路径。本次不改变 cwd/target/authority。 |
| `e2e/lib/assemble-core.ts`：`instantiateMemberRepo`，97–106 | 递归复制成员模板并提交基线；只清运行副本而不处理四模板源，会再次污染裸仓验收。 |

这些落点限定修改面，不表示公共 preflight 已覆盖所有五阶段入口。Max 双审原文：

> 全库引用所见，LogicalCodebaseProviderAdmissionPreflight 的生产调用只有 coordinator_provider_turn.inc.rs:461-475 与 run/single_candidate.rs:524-555。其他五阶段 validated 入口通常直接走 gateway。因此应准确描述为“这两个入口共用的预检错误泛化了成员需求”，不能声称这个 preflight 本身已覆盖所有多仓入口。

## Goals / Non-Goals

**目标：** 按实际消费边界去掉错误成员门；使 fresh-LC 合法自举走既有凭据/RootRecipe 链；LC SC 的必要文件检查与 prompt 正文同源，恢复可诊断 waiting 而非运行时 Failed；恢复 owner 的真实裸仓验收。

**设计级非目标：** 不重写生命周期、authority resolver、SC prompt 或发布链，不引入 receipt TTL、全源 digest、新 phase、备用 cwd/worktree 或成员规则生成器。禁止事项详见后文；proposal 的所有非目标继续适用。

## Decisions

### 1. 三路消费矩阵是唯一分流合同

| 路径 | 必需规则来源 | 成员缺失如何处理 | 保持的硬边界 |
|---|---|---|---|
| A：普通根消费者（五阶段多仓正常请求） | canonical LC root 的规则、真实已发布政策、最终 receipt，按既有有效性校验 | 不消费的成员缺失仅审计；只有这些缺失时 ready=true、allowed_actions=[] | 根/policy/receipt 既有失败门及其他准入门不动；Normal+自举桩政策的根缺放行不动、不扩大 |
| B：AggregateBootstrap recipe turns | 已重验 credential 绑定的合法 root recipe | 成员规则门豁免；根规则及最终 receipt 尚未生成是首建预期 | durable Running、LC/root/step/input digest、真实 recipe capability，以及 authority/policy/trust/gateway/cwd/target/availability 原门 |
| C：LC SC author/重跑消费链 | 已解析 `repository.path/.claude/rules/language.md` 的 UTF-8 正文 | 仅实际消费文件必需；其他成员缺失不阻断 | SC 拒绝启动、Prepare/Open、原会话重试、revision 注入、其他根/policy/trust/capability 门不动 |

既定方案优先于两个已排除替代方案：只加「根有效即可忽略成员缺失」条件仍会锁住根尚未生成的 bootstrap；机械删除公共成员门会令 SC 实际读取错误重回 Failed。继续让全部 SC 检查全部成员则不能按消费者收窄，且 aggregate-root-view SC 会误拦。自动补成员文件不是替代方案，已经否决且违反不复制成员规则合同。

### 2. 路 A：诊断事实与阻断材料分开

延伸现有 `ProviderAdmissionPreflightResult` 的 `missing_member_rules`、`missing_materials`、`allowed_actions` 结构，不新建平行状态机。去掉「任何相位都阻断」的注释与测试前提，将不消费的成员缺失留为可审计事实，不读取成员正文做无谓 digest 计算；不能把不消费文件的不可读错误制造成全局门槛。

仅成员规则缺失时不加入成员准备动作；manifest、checkout、authority、policy 等真实失败仍保留。特别不能整体清空 `missing_materials`，因为同一集合含「成员无 recorded checkout」。

`ProviderRuleReference.digest: None` 的已审结论为「全库当前无下游消费（仅构造/导出）」；实施者须复核当时的全库调用/使用面，保持缺失审计，确保无下游把引用在场误当成正文证明。本次不发明 digest 值、不扩建其消费协议。

### 3. 路 B：先重验凭据，再复用相位 gateway

`ProviderAdmissionPhase` 仅表达生命周期。保持 `check` 中先对 durable Running operation 重验凭据的顺序，再按已重验 phase 调用 gateway：Normal 仍用现有 `validate`，AggregateBootstrap 调既有 `validate_root_recipe_request`。后一通道仍消费真实 recipe capability；不得把 Unknown Normal 行改成 Confirmed、移除 capability 门，或让普通会话借 recipe 证据搭便车。

fresh-LC 根规则和最终 receipt 不存在不等于校验失效：它们由本次合法 recipe 生成，按既有 bootstrap 边界进入。LC、root、step、input digest 漂移或 operation 退出 Running 时凭据立即失效，不能降级到普通会话继续、不能扩大其他豁免。保留 #8 发布链产物与复验、trust、target 和 D4 原有职责。

### 4. 路 C：在真实读取点完成 waiting 与同源消费

复用 `workspace_repository_for_session` 已给出的 `RepositoryRecord`。LC 路径消费前只读取 `repository.path` 下的实际文件；成功得到的同一份 `String` 直接用于既有 prompt context，不能先检查一份、随后再读另一份。无需新增消费 phase，SC 已由 durable flow 明确分流。

成员路径与 admission 的 checkout 判定收敛到既有身份解析结果（identity registry primary checkout），不能独立选首个 Main/任意 checkout，更不能从其他 worktree 找文件兜底。多 target、无唯一 focus 得到 aggregate root view 时，SC 仍需其实际根文件正文，但不要求所有成员文件。

LC 实际读取的 NotFound、权限错误、目录错误、非法 UTF-8 统一进入 provider 前的 waiting 合同，reason 保持 `member_language_rules_missing` 语义，detail 区分错误与来源，Prepare/Retry、durable Prepare/Open、provider 零启动；恢复文件后原会话读取并继续。仅 LC 臂改变，非 LC 当前失败行为保持原样。`WorkItemPlanMarkdownAuthorContext` 缺必需正文拒绝启动、conversational-gate revision 注入与 durable 分流不动。

## Boundary Semantics / 禁止事项

1. **不混称缺失与不可读。** 不消费的成员文件不读正文；实际消费文件的 NotFound/权限/目录/非法 UTF-8 在 provider 前可诊断停等。不能把 IO 错误无差别当作可豁免的缺失。
2. **成员豁免不是根豁免。** receipt 缺失/损坏、locator、AGENTS digest、root 身份漂移继续保持既有拒绝；不能把 placeholder 作为根有效证明或扩大普通请求放行。
3. **不许扩写校验承诺。** Max 双审原文：
   > “根规则与 receipt 有效”的现有校验能力有限：admission 对根 language 仅 is_file；verify_published_policy_body 校验 locator/policy/receipt 及 AGENTS 原字节 digest，不重算当前 language 内容；gateway 某些路径仅在 receipt 存在时校验它。故不能声称所有五阶段已经具备相同的缺 receipt、语言规则不可读或内容漂移拒绝能力。是否扩大这些既有缺口应单独明确范围，而不是悄悄混入本次收窄。
4. **不许发明 receipt TTL 或恢复 UI。** Max 双审原文：
   > receipt 缺失、损坏、policy locator/AGENTS digest/root 身份漂移均不能借成员豁免放行。但当前 latest_finalized 的 IO/JSON 错误可能作为 Store 返回，并非统一 Waiting；不能承诺已存在完整可恢复 UI。当前没有 receipt 时间 TTL，“过期”不可凭空解释为超过某个时长。
5. **waiting 投影按入口保留。** Max 双审原文：
   > 聚合初始化与 SC 的等待投影不同：初始化 admission 错误经 driver 包为 retryable ProviderTurn，bootstrap_projector.inc.rs:312-348 可表现为 Failed＋Retry；SC 才是 Prepare/Open。Prepare/Retry 文案也不等于已有自动生成成员规则的产品动作，报告与验收不可混称。
6. **不能扩大写面或能力。** 不动 trust/capability/cwd-authority/target/D4/#8 各门；不复制/自动生成成员规则，不伪造 Confirmed、不把 recipe 证据给普通会话，不为 SC 新造 phase、不改非 LC 行为。
7. **不能错误删除 SC 自备材料。** `tests/it_web/web_lc_gateway_multi_provider/harness.rs` 的 `seed_member_rule_material`（3694–3700、7575–7585）服务真实 SC，保留；不能与四模板/owner 副本注入一并清理。

## Migration Plan

1. 先将 admission 有效根 fixture 改为真实根规则、政策 locator 与匹配 receipt；正向用例不得靠 placeholder 或成员补文件过关。
2. 按三路补先失败测试并改产品对应边界，保留既有失败门；公共 admission 的 A/B 修改由同一 owner 串行处理，SC 实际消费臂在公共契约落定后接入。
3. 更新钉死旧全相位成员门的单测与 bootstrap repair 时序，保持 SC 核心 waiting 臂和非 LC 回归。
4. 由 controller 核实注入归属并回滚四个当前验收副本及 `e2e/fixtures/repos/{busi,api,gateway,frontend}/.claude/rules/language.md` 四模板源；只处理注入文件，不重置成员仓或覆盖用户修改。文档接地时四模板文件已不存在，故实施时核实已回滚则验收缺席，不能重建或假报再次删除。
5. 真正装配裸仓后，controller 统一验证并进行页面复验：四仓无任何规则文件，启动聚合初始化不再遇成员缺失门，根由 recipe 生成、成员树/Git 状态零写、五阶段后续可继续。服务 4317 由 controller 管，实施者不重启或改配置。
6. 同步/归档前按 `proposal.md` 的时序核对 REQ-REG-12 基线；「有效 bootstrap credential 解除裸仓首建死锁」显式替换旧 Scenario「bootstrap phase 只豁免根规则」身份和已 supersede 的只豁免根语义。若 `lc-root-initialization` 先归档，先在归档后主规格替换该旧身份/内容，再以新基线完成 strict 校验，不能为校验兼容恢复错误旧标题。本次文档任务不修改旧 change 或主规格。

## Risks / Trade-offs

- **[风险] 错把预存 placeholder 放行当成有效根证明。** → fixture 必须有真实根规则和 receipt；正负用例分离，普通根缺失/发布链失效仍零启动。
- **[风险] 公共成员门删除后 SC 读错来源或读取失败成为 Failed。** → 同一已解析 `repository.path`、单次 UTF-8 读取、同一正文注入，保留 Prepare/Open 与恢复原会话测试。
- **[风险] bootstrap 只测试 capability source 而漏掉 admission→gateway 接线。** → 完整路径覆盖 Normal Unknown、合法 recipe 证据和有效 credential；无效凭据/证据与普通搭便车均零启动。
- **[风险] 名为缺根的旧测试实际测成员缺失。** → 更正名称、真实根材料与断言对象，保留 cwd≠target/symlink alias 独立性的对照。
- **[风险] 有 seed 的矩阵绿掩盖 owner 裸仓死锁。** → 不删 SC harness seed，但终验必须是独立裸仓装配并断言四成员规则文件不存在。
- **[范围界限] 双审未实测外部 provider CLI 的隐式文件发现。** → 不据此发明额外消费者或准入门；以已确认的产品消费路径和真实 owner 页面复验为验收依据。
