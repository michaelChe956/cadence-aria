I have completed my read-only verification. I've read the frozen contract (proposal/design/specs/tasks), the background analysis doc, the four prior subagent reports, the actual production code (review.rs, review_repair.rs, structured_output.rs, author_revision.rs, revision.rs, prompts.rs, artifact_constraints.rs), the lifecycle downstream consumption (`validate_confirmed_design_specs`), the story campaign artifacts, and confirmed a clean working tree.

---

## G1 子目标覆盖矩阵（子目标→覆盖条目→缺口）

把「单仓 Design 段可用且被验证」拆为 6 个可判定子目标，映射到 13 项修改清单：

| 子目标 | 覆盖条目（清单编号） | 覆盖性质 | 缺口/风险 |
|---|---|---|---|
| S1 生成可用（author 产出结构合规 Design） | 8（skeleton 防照抄文案修正）、6（用户反馈返修入口注入 schema/skeleton/fence） | 初次生成（`build_prompt`）已由共享层覆盖（schema+skeleton+fence），本 change 只补**用户反馈返修入口**这一后门；8 是文案小修 | 初次生成的语义充分性（7 heading 是否填对、三族 ID 是否逐项追踪）不被 deterministic validator 保证——但已在 non-goals 明确（D-07 grammar 收紧另立项），由 campaign golden normalizer 的 dec_req_links/source 覆盖部分兜底 |
| S2 结构合规率（7 heading/三族 ID/source/禁止项） | 10（part_02 表驱动负例矩阵）、8、Campaign（golden normalizer + 6 形态 corpus） | **只测不改码**：validator（`artifact_constraints.rs`）已存在，本 change 用「合法基准单点变动→finding 断言」锁定现状契约 | 负例矩阵只锁「存在性」门（每类 token 至少出现一次），不锁「ID 唯一/非空/逐项追踪」——这是有意不碰（D-07），但结论措辞不能声称「结构语义充分」 |
| S3 review 可靠（边界判定 + 结构化输出 repair） | 1/2/3（判例模块+单点注入）、4（is_repairable 指纹+删死分支）、5（repair prompt nonce 排除+readable 回灌）、9（design_reviewer_boundary 测试） | 生产改动 + contract-driven 测试 | 判例只覆盖 3 条边界（抽象追踪/测试越界/风险提及）；禁止项 heading 的 must_fix 由 deterministic schema gate 覆盖，不需判例——覆盖完备。**但 B-1 主路径（两处 nonce 都写对的照抄）不靠 is_repairable，只靠判例无封装设计**，其正确性完全依赖条目 9 的负例测试断言内容（见 G4） |
| S4 返修可控（reviewer 返修 + 用户反馈返修） | 5（repair 侧）、6/7（用户反馈返修入口补契约+传参）、11（part_32 Design 多轮滑窗）、12（author_revision_loop Design 反馈） | 生产改动 + 测试扩展 | compact_history 本期在反馈入口**暂缓注入**（D3 有充分理由：resume 会话已有服务端历史、压缩无硬 token 上限）——意味着 Design 用户反馈返修在 resume 下无客户端历史压缩，这是**有意的残留缺口**，须在报告记录而非宣称已闭环 |
| S5 确认安全（pass 不自动完成/确认才 Confirmed/无 aggregate contract） | 13（确认红线测试） | 纯测试锁定现有行为（`routing.rs:94` 回 AuthorConfirm、`decisions.rs:284` finalize 才 Confirmed） | 已核实 `part_31.rs:919` 已有「单仓 Design 无 aggregate contract」半条测试，13 补齐完整红线——无缺口 |
| S6 实测达标（campaign 证据链） | Campaign 全套（corpus+fixture+digest+golden normalizer+强化 manifest 校验器+baseline/revised） | 新建验证产物 | 见 G2——这是**达成「可用」结论的最薄弱环节** |

**总体**：S1/S2/S3/S4/S5 的结构性改动均有对应生产改动与单测锁定，覆盖基本完备；「可用」结论的成败集中在 S6 的证据强度（G2）与全流程衔接口径（G3）。

---

## G2 证据链强度评估（含样本量统计弱点）

**证据链结构**：baseline（改造前）× 3 provider × 12 样本 + revised（改造后）× 3 provider × 12 样本，配对比较（--paired）。这是与 story 同级、且结构上正确的设计。但存在四个统计/口径弱点：

**1. full-chain「全数通过」的统计含义被高估（最核心）**
- 36 样本零失败是**硬 gate**（12/组合），不是失败率的统计估计。用 rule-of-three（0 失败，95% 单侧置信上界 = 3/n）：
  - 合并 36 样本：失败率上界 ≈ **8.3%**；每 provider 12 样本：上界 ≈ **25%**。
