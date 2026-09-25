# F-60 P1b 进度报告：Plan JSON 子链增量契约等价（worktree 基重做）

- 日期：2026-09-26
- 基线：worktree `.worktrees/feat-b-0808-add-monorepo` @ `3dfacdb6`（F-60 P0 已落地）
- 方案：`cadence/designs/2026-09-25_技术方案_F60一次成功率_v1.0.md`（v1.1）因素一表 Outline/Split/Draft 行 + 因素五 P1 节 + oracle 修正
- 前手参考：F60P1 曾误落主仓 main（已废弃，代码不存于任何分支）；本报告为 worktree 基上的完整重做

## 1. worktree 基盘点结论（先盘点、不重复造）

对 `src/product/work_item_split_engine/prompts.rs`（基线状态）逐项核对方案 P1 要求：

| 方案 P1 要求 | 基线状态 | 结论 |
|---|---|---|
| Outline 初次：真实来源 ID 规则 + 最小示例（request 实际 spec_ids） | 已有：`example_source_spec_id_array`（真实 ID 优先）+ strict contract 来源条款 + `outline_traceability_example.rs` 测试 | **无需重做**（前手提示的 outline_source_spec_id_rule/真实 ID 示例确已在基线） |
| Outline 初次：选项语义 | 已有：`split_option_semantics`（[user_option_semantics] kind 组成教学）+ [user_options] | 无需重做 |
| Outline 修订：来源 ID 规则 + 最小示例 | **缺**：修订 strict contract 无来源 ID 条款、无最小正确示例（与方案变体表「依赖历史」判断一致） | P1b 补 |
| 占位回退显式标注（因素二：不得把占位当真实来源教给 author） | **缺**：request 无真实 ID 时示例静默回退 `story_spec_0001` 占位，无「仅为形状占位」标注 | P1b 补 |
| Split 初次/redo 共用 output contract | **部分**：sentinel+schema+kind 指导为两份重复内联字面量；redo 分支缺 [user_options] 与 [openspec_constraint_summary]（初次均有）——Final Compile 对 work_items 强制 kind 组成（`work_item_split_validator/plan.rs` `frontend_backend_split_required` 等）与双来源追踪（`traceability_refs_required`），redo 丢选项重述会破坏组成约束 | redo 补重述 + 抽共享渲染 |
| Draft：封闭字段 + [self_check] + 唯一 sentinel + 15,600B 预算 | 已有（`prompt_contract.rs` 既有断言）；反馈轮变体无 [self_check]/预算钉住测试 | 核对通过 + 补反馈轮回归测试 |
| nonce 每轮新生成 | 结构上已保证（四个 builder 各自 `structured_output_nonce()`，无旧轮 nonce 复制路径） | 补「初次≠修订≠二次修订」回归钉住 |
| 共享合同渲染（初次/修订等价的可证结构） | **缺**：初次/修订 strict contract 为两份大体重复的内联字面量 | P1b 抽共享函数 |

## 2. 变更内容（`src/product/work_item_split_engine/prompts.rs` + 测试）

1. **Outline 共享合同渲染**：新增 `outline_strict_output_contract(nonce, source_id_rule, minimal_example, revision)`——初次/修订共用同一渲染，gate 必需条款（sentinel/nonce 规则、JSON 转义、context_blockers 纪律、完整 schema、最小正确示例、来源 ID 条款）两轮同源；轮次差异仅限措辞（提供/保留、context_blockers 首猜指引仅初次、规划过程/修改说明）。
2. **Outline 修订补齐来源教学**（增量契约等价核心）：修订 strict contract 新增来源 ID 条款 `outline_source_spec_id_rule(..., revision=true)`——直接点名 request 已确认的真实 spec ID（story/design 分句），对应一侧为空时指回上一版 outline 既有来源；两数组禁止空数组、禁止虚构或照抄占位 ID。修订 prompt 同时注入与初次同一份最小正确示例（真实 ID 优先）。
3. **占位显式标注**：`example_source_spec_id_array` 返回 `(String, using_placeholder)`；`minimal_example_prefix` 在占位时输出「示例中的 spec ID 仅为形状占位、不是真实来源，必须替换为会话中已确认 spec 的真实 ID，禁止照抄占位值」（初次/修订同规则）。
4. **Split 共享 output contract + redo 选项重述**：新增 `split_output_contract(nonce, composition_rule, schema_tail)`（sentinel 规则、可读过程纪律、kind 合法值、完整 schema 两轮同源，仅组成条款按轮次置换）；`build_revision_prompt` redo 分支补 `[openspec_constraint_summary]`（真实 story/design spec ID）与 `[user_options]` 四旗标完整重述——redo 轮不依赖会话历史补选项约束。
5. **Draft 核对**：正文未改（封闭字段合同、[self_check]、sentinel、预算断言维持既有）；新增反馈轮回归测试钉住「Some(feedback) 下 [self_check]/[canonical_field_contract]/sentinel/15,600B 预算不丢」。
6. **nonce 回归钉住**：新增测试断言初次/修订/二次修订 nonce 两两不同、修订 prompt 不携带上一轮 nonce、Split 初次≠redo nonce。

