# WP1 campaign 驱动器族未测区核对表（campaign-untested-audit）

> change ③ retire-legacy-workitem-protocol / WP1.4（REQ-RET-04）。范围：`cadence/reports/workitem-coding-campaign/` 全部 .mjs 逐区核对，判定=本轮重测/既有验证轮仍引用→登记引用面保留；用途已被取代且零引用→标记退役并登记（不静默删除、不虚编用途）。

## §1 驱动器族逐区核对表（2026-09-19 实读）

| 文件（行数） | 测试覆盖（实读导入锚） | 未测区 | 引用面（谁还在用） | 判定 |
|---|---|---|---|---|
| `workitem_run_campaign.mjs`（3118） | 策略内部件有覆盖：`campaign_driver_policies.test.mjs:46`、`stage3_campaign_driver.test.mjs:27`、`coding_usage_collection.test.mjs:12`（collectUsageByRole） | **E2E 主流程**（prepare→generate→evaluate→Confirmed→handoff 全链）无专属测试 | 本轮 T1 重测 workitem 腿（campaign/codex、campaign/pi）+ 阶段 2/专项测量轮先例 | **保留**——本轮 actively 引用；E2E 由真实跑验证（本轮 2-3 案例即实测） |
| `coding_run_campaign.mjs`（1490） | `coding_amendment_policy.test.mjs:17/:34`（44 用例）、`coding_usage_collection.test.mjs:11`（codingUsageResult）、`campaign_driver_policies.test.mjs:49`（fixtures 导入） | **E2E 主流程**（handoff 回读→group attempt→coding WS 自动驱动）无专属测试 | change ① T3 验证轮（kimi 0556a410 真实收口中）+ 3.6 convergence-36（f6-amendment 等）+ change ② 多仓 coding（C2 进行中，Q2-b 两期将用） | **保留**——引用面在案且后续 change 仍需 |
| `extract_golden_findings.mjs`（229） | 零测试 | 全部（抽取脚本本体） | 全仓运行引用=**零**（仅历史计划文档 `2026-08-26_计划文档_WorkItem策略层止血（阶段1）_v1.1.md` 记载其产出用途，及本 C3 计划作为未测区例点名；无任何 .mjs/脚本/Rust 消费方 import） | **标记退役并登记**——用途已被取代：其产物 `src/product/work_item_plan_policy/fixtures/golden_findings.json`（14 items，Rust tests_classify.rs/prompt_contract/reviewer_finding_channel 三面消费并钉数）已固化入仓，再生成需求为零。REQ-RET-04：不静默删除，退役处置留 controller 终裁后统一执行 |
| `stage3_campaign_fixtures.mjs`（167） | 作为 fixture 源被 `stage3_campaign_driver.test.mjs:41/:435/:733/:784` 导入 | —（fixture 库非独立驱动器） | stage3 测试族 | **保留** |
| `campaign_driver_policies.test.mjs`（1337）/ `coding_amendment_policy.test.mjs`（1414）/ `coding_usage_collection.test.mjs`（103）/ `stage3_campaign_driver.test.mjs`（916） | 自身即测试（合计 50+ 用例） | — | CI/验证轮 | **保留** |
| `fixtures/`（golden_findings 源材料、digests.txt、07-fullstack-levels.md 等） | 由 `workitem_run_campaign.mjs` 启动校验（digests.txt）与 Rust golden 测试（编译入仓副本）消费 | — | 本轮重测启动校验+Rust 三面 | **保留** |
| `README.md`、`reports/` | SOP 文档+历史产物 | — | — | **保留**（历史归档文档，残留归零断言豁免类） |

## §2 coding_run_campaign 场景清单对照（assignment 指定顺带核）

`coding_run_campaign.mjs` 自动行为场景（README §2+「已知行为与失败关闭」）逐项对照：`coding_permission_request` 自动批准、`coding_choice_request` 首选项/关键词策略、`coding_gate_required`/未知 gate 停止自动响应、Plan Repair 停止、未知协议消息停止、`review_complete needs_human` 失败关闭、`review_decision_response` options 策略、`provider_select_request` 角色策略、usage 双键采集（usage_by_role+usage）——其中策略内核由 `coding_amendment_policy.test.mjs`（44 用例）+`coding_usage_collection.test.mjs`+`campaign_driver_policies.test.mjs` 覆盖，E2E 形态由 change ① T3 真实跑（kimi 0556a410）验证中。**本轮 T1 重测不使用 coding 腿**（REQ-WSC-07 必选判据仅 workitem 腿，C3 计划锚点速查 :63 明示）→ 不构成本轮未测风险，登记保留待 change ②。

## §3 结论

- 退役标记：1 项（`extract_golden_findings.mjs`——零引用+用途被固化 fixture 取代）；处置=登记待终裁，不删除。
- 保留：7 项（2 主驱动器 E2E 靠真实跑验证+引用面在案、1 fixture 库、4 测试文件、fixtures/、README/reports）。
- 本核对表即 REQ-RET-04 campaign 未测区销账依据。