- 即「36/36 通过」只能支持「失败率 < 8%」这类弱结论，**不能支持「弱模型可用率 ≥ 95%」**。若最终报告写「达标」而不写置信区间，就犯了 story gate-report 同款问题（story 10/10 也从未声明置信界）。
- 样本多样性的另一弱点：12 样本 = **6 个不同语料 × 2 次重跑**（同文本跑 2 遍），不是 12 个独立语料。测的是「重跑方差」，不是「形态内泛化」。

**2. 边界假阳/假阴各只有 6 次观测，零门槛基本是 smoke test（最有力的质疑点）**
- D04（抽象追踪正例）假阳、D05（测试越界反例）假阴，各 2 重复 × 3 provider = **6 观测**。
- 0/6 的 rule-of-three 上界 = **50%**。也就是说「假阳率 < 50% 且假阴率 < 50%」——**零门槛判定不可信**，只能证明「没跑崩」，不能证明「边界可靠」。
- 若要把「边界零门槛」当作 release gate，6 观测远不够（建议每形态 ≥ 15-20 观测，0/20 → 上界 <15%）；若只当作「判例加固是否有效的触发信号」，则当前 spec 已用「独立最低门槛」表述，勉强可接受但必须在报告措辞上降级。

**3. D05 的「full-chain 一次成功」与「测试越界必须返修」存在口径冲突（必须澄清）**
- D05 设计上就是**故意让 reviewer 判 must_fix 并触发返修**。若「full-chain 一次成功」=「review 首轮 pass 无返修」，则 D05 永远不可能通过，gate 自相矛盾。
- 正确口径应为：D05 的 full-chain 成功 =「全链走完（author→review must_fix→返修→review pass→确认）且首轮边界判定正确」，边界假阴另行独立计。terra-review R3-M0 已明确要求此口径写进 campaign README（tasks 1.1 有「口径写入 README」的验证项，但**当前 spec/tasks 未写清这个 D05 特例**）——这是最小补充集第 1 项。

**4. baseline 的边界基线更薄**
- baseline 在无判例时采集 D04/D05 各 6 观测，用它证明「改造前有假阳/假阴、改造后归零」的**收益对比可信度同样受 6 观测限制**。收益结论只能写「在 6 次观测内从 X 降到 0」，不能写「边界问题已解决」。

**结论**：证据链**结构达标、强度不足**。它足以支撑「单仓 Design 段在 6 形态冻结语料上可跑通、无结构性回归」；**不足以支撑「弱模型边界判定可靠」**。这是整个 change 里唯一会决定「能不能宣布可用」的短板。

---

## G3 全流程衔接建议

**现状核实**：
- issue→story 已验证（story campaign 30/30）。
- design 段（生成→review→返修→确认）本 change 覆盖。
- design→work item 衔接：下游 `validate_confirmed_design_specs`（`lifecycle.rs:971-1003`）**只检查 `confirmation_status == Confirmed`，不重校验 design 的 heading/ID/source/finding**；`write_design_to_openspec_and_recompile`（`runtime_units/clarification/openspec.rs:62`）是独立 runtime 链路，不在本 change 范围。

**建议**：**不纳入 gate，但补 1 个端到端冒烟（只验证不改码）**，并把结论措辞与 proposal 范围声明对齐。

1. 单仓 Design 不产生 `involved_repository_ids`/`change_order`（aggregate 专属），design→work item 的跨仓 depends_on 风险**在单仓分支为零**——所以下游消费风险的实质性很小，不值得为它扩 gate。
2. 但「单仓全流程可用」这个最终目标措辞下，design→work item 是最后一段未验证链路。两种收口方式二选一：
   - **推荐**：在 revised campaign 中加 1 个「end-to-end smoke」样本（取 D01 确认后的 design，调 `generate_work_items` 端点，断言成功且 `source_design_spec_ids` 正确写入），**不设 gate**，仅作衔接证据。成本约 1 样本。
   - 或：最终验收标题/结论**严格降为「单仓 Design 段可用」**（proposal 范围声明其实已写：「验收结论只声称『单仓 Design 链路可用』，不声称全量 Design 链路可用」）——这是正确的自我约束，需确保 campaign 报告、gate-report、验收标题都沿用此措辞，**不得**写成「单仓全流程可用」。
3. 两者不冲突：smoke 补的是「衔接不断」，措辞约束防的是「过度承诺」。建议都做。

---

## G4 反向压力测试三问三答