新测试文件：`tests/f60_plan_json_contract_equivalence.rs`（9 个测试，TDD：4 个缺口测试先红后绿；nonce/Draft 核对测试基线即绿作为回归钉住）。

## 3. P0 分层合同对齐评估（前手未考虑项）

P0 为 Markdown author 族立了两档先例：初次=完整合同块；后续轮（choice 续跑 delta／resume 增量修订）=一行短引用（点名全部必需 heading+关键 token+指向会话开头完整合同）。对 JSON 子链的评估结论：

**JSON 子链不适用「修订=一行短引用」形态，分层以「上下文两档」同构实现**（已在注释 `outline_strict_output_contract` doc 中落档）：

1. **nonce 每轮新生成**与短引用根本冲突：P0 短引用指向「会话开头的完整合同」，而 JSON sentinel 规则内嵌当前轮 nonce——短引用会把 sentinel 匹配规则引向旧轮 nonce，主动误导（比缺合同更糟）。
2. **P1 契约等价要求修订自足**：方案因素五 P1 测试要求修订轮携带「最新 nonce、source ID、必填 schema、options 语义」；「必填 schema」无法一行穷举（Markdown 7 个 heading 可以）。
3. **JSON 子链的两档在「按轮次裁剪重复内容」上与 P0 同构**：初次=全量上下文（story/design/repo 结构）+完整合同；修订=短上下文（issue_ref+feedback）+完整合同+真实来源 ID 教学。P1b 在修订侧补齐的来源 ID 条款+最小示例，正是 P0 短引用「点名关键 token 不丢」精神在 JSON 面的对齐动作；修订不重复全量上下文对应 P0 的「不重复全文」。

## 4. 与前手（主仓 F60P1，已废弃）方案差异

前手报告不可得（主仓 `cadence/reports/` 无该文件，前手提交不存于任何分支），差异按 context 转述对照：

- 前手未考虑 P0 分层对齐（context 明示）；本报告 §3 完成「评估 JSON 合同的短引用形态」并落档结论。
- 前手在主仓 main 上实现（基线不含 P0 出口装配）；本实现基于 worktree 3dfacdb6（P0 已在），盘点后只补真实缺口、不重复造基线已有部分（初次真实 ID 教学、选项语义、Draft 合同均未重写）。
- 本实现把「共享合同渲染」落实为单一函数（初次/修订参数化），而非两份平行文本补丁，等价性由结构保证。

## 5. 验证证据

- TDD 红→绿：4 个缺口测试（修订来源规则+示例 / 修订占位标注+上一版指向 / 初次占位标注 / split redo 选项重述）在实现前 FAILED、实现后 ok。
- 定向全量：`cargo test --lib product::work_item_split_engine` → **126 passed, 0 failed**（基线 117 + 新增 9）。
- 关联域：`cargo test --lib product::workspace_engine` → **772 passed**（与 P0 报告基线一致）；`cargo test --lib workspace_ws_handler` → **131 passed**。
- `cargo clippy --lib -- -D warnings` → 干净；`cargo fmt` → 已格式化。
- 未动服务器/运行时 gate；改动域仅 `work_item_split_engine` prompts + 测试。

## 6. 边界与遗留

- Split 面未新增 `split_option_semantics`（kind 组成教学）到初次/redo——初次 Split prompt 基线即只有 [user_options] 原始旗标，redo 按「与初次同构」原则补重述；把 outline 级 kind 组成语义下放到 Split 属新教学范围，超出 P1「补必要选项约束」，如需另议。
- 修订 trusted_verification_commands 条款措辞统一为「必须保留/提供 trusted_verification_commands：仅登记已确认仓库/Design/Outline 证据支持的…」（原初次「必须包含」/修订「必须保留；仅登记证据支持的」两份措辞合并为共享渲染的参数化形式），语义不变（合并后教学更强：点名仓库/Design/Outline 三类证据）。
- 占位 ID 仍为 `story_spec_0001` 风格（既有测试钉住的零填充风格），仅在无真实 ID 时出现且已显式标注。
- P2 真实 provider 首轮成功率测量不在本包范围（方案 P2 独立立项）。
