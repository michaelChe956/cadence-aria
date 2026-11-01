## V1 逐项核验表

| 项 | 结论 | 证据 | 修正建议 |
|---|---|---|---|
| 3. build_review_input 判例插入点 | **正确（位置需精确到行）** | `src/product/workspace_engine/prompts/review.rs:15` 起：开头声明(38-42) → `reviewer_boundary_rules_for`(43) → schema gate(44-46) → compact_history(47-60) → missing notes(61) → artifact diffs(62-66) → open findings(67-69) → 当前 Artifact(70-71) → 「审核边界说明」固定段(72-79) → nonce 构造(80-81) → `reviewer_output_contract`(82-105)。判例串应插在 **review.rs:79 之后、80 行 nonce/contract 之前**，形如 `if matches!(self.session.workspace_type, WorkspaceType::Design) { prompt.push_str(...); }`（约 4 行，与清单一致） | ① 插入必须严格在 contract 之前——`part_08.rs:569-583 assert_review_contract` 断言 `nonce="{nonce}"` 出现次数恰为 1 且不含 `</ARIA_STRUCTURED_OUTPUT nonce=`，判例串只要不含 `nonce=` / `ARIA_STRUCTURED_OUTPUT`（清单已锁）即不破坏；② 清单提到的 prompts.rs:809-812 实为 `author_skeletons_are_gate_incomplete...` 测试（`prompts.rs:781-798` 是 skeleton strip_prefix；`prompts.rs:803-812` 是 EXAMPLE_NONCE/96aca42f 顺序断言，属 `reviewer_output_contract` 内部单测），与本插入点**无耦合**，不会破坏 |
| 4. is_repairable 指纹判据 + 删 NonceMismatch | **正确但有两处需明确** | `review/structured_output.rs:38-51`；`recoverable_value()` 返回 `Option<&serde_json::Value>`（:31-36）。死分支确认：`cross_cutting/structured_output.rs:123-132` NonceMismatch 仅在 `failed(..., None)` 路径产生，recoverable_value 恒 None → `is_repairable` 的 `error.recoverable_value.is_some()` 前置条件使该 arm 永假，可安全删除。删后 matches! 剩 `MissingEndTag / InvalidEndTag / MissingJsonNonce / JsonNonceMismatch` 四个。其它引用：`is_repairable` 仅 `review/drive.rs:62,270` 两处调用，`NonceMismatch` 在 product 下无其它引用（cross_cutting 枚举本身保留，`parse_block_at` 仍产生该 code，仅不再走 repair） | ① **序列化稳定性**：Cargo.toml:32 `serde_json = "1.0.149"` 未开 `preserve_order`，Map 为 BTreeMap，`value.to_string()` 字段序确定；且判据是「四个 ID 子串各自存在」与字段顺序无关，ID 含 `-`/数字无需转义——建议实现为 `let s = value.to_string(); [\"DEC-001\",\"CMP-002\",\"API-002\",\"REQ-003\"].iter().all(\|id\| s.contains(id))`，勿做整体字符串相等；② **误判率**：`artifact_constraints.rs:403` 显示真实骨架 token 形如 `[DEC-001]`（3 位），与判例 ID 同形——单个 ID 撞车常见，但要求**四 ID 同时**出现在同一份 reviewer JSON（verdict/summary/findings 文本）中概率很低；残余风险是真实 review 回显完整追踪关系时被误判不可修复→退 needs_human（fail-closed 方向，可接受）。建议在测试里加一条「真实四 ID 追踪 review 仍可修复会误伤」的已知取舍注释，或把判例 ID 编号选成真实产物极少组合（如 DEC-001/CMP-002/API-002/REQ-003 已是错位编号，保持即可）；③ 指纹判据建议同时覆盖 `MissingEndTag/InvalidEndTag`（它们的 recoverable_value 同样可能含照抄载荷），清单只写了两个 code，属可加固点非必改 |
| 5. repair 回灌 readable 文本 | **正确，且比清单预想更简单** | 不在 `ReviewCompletionError` 上，而在 `ProviderCompletion.readable_output`（`cross_cutting/streaming_provider/mod.rs:250-252`，`from_output`:277-283 用 `parse_structured_output` 的 `parsed.readable_output`，失败态也保留剥离 sentinel 后文本）。`build_review_repair_input` 已持有 `completion: &ProviderCompletion`（`prompts/review_repair.rs:14`），把 :40 `completion.full_output` 改为 `completion.readable_output` 即可，**无需配套改动** | ① 边界：`MissingStartTag` 时 readable_output==full_output，但该 code 不可修复，无影响；② readable_output 可能为空串（全部输出都在 sentinel 内被剥离）——prompt 模板「原始输出：\n{}」空串无害，但建议空时回退 full_output 或省略该段，避免弱模型面对空段乱补；③ nonce 排除提示行注意不要包含 `nonce="` 字面（part_10:758 的 prompt 断言不受影响，但保持与 part_08 契约同款纪律）；④ 既有回归 `part_10.rs:727-761` 用 `ProviderCompletion::plain`（full==readable），改造后仍绿 |
| 6. build_author_revision_prompt 签名与 Design 注入 | **基本正确，两处需修正** | 当前签名 `pub(crate) fn build_author_revision_prompt(&self, feedback: &str) -> String`（`prompts/author_revision.rs:7`）。生产调用方唯一：`prompts/revision.rs:47`（在 `build_revision_input_with_resume` 内，`resume_provider_session_id` 已在 :37-41 算好，fresh/resume 标记可传 `resume_provider_session_id.is_some()`）。测试调用方唯一：`tests/author_revision_loop.rs:477`——**签名变更必须同 commit 改此行否则编译失败**。四项注入素材全部现成：`author_artifact_schema_contract_for`（`artifact_constraints.rs:310`）、fence 契约文案范本（`prompts.rs:454-458`）、`author_artifact_skeleton_example`（`prompts.rs:471`）、`append_missing_context_notes_to_prompt`（`prompts.rs:427`，是 `&self` 方法可直接复用）。Story 分支字节不变可行：注入与四反引号全部包在 `if workspace_type == Design` 内；无既有测试断言三反引号围栏（`author_revision_loop.rs:471-479` 只断言 contains 反馈/标题/改动摘要/增量修订） | ① `workspace_type` 参数冗余：方法内 `self.session.workspace_type` 直接可得，建议只加 fresh/resume 一个参数（或两个都不加、全部内部推导），减少签名扰动；若坚持双参数，调用方与 477 行测试同步；② 四反引号嵌套表述建议抄 prompts.rs:457 的既有措辞模式并在输入段写明：「下方当前产物用四反引号围栏包裹，产物内部的三反引号代码块属于产物内容，不要当作输入边界」+ 输出段保留既有 ```` 规则；输出侧 fence 是三反引号 ```artifact（内嵌代码块时四反引号），输入四/输出三的区分要一句「输入围栏仅用于界定材料，输出请按 artifact fence 契约重新包裹」；③ Design 产物若自身含四反引号序列仍会破围栏——建议测试 12 加一条「内嵌三反引号」用例并把四反引号列为已知边界 |
| 8. skeleton 提示语 REQ/AC→DEC/CMP/API | **需修正（波及面比清单大一行）** | 提示语与测试前缀是**四种类型共享同一字面串**：`prompts.rs:476/479/482/485` 四条 skeleton 都带 `缺稳定 ID、REQ/AC 与追踪 token`；测试 `prompts.rs:781-798` 用**单一 strip_prefix 串**（:791）循环四种类型。若只改 Design 行(:479)，:791 的 strip_prefix 对 Design 会返回 None → 测试 panic | 测试重构应为「每类型期望前缀表」或改用通用定位（如 `split_once(\"：\\n```artifact\\n\")`）后仍校验 gate 不通过；同时 :473 的 Story 注释保留 REQ/AC 措辞不动。除该测试外，`rg REQ/AC` 全仓仅命中 prompts.rs 这 6 处（473/476/479/482/485/791），无其它文件波及 |
| 9. tests/design_reviewer_boundary.rs | **有风险（文件位置/接线）** | `build_review_input`、`build_review_repair_input`、`is_repairable` 全是 `pub(crate)`；仓库根 `tests/` 是集成测试目录**无法访问**。workspace_engine 测试的接线方式是 `src/product/workspace_engine/tests.rs:1-35` 的 `include!("tests/part_NN.rs")` + :36-37 `mod author_revision_loop;` | ① 新文件应放 `src/product/workspace_engine/tests/design_reviewer_boundary.rs`，用 `mod design_reviewer_boundary;`（推荐，自成 helper 域）或 include!（可复用 part_01 的 `make_session`/`artifact_payload`/`complete_design_artifact`——part_01.rs:307/40/59 已有这些 helper，够用）；清单漏了「在 tests.rs 加一行声明」的配套；② 「判例恰 1 次」断言：非 Design 时判例为 0 次——只要断言写 `count()==1` 仅对 Design session、`count()==0` 对 Story/WorkItem/WorkItemPlan 各建一个 session，无误判；判例唯一 marker 选判例专属句子而非 ID（ID 可能与骨架/规则文本撞），并加锁 `!contains(\"nonce=\") && !contains(\"ARIA_STRUCTURED_OUTPUT\") && !contains(\"EXAMPLE_NONCE\")`；③ candidate→finding 走解析路径用 `ProviderCompletion::from_output`（part_10:936 有范本）即可，无需跑 drive 全链路 |
| 10. part_02 Design 负例矩阵 | **正确** | `validate_workspace_artifact_constraints` 为 pub(crate)（artifact_constraints.rs），part_02.rs 已有 Design fixture（:118、:287-334），基准可用 `complete_design_artifact`（part_01.rs:59） | 无。注意矩阵走真实校验路径时 Story 的 REQ/AC 断言别被 Design 用例覆盖串味 |
| 11. part_32 Design 滑窗 fixture | **正确** | part_32.rs（486 行）经 `tests.rs:34` include!，共享 part_01 helper；既有 part_31/part_32 已覆盖三入口滑窗行为 | 无 |
| 12. author_revision_loop 扩展 | **正确（依赖项 6 同步）** | 文件存在（1061 行），`prompt_engine_with_artifact`(:470 附近) 可直接换 Design artifact；「Story/WorkItem 字节不变负例」需在改造前先固化基线串（建议在改 author_revision.rs 的同一变更里先写快照断言再改实现，或直接 `assert_eq!(prompt, 旧实现手写期望)`） | 字节不变断言要求期望串硬编码完整 prompt——Story 分支现 prompt 较短可行；WorkItem 同理 |
| 13. 确认红线测试 | **正确** | `decisions.rs:54-58`（author_decision 仅 AuthorConfirm 阶段）、:187-190 `AcceptFinalize → finalize_current_artifact → Finalized`（:282 注释：Confirmed 落库 + Completed 节点）；pass 后走 `enter_author_confirm`（:858）不自动 Completed；单仓无 aggregate contract 由 `prompts.rs:222-236`（`is_aggregate_story_or_design` 才注入 contract）保证，断言 `structured_output_contract.is_none()` 即可 | 「无 aggregate contract」同时断言 revision input（`build_revision_input` 恒 `structured_output_contract: None`，revision.rs:63）与 author input 两处更完整 |

## V2 清单遗漏的实施级配套

1. **tests.rs 接线行**（项 9）：`src/product/workspace_engine/tests.rs` 需加 `mod design_reviewer_boundary;` 或 include! 行——清单未列。
2. **author_revision_loop.rs:477 调用点同步**（项 6）：签名变更的既有测试调用必须同变更落地，否则编译失败。
3. **prompts.rs:791 strip_prefix 双前缀适配**（项 8）：见 V1。
4. **item 5 空 readable_output 回退策略**：建议空串时省略「原始输出」段或回退 full_output。
5. **判例常量导出**：`reviewer_boundary_examples` 需 `pub(crate) fn` 供 review.rs 与测试共用唯一 marker，避免测试自拼串漂移。
6. （可选加固）is_repairable 指纹覆盖 MissingEndTag/InvalidEndTag。

## V3 实施顺序微调建议

清单按 tasks.md 顺序（M3.1 repair → M3.3 skeleton → M3.4 红线 → 4.x 判例 → 5.x 修订）整体可行，无跨项编译死锁；微调：

1. **项 1+2+3 必须同一变更落地**（mod 声明与常量、注入点、防回归锁测试），项 3 单独落地会因引用未声明模块编译失败。
2. **项 6+7+12 同一变更**（签名 + 调用方 + 既有测试 477 行 + 新负例），先写 12 的 Story/WorkItem 字节不变快照（RED 不会红，作为守卫）再改实现。
3. **项 8 与其测试重构绑定**，且应在项 6 之前做（项 6 的 Design 注入引用 `author_artifact_skeleton_example`，若 8 后做会二次触碰同测试）——与 tasks.md 3.3→5.1 顺序一致，保持即可。
4. **项 4 先 RED 后 GREEN**（tasks.md 4.1 已如此）：四象限指纹测试先失败（现无指纹判据），再实现；`part_10.rs:727` 既有 repair 回归在改 review_repair.rs 前先跑一遍确认基线绿。
5. Campaign 产物（baseline 先于 prompt 改造）要求 M2.3 全部采集完成后再合入任何 prompt 改造 commit——建议 prompt 改造分支与 campaign 采集分时进行，避免 baseline 混入改造后行为。

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "只读审查：对修改清单 13 项逐一对照真实代码给出 file:line 证据与结论，未修改任何文件、未执行 git 写操作、未派生子代理"
    }
  ],
  "changedFiles": [],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {
      "command": "rg / sed 定向阅读（prompts.rs、prompts/review.rs、review/structured_output.rs、prompts/review_repair.rs、prompts/author_revision.rs、prompts/revision.rs、cross_cutting/structured_output.rs、streaming_provider/mod.rs、decisions.rs、artifact_constraints.rs、tests/part_01|02|08|10|32、tests/author_revision_loop.rs、openspec/.../tasks.md）",
      "result": "passed",
      "summary": "完成 7 组核验点的代码取证，无只读约束违例"
    }
  ],
  "validationOutput": [
    "项3 插入点确认 review.rs:79-80 之间，与 prompts.rs:803-812 及 part_08 assert_review_contract 无冲突",
    "项4 NonceMismatch 死分支确认（cross_cutting/structured_output.rs:123-132 recoverable 恒 None）；serde_json 无 preserve_order，子串判据与字段序无关",
    "项5 readable_output 在 ProviderCompletion 上现成可得（streaming_provider/mod.rs:252,279），无需配套",
    "项6 生产调用方唯一 revision.rs:47；测试调用方 author_revision_loop.rs:477 需同步；workspace_type 参数冗余建议去掉",
    "项8 strip_prefix 共享前缀需按类型适配（prompts.rs:791）",
    "项9 新测试文件须落 src/product/workspace_engine/tests/ 并在 tests.rs 加声明",
    "实施顺序无编译死锁，微调建议见 V3"
  ],
  "residualRisks": [
    "is_repairable 四 ID 指纹对真实回显完整追踪关系的 review JSON 有低概率误判为不可修复（fail-closed 方向，可接受，建议文档化）",
    "Design 产物内含四反引号序列时输入围栏仍会被截断（建议测试覆盖三反引号内嵌并把四反引号列为已知边界）",
    "reviewer_boundary_examples 提示语若未来与 part_02:334 Design boundary 规则文案互相复制措辞，唯一 marker 断言可能撞车——marker 应选判例专属句子"
  ],
  "noStagedFiles": true,
  "diffSummary": "零 diff：本任务为只读实现正确性审查，产出为中文 markdown 评审报告（V1 逐项核验表 / V2 遗漏配套 / V3 顺序微调）",
  "reviewFindings": [
    "需修正: prompts.rs:791 共享 strip_prefix 前缀 — 项 8 只改 Design skeleton 文案会使既有测试 panic，须按类型适配前缀",
    "需修正: 项 9 新测试文件位置 — pub(crate) 可见性要求放在 src/product/workspace_engine/tests/ 并在 tests.rs 补声明（清单遗漏）",
    "需修正: 项 6 — author_revision_loop.rs:477 既有调用必须与签名变更同 commit，且 workspace_type 参数冗余",
    "no blockers: 其余各项（3/4/5/10/11/12/13）按清单可实现，细节修正建议见报告"
  ],
  "manualNotes": "openspec 四件套已通读（proposal 43 行 / design 55 行 / tasks 34 行）；specs/ 为目录未展开逐 spec 比对，本审查以 tasks.md 修改清单为准。背景材料（analysis-docs 与 /tmp/design-analysis 五份报告）未读，不影响逐项代码核验结论。"
}
```