**质疑 1：「36/36 只是样本太小/运气好，不能证明弱模型可用」**
- **是否成立**：部分成立。rule-of-three 显示 0/36 的失败率上界 ≈ 8%，「可用率 ≥ 95%」这类强结论确实撑不起来。
- **补充**：(a) gate 本质是 release gate 而非统计估计，与 story 10/10 同口径、且设计样本量（12/组合）已高于 story；(b) baseline→revised 配对能证明**相对收益**（改造后 ≥ 改造前），这个结论不依赖绝对样本量；(c) 语料冻结可复现。若要更强结论：把边界形态观测提量，或在报告明写「6 形态冻结语料上 36/36，失败率 95% 上界 <8.3%」。

**质疑 2：「边界假阳/假阴各 6 观测、零门槛等于没测」**
- **是否成立**：**成立**——这是三问中最有力的一条。0/6 的 95% 上界 50%，完全不能支撑「边界可靠」。
- **补充**：(a) 边界判例的**语义正确性**已被候选→finding 单测（条目 9）与 deterministic schema gate 部分锁定，真机测的是「弱模型读判例后的分类行为」——只有这一层弱；(b) 建议把 D04/D05 重复从 2 提到 ≥5（每 provider 5 次 → 每形态 15 观测，0/15 → 上界 <20%），或另立边界 mini-campaign，或**明确把「边界零门槛」降级为触发信号而非 release gate**。不补观测又坚持当 gate，就是假证据。

**质疑 3：「design 确认后到 work item 的衔接没验证，『全流程可用』是过度承诺」**
- **是否成立**：条件成立。若最终结论措辞是「单仓全流程可用」，则成立；若措辞是「单仓 Design 段可用」（proposal 已自我约束），则不成立。
- **补充**：见 G3——补 1 个 end-to-end smoke（只验证不改码）即可封口，成本极低；同时全链路验收文案统一为「单仓 Design 段」。

**额外发现（评审中核实到的实现细节）**：冻结 design.md D1 写「`reviewer_output_contract(...)` 结果**之后**追加判例」，但修改清单条目 3 写「结果**之前** push_str」，且 opus-deepdive D4 明确推荐「判例落在整个 contract 之前」（并建议加「判例在 `实际输出模板` 之前」的排序断言）。三者不一致——实现应取「之前」（判例先于输出格式/nonce 模板，语义更连贯，且不与真实 nonce 模板交错），但**必须**在契约里统一措辞并补排序锚定测试（条目 9 目前只写「判例恰 1 次」，未显式要求位置断言）。

---

## G5 最终结论与最小补充集

**置信度分级**：
- 「单仓 Design 段（issue→confirmed design）结构可用、无结构性回归」：**高（0.75）**——生产改动有充分 contract-driven 单测，campaign 提供与 story 同级的实测。
- 「弱模型边界判定可靠」：**中低（0.35）**——受 6 观测限制，「假阳/假阴 = 0」不能作为可靠性证据。
- 「单仓全流程可用（含 design→work item）」：**低（0.3）**——衔接链路未验证，措辞若不收敛即属过度承诺。

**必须补充的最小集合（按优先级）**：

