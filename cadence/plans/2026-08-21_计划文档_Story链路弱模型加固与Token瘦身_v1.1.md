# 计划文档：Story 链路弱模型加固与 Token 瘦身 v1.1

- 关联 change：`openspec/changes/harden-story-pipeline-weak-models`
- 分支/worktree：`.worktrees/feat-b-0808-add-monorepo`（branch `feat-b-0808-add-monorepo`）
- 关联 requirements：`story-pipeline-weak-model-hardening`、`kimi-acp-client-services`、`session-policy-envelope`（delta）
- 工具链约束：`cadence/project-rules/build-test-commands.md`（禁 `-j 1`；定向快反馈 `cargo test --locked --lib <filter>`）；Web 验证 `cd web && pnpm test && pnpm tsc -b`
- Campaign 策略（用户已确认 B）：开发迭代期每组合 5 样本；最终 gate 每组合 20 样本；语料 ≥5 需求形态

## Task → 映射表

| Task | Requirement/Scenario | 修改文件 | RED 测试 | 定向命令 | 完成证据 |
|---|---|---|---|---|---|
| 1.1 | campaign 语料 | 新建 campaign 目录 | 语料可被 diff 脚本消费 | — | 语料 + digest |
| 1.2 | 基线 campaign | —（运行） | — | API 运行 | 基线报告 |
| 1.3 | resume usage 决策 | —（运行） | — | API 运行 | 报告含结论 |
| 1.4 | golden 规范化 | 新建 diff 脚本 | 脚本对基线产出 pass/fail | — | 脚本 + 基线判定 |
| 1.5 | kimi wire fixture | tests fixture 目录 | fixture 可回放 | — | fixture 文件 |
| 2.1 | sentinel 协议（收敛 parser） | `cross_cutting/structured_output.rs`、`workspace_engine/parsers.rs` | 新格式/JSON nonce 不一致/旧格式/envelope 剥离/照抄拒绝/尾随文本/错误码可区分 | `cargo test --locked --lib structured_output` + parsers | 单测全绿 |
| 2.2 | JSON 受限恢复 | 同上 | fence 包裹/多候选/字符串 `{}`/深度 32 与 33/字节临界与 +1/错配括号/escaped quote/fence 外非空/超大 | `cargo test --locked --lib structured_output` | 单测全绿 |
| 2.3 | 消费方原子切换 | review.rs、prompts.rs、review_repair.rs、coding/image/split/fake、web test_controls | 各模块既有测试 + envelope nonce 断言 | 各模块 `--lib <filter>` | `rg` 无旧指令 + 全绿 |
| 2.4 | few-shot | prompts.rs、revision.rs、review.rs | 注入点断言 + reviewer 照抄拒绝 + author 骨架拒绝 | `cargo test --locked --lib prompts` | 单测全绿 |
| 3.1 | severity 后端 | review/structured_output.rs、parsers.rs findings | live 拒绝旧值/回放归一化/未知旧值拒绝/round-trip/impact 幂等 | `cargo test --locked --lib review` | 单测全绿（live 解析路径无 6 档枚举分支） |
| 3.2 | severity Web | web api/types、store-types、ReviewVerdictEntry、ProviderStreamEntry | web 测试 | `cd web && pnpm test && pnpm tsc -b` | 全绿 + `rg strong_recommend_fix web` 无残留 |
| 4.1 | 摘要生成器 | 新建 history compaction | ≥3 轮压缩/失败退回/未关闭定位 | `cargo test --locked --lib workspace_engine` | 单测全绿 |
| 4.2 | 三入口接入 | prompts.rs、revision.rs、review.rs | 三入口单测 + 字符数代理断言 | 同上 | 单测全绿 |
| 5.1 | 能力声明+分发 | kimi session.rs | initialize payload + 并发用例 | `cargo test --locked --lib kimi` | 单测全绿 |
| 5.2 | 终端实现（含 grammar 白名单 + OS 隔离） | kimi session.rs（+TerminalManager） | fixture 全链路/并发/清理/越界与执行型 flag/option-smuggling/资源临界/bwrap 缺失拒绝/bwrap 隔离效果/FD 锚定/fsmonitor 探针 | `cargo test --locked --lib kimi` + 真机 | 真机 bash/grep 成功 |
| 5.3 | fs 实现 | kimi session.rs | 正常/越界/symlink/新建父目录 | `cargo test --locked --lib kimi` | 单测全绿 |
| 5.4 | 权限与角色 | approval_bridge + kimi | 三角色用例 | `cargo test --locked --lib kimi` | 单测全绿 |
| 6.1 | MCP envelope | kimi session.rs + policy | 注入/漂移(新会话+superseded)/无 bundle | `cargo test --locked --lib kimi` + 真机（可选） | 单测全绿 |
| 7.1 | 全量 | — | — | 四件套 + openspec strict + web | 全绿 |
| 7.2 | gate campaign | —（运行） | 20 唯一 case_id baseline/revised 配对 + validate_manifest.py 校验 + golden 禁止差异 fail | API 运行 + `python3 validate_manifest.py` | 报告 + gate 判定（成功率/token/golden 全达标） |

## 阶段顺序与退出条件

1. **阶段 0（tasks 1.x）**：基线 + fixture + golden + resume 结论。退出：1.1 语料 digest、1.2 基线报告+manifest、1.3 resume 结论、1.4 golden diff 脚本可运行、1.5 wire fixture 落盘，全部满足后方可进入阶段 1。
2. **阶段 1（tasks 2.x/3.x）**：P0 协议/恢复/示例/severity 全栈，单工作包原子切换。退出：2.3 `rg` 无旧指令 + 2.x/3.x 定向测试全绿。
3. **阶段 2（tasks 4.x）**：P1 窗口。退出：4.2 三入口单测全绿；若 1.3 判定需 fresh 切换，一并落地。
4. **阶段 3（tasks 5.x）**：P2a kimi。退出：5.2 真机 bash/grep 成功记录。
5. **阶段 4（task 6.1）**：P2b MCP。退出：单测全绿。
6. **阶段 5（tasks 7.x）**：全量 + gate campaign。退出：7.1 全绿；7.2 任一组合 <19/20 或 token gate 未达则任务不得勾选。

## 提交建议

- 阶段 0 报告/fixture/golden 单独 docs commit；阶段 1 单 commit（原子切换，避免中间态）；阶段 2/3/4 各自 feat/fix commit；阶段 5 报告 + 勾选 tasks.md。
