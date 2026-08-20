# Tasks: relax-legacy-rule-read-gate-for-generation

## 1. 核心函数与文案（REQ-PROMPT-03）

- [x] 1.1 `routing_reference.rs`：新增 `generation_cadence_routing_rules_reference(context)`，Legacy 分支返回按需查阅降级文案，Logical 分支逐字复用 `logical_cadence_routing_rules_reference`
- [x] 1.2 `LEGACY_REFERENCE` 与 `direct_cadence_routing_rules_reference` 保持不变（守卫 coding 路径）

## 2. 生成类注入点切换

- [x] 2.1 `work_item_split_engine/prompts.rs`：outline 与 draft 两条 runtime contract 切换到 `generation_...`
- [x] 2.2 `workspace_engine/prompts.rs` 与 `prompts/revision.rs`：plan outline / revision 注入点切换
- [x] 2.3 `workspace_engine/draft_batch/runs.rs`、`web/workspace_context/prompts.rs`：生成侧注入点切换
- [x] 2.4 确认 `provider_context_builder.rs` 与 `coding_workspace_engine/prompts.rs` 仍调用 `direct_...`，零改动

## 3. 测试与验证

- [x] 3.1 `routing_reference.rs` 单测：generation Legacy 文案断言（含按需语义、无强制完整读取/阻塞字样）；Logical 与 direct 输出逐字一致；direct Legacy 文案回归守卫
- [x] 3.2 更新受影响 prompt 契约测试（parser_prompt 系列、routing_reference_contract、各 prompts 内嵌测试）并全绿
- [x] 3.3 定向回归：routing_reference、parser_prompt、routing_reference_contract 模块测试通过，`cargo fmt` / `cargo clippy` 无新告警

## 4. 收尾

- [ ] 4.1 人工冒烟：在测试项目上用弱模型 provider 跑一次 outline 生成，确认规则文件缺失场景不再阻塞且产物正常进入校验流程