1. **明确 D05 口径**：campaign README 写清「测试越界反例的 full-chain 成功 = 全链走完且首轮 must_fix 判定正确」，与「边界假阴独立计数」分离，避免把正确 must_fix 误记失败。
2. **边界观测提量或降级**：D04/D05 重复 2→≥5（或另立边界 mini-campaign）；否则把「边界零门槛」在报告中降级为辅助信号，不得当作 release gate 宣布「边界可靠」。
3. **统一注入位置并补排序断言**：design.md「之后」↔修改清单「之前」取「之前」，新增「判例出现在 `实际输出模板` 之前」的防漂移测试。
4. **明确 B-1 照抄负例语义**：该测试应断言「判例不含 sentinel/nonce/完整 JSON，直接照抄进结构化输出不可能构成合法 verdict JSON」（is_repairable 指纹**不**防 B-1，只防 B-2/JsonNonceMismatch）。
5. **结论措辞收敛**：验收标题/报告统一为「单仓 Design 段可用」，或补 1 个 design→work item end-to-end smoke 后再称「全流程」。
6. （确认项）manifest 校验器 `--paired`/digest 校验的 Python 单测覆盖（tasks 2.2 已要求；story 现有 `validate_manifest.py` 的 `--paired` 只在 docstring、无 digest、无 author/reviewer 分离，设计版必须真正实现并测）。

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "按委托完成「目标达成度与验收充分性审查」：严格只读（未修改任何文件、未执行 git 写操作、未派生子代理），范围严格限定在单仓 Design 段评审，未扩展到 aggregate 分支或改动建议的实现。输出为 G1-G5 中文评审。"
    }
  ],
  "changedFiles": [],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {
      "command": "git status --porcelain=v1 && git diff --cached --name-only && git diff --name-only",
      "result": "passed",
      "summary": "均无输出，工作树干净、无暂存文件"
    },
    {
      "command": "read openspec/changes/harden-single-repo-design-weak-models/{proposal,design,tasks}.md + specs/两个 spec.md",
      "result": "passed",
      "summary": "通读冻结契约，提取验收口径与 6 项 requirement"
    },
    {
      "command": "read cadence/analysis-docs/2026-08-22_Design链路加固需求分析.md + /tmp/design-analysis/{code-audit,campaign-design,fewshot-design,terra-review,opus-deepdive}.md",
      "result": "passed",
      "summary": "核对五份背景材料，确认 D1-D6 决策与判例/repair 交互结论的来源"
    },
    {
      "command": "定向阅读 prompts/review.rs、review_repair.rs、review/structured_output.rs、author_revision.rs、revision.rs、prompts.rs、artifact_constraints.rs、lifecycle.rs、runtime_units/clarification/openspec.rs",
      "result": "passed",
      "summary": "逐条核实 13 项修改清单对应的现状代码与调用点（build_review_input 唯一 Design review 入口、is_repairable 死分支、revision.rs:47 唯一反馈入口等）"
    },
    {
      "command": "ls + grep 核对 story-weak-model-campaign/{validate_manifest.py,manifest.schema.json,golden_diff.py,gate-report.md,gate-manifest.json}",
      "result": "passed",
      "summary": "确认 story validator 现状为 warning-only（--paired 仅 docstring、无 digest、无 author/reviewer 分离），佐证设计版「强化校验器」的必要性"
    }
  ],
  "validationOutput": [
    "13 项修改清单与冻结契约（proposal/design/specs/tasks）整体一致；D1 单点注入、D2 内容指纹、D3 无封装判例、D5 golden 不比 ID 集合相等、D6 强化 manifest 均已正确落地到清单。",
    "确认 build_review_input（prompts/review.rs:15-111）是普通单仓 Design 唯一 review 入口，其余 5 处 reviewer_output_contract 调用点（review.rs:285/413/598/668/811）均属 WorkItemPlan 家族，单点注入方案成立。",
    "确认 is_repairable（review/structured_output.rs:38-52）中 NonceMismatch 分支恒不可达（cross_cutting/structured_output.rs 该码 recoverable_value 恒 None），删死分支+载荷指纹覆盖 JsonNonceMismatch/MissingJsonNonce 成立。",
    "确认 validate_confirmed_design_specs（lifecycle.rs:971-1003）仅校验 Confirmed 状态、不重校验 design 内容，design→work item 衔接在本 change 范围外。",
    "识别一处契约措辞不一致：design.md D1「reviewer_output_contract 结果之后」vs 修改清单条目 3「结果之前」（opus-deepdive 推荐「之前」），需统一。",
    "统计口径：0/36 full-chain 失败率 95% 上界≈8.3%；边界 D04/D05 各 0/6 上界≈50%。"
  ],
  "residualRisks": [
    "边界假阳/假阴各仅 6 观测，零门槛不可作为「边界可靠」的 release gate 证据",
    "D05「full-chain 一次成功」与「测试越界必须返修」的口径冲突未在契约中写清",
    "compact_history 在用户反馈返修入口本期暂缓，resume 会话下无客户端历史压缩",
    "design→work item 衔接未验证，若结论称「全流程可用」属过度承诺",
    "B-1 主路径（两处 nonce 都对的照抄）不靠 is_repairable，仅靠判例无封装设计+负例测试，测试断言语义需明确"
  ],
  "noStagedFiles": true,
  "diffSummary": "只读评审任务，无任何代码/测试/文档改动",
  "reviewFindings": [
    "major: 边界观测不足——D04/D05 各 0/6 的 95% 上界 50%，「边界零门槛」只能作触发信号，不能作 release gate；需提量或降级结论",
    "major: D05 的 full-chain 与边界假阴口径冲突，必须写入 campaign README",
    "minor: design.md D1 注入位置「之后」与修改清单「之前」不一致，需统一并补排序断言",
    "minor: B-1 照抄负例测试的断言语义需明确（判例不可构成合法 verdict JSON，而非依赖 is_repairable 拦截）",
    "minor: 结论措辞须与 proposal 范围声明对齐（「单仓 Design 段可用」，非「全流程可用」），或补 design→work item end-to-end smoke"
  ],
  "manualNotes": "最小补充集六项：①D05 口径写入 README；②边界观测提量或降级；③统一注入位置+排序断言；④明确 B-1 负例语义；⑤结论措辞收敛或补端到端 smoke；⑥确认 manifest 校验器 --paired/digest 的 Python 单测覆盖。"
}
```