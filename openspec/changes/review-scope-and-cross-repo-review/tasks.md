# 高层工作包：多仓评审范围收敛与 coding 跨仓最终评审

本文件只记录未来 apply 的高层工作包及其契约映射；精确文件、命令、测试与提交切分由后续 `cadence/plans/` 的已确认 Plan 展开。本次仅创建契约工件，不执行、不勾选实现任务。

## 1. 形态门禁与单仓不变基线

- [ ] 1.1 落地唯一形态判定（`IssueRecord.logical_codebase_id`）并让全部新逻辑只挂多仓分支，单仓走原实现；验收每条新增 requirement 的单仓场景均可观察（窗口记录与 window id、早返回路径、基线 fail-closed、prompt 与摘要字节、provider 启动次数），且既有单仓用例不改一行即全绿（REQ-PLN-11、REQ-ENV-13/14、REQ-GFR-CR-01/02）。

## 2. 多仓评审范围与材料（规划侧）

- [ ] 2.1 实现多仓评审范围取 issue 选定仓库集合、移除 primary 概念与锚定事件；验收 2 仓 issue 的 reviewer prompt 无 primary 措辞、无主成员事件，集合解析失败显式阻断（REQ-PLN-11）。
- [ ] 2.2 实现仓库清单全量注入与 500 KiB 绝对上限阻断；验收 3 仓全量命中、无 omitted/truncated 标记，超限返回 `planning_material_over_budget` 且零 provider 启动（REQ-PLN-12）。
- [ ] 2.3 实现材料块进入稳定注入面并在 3 轮以上长会话压缩后仍完整可见；验收压缩后 prompt 仍含完整清单与逐仓实况，单仓 prompt 字节不变（REQ-PLN-13）。

## 3. 逐仓实况与基线（规划侧）

- [ ] 3.1 实现逐仓五行实况（职责/关键目录/入口/构建测试命令/基线）与缺项「未识别」；验收 Rust+Vue 两仓夹具出现两段实况（REQ-PLN-14）。
- [ ] 3.2 实现逐仓基线取 HEAD 分支并对 detached HEAD / 空仓降级为本仓「基线未识别」；验收该仓降级而其余仓继续、单仓仍 fail-closed（REQ-PLN-14）。

## 4. 多仓判定与容错（规划侧）

- [ ] 4.1 实现多仓 must-fix 四类必查（需求漏项 / 仓库选择不合理 / 验收不可判定 / 跨仓影响缺失）；验收多仓规则段含四类、单仓文案逐字不变（REQ-PLN-15）。
- [ ] 4.2 实现多仓 finding 完整性门（缺 `evidence` 或 `required_action` 即不收，`needs_human` + `malformed_findings`）与旧轮 message/evidence 保留；验收负例夹具与单仓照旧行为（REQ-PLN-16）。
- [ ] 4.3 实现多仓 splitter 分层目录摘要与 `repository_structure_unavailable` / `repository_structure_empty` 显式失败；验收多层仓出现二级条目、单仓摘要逐字不变（REQ-PLN-17）。

## 5. 多仓评审身份与不建窗（策略侧）

- [ ] 5.1 实现每会话唯一身份承载 reviewer 与 author 的多仓身份（含存量会话补生成一次、随会话删除解绑）；验收同会话跨轮摘要域恒定、单仓窗口编号与摘要不变（REQ-ENV-13）。
- [ ] 5.2 实现多仓文档环节评审跳过评审窗、基线快照、对账与恢复，材料缺失改 `plan_review_material_unresolved`；验收多仓无窗口记录与窗口错误码、单仓窗口与零锁恢复不变（REQ-ENV-14）。

## 6. coding 跨仓最终评审主体

- [ ] 6.1 条件化停用多仓每仓 internal review 与 GroupFinalReview（含删除无生产调用的执行入口），保留 code reviewer 门与组形态；验收多仓 InternalReviewer provider 启动次数为 0、单仓链路逐项不变（REQ-GFR-CR-01）。
- [ ] 6.2 实现存量 `stage = internal_pr_review` attempt 的形态分派；验收多仓零 provider 走 readiness→FinalConfirm、单仓原路径完成、阶段字符串不变（REQ-GFR-CR-02）。
- [ ] 6.3 实现 issue 级跨仓评审恰好一次与按 issue 幂等，并保证不新增 attempt 阶段；验收三仓一次记录、重复触发不重复、attempt 的 `stage` 不变（REQ-GFR-CR-03）。
- [ ] 6.4 实现两处收口钩子的触发接线与「全部开发完成」判定；验收 group 形态与 WorkItem scope 形态各触发一次，per-attempt 收口逻辑不变（REQ-GFR-CR-03）。
- [ ] 6.5 实现逐仓 diff 分段材料（稳定排序、与 code reviewer 同源口径、超预算显式标注）与 provider 回退链；验收三仓三段、回退不 Skipped、双缺 Skipped（REQ-GFR-CR-04）。

## 7. advisory、留痕与来源枚举

- [ ] 7.1 落实 advisory 红线：结论不进交付门/采信门/返工/分诊、不改 attempt status/stage、不建 pending gate、不开窗、不自动重试；验收要求修改与阻塞两种结论下用户仍可确认、无任一分流指令（REQ-GFR-CR-05）。
- [ ] 7.2 实现只读呈现面（issue 级记录、新待办 kind、最终确认卡片两个可选字段）且不改 ready/blocked 判定；验收卡片显示警告、通过结论不投影提醒、单仓无该呈现（REQ-GFR-CR-05、REQ-GFR-CR-06）。
- [ ] 7.3 实现已知风险确认留痕（时间 + 评审记录 id + 结论 + findings 摘要）与幂等；验收留痕 JSON 不含确认人与通道键、同键不重复、单仓无留痕（REQ-GFR-CR-06）。
- [ ] 7.4 实现 `issue_cross_repo_review` 来源取值：登记严格字符串表、兜底与既有取值兼容、新成员不进 attempt stage；验收跨仓 findings 取值、省略兜底、存量读出不变、阶段序不变（REQ-GFR-CR-07）。

## 8. 契约一致性审查与归档硬门

- [ ] 8.1 全量复核实现与本 change 三个 delta 的一致性，并对每条 requirement 的单仓不变场景取证；验收 strict 校验通过、单仓回归用例全绿、未实现工作包不勾选（全部 requirements）。
- [ ] 8.2 【归档硬门】若 `provider-desandbox-full-open` 先归档，本 change 归档前 MUST 先修订主规范 `REQ-ENV-REVIEW-B` 的适用范围（以 MODIFIED 补入「多仓文档环节材料评审不进入 B 档窗口」），使主规范不出现与 `REQ-ENV-14` 并存的自相矛盾；交付修订记录与两个 change 的 strict 结果，否则禁止归档（REQ-ENV-14、REQ-ENV-REVIEW-B）。
- [ ] 8.3 【重叠复核】归档前重跑在途 change 的 header 比对，确认 `logical-codebase-aggregate-planning` / `session-policy-envelope` 的在途 MODIFIED、ADDED 条款未被本 change 覆盖；有新增重叠时先对齐再归档（三个 delta 全部）。

## 后续工作流

- 契约审阅通过后才展开 `cadence/plans/` 的实施 Plan 与 apply；本次工件创建不代表任何源码行为已实现。
- 非目标（不在本 change 实现）：多成员对账域与集合屏障、物理 no-exec、verdict 结构错误静默降级的修复、severity 与 revision 轮规则调整、跨仓评审重试与重评入口、两处 GroupFinalReview prompt 的文本合并。
