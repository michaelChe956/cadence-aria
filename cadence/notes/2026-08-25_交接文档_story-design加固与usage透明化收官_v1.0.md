# 交接：story+design 加固全线收官 + usage 透明化（2026-08-25）

> 一句话：三个 change 全部完成归档并推送；story 真实基线 40/40、design 真实基线 48/48；usage 采集（协议层+本地文件）与气泡展示已上线并真机验证。

## 仓库与工作区

- 仓库：/home/michaelche/workspace/github/cadence-aria，工作在 worktree：`.worktrees/feat-b-0808-add-monorepo`（分支同名，已推送 origin 同步）
- 当前分支 HEAD：含全部收尾提交（最后为 usage-transparency 归档）
- 另一会话在同一 worktree 有未暂存的 web 测试改动（web/src/components/lifecycle/IssueLifecycleWorkbench*），**不要动**，它自述是样式修改

## 三个已归档 change

| Change | 归档 | 实质内容 |
|---|---|---|
| harden-story-pipeline-weak-models | archive/2026-08-22-* | sentinel nonce 协议/severity 三档/滑窗/kimi ACP client_services（terminal/fs/bwrap/MCP 受控注入）；真实基线 3 组合 10/10 |
| harden-single-repo-design-weak-models | archive/2026-08-24-* | design 判例 few-shot（无封装三判例）/repair 三防线/四入口契约/结构回归矩阵/campaign 基础设施；真实基线 48/48（含 codex 第四组合） |
| usage-transparency | archive/2026-08-25-* | UsageReport 事件管道+四家采集（claude 协议层；pi/codex/kimi 本地会话文件兜底）+气泡标题行琥珀色 token 标签 |

## 完整事件链（环境灾难级排查，后续会话最需要的部分）

design campaign 初测全军覆没，根因链共五层（全部已修）：

1. **锚定缺失（最根本，用户指出）**：会话 `repository_path` 从未设置（socket.rs:190 在 WS 连接时设置，但 campaign 走的 API+WS 组合里 generate 创建的 record 无此字段，working_dir 退化为后端进程目录 cadence-aria 而非目标仓库 naruto）。**产品逻辑：issue 绑定 repository（naruto 单仓），所有 provider 必须以 naruto 为根执行。**
2. **kimi 完成信号被误杀**（29586de6）：`KimiClientServiceDispatcher` Drop 在会话正常完成时 cancel 了共享引擎 run token → Completed 事件被竞态吞掉 → 误标「运行已中止」。修复：Drop 只 cancel `child_token()`。
3. **fs 绝对路径一刀切**（1cbeed53）：`validate_relative` 把一切绝对路径当越界（组件 RootDir 即拒），没跟授权根比对。修复：组件级前缀比对（Path::starts_with 防前缀混淆）+剥离转相对。
4. **Orchestrator 角色全拒**（bdecb32d）：policy.rs 的 `_ => Deny` 把 author 角色的 kimi 客户端服务全拒，与项目规则 read-gate（读不到规则必须暂停问用户）死锁。修复：Orchestrator 授 FsRead+Terminal（沙箱/白名单照旧），FsWrite 仍拒。
5. **批量采集瞬态**：后端重启后启动窗口期三路冲击致 pi/kimi 批量全灭（**规避纪律：后端重启后等 70s+ 再采**；且批量失败无法稳定复现，单跑全正常）。

期间还修了：
- campaign driver 两处：choice 优先「跳过/继续」（kimi 工具阻塞死循环）→ 后改为**仅工具阻塞类 choice 才优先跳过**（否则劫持设计选型如 JSON vs MessagePack）；kimi 本地 usage 读取的两处（200ms 重试防落盘时序 + `session_` 前缀剥离防双重前缀）
- codex parse 死代码 fallback（"total_token_usage.prompt_tokens" 扁平 key → 嵌套 pointer）

## 关键产物位置

- SDD ledger（两个 change 全过程）：`.superpowers/sdd/2026-08-21_计划文档_Story链路弱模型加固与Token瘦身_v1.1/progress.md` 和 `.superpowers/sdd/2026-08-22_计划文档_单仓Design弱模型加固_v1.0/progress.md`
- design 会审报告集：`cadence/notes/design-harden-reviews/`（8 份，判例定稿文案在 fewshot-design.md）
- design 成绩单：`cadence/reports/design-weak-model-campaign/gate-report-final.md` + gate-manifest-final.json（48 样本）
- story 重测成绩：/tmp/story-retake（40/40，/tmp 数据不持久——如需复核要重跑）
- design 语料/golden/driver：`cadence/reports/design-weak-model-campaign/`（corpus 6 形态+fixture+digest、golden normalizer、强化 manifest 校验器、run_campaign.mjs + story_run_campaign.mjs）

## 环境运维要点（血泪经验）

- **后端启动**：`nohup target/debug/aria web --workspace . --host 127.0.0.1 --port 4317 & disown`（worktree 根目录跑）；**禁止复合命令里夹带 setsid/nohup 启动**（容易被静默吞掉）。**重启后等 70 秒以上再跑采集**。
- 前端：`cd web && pnpm dev`（5173）。
- **复合命令含 `(setsid nohup ... &)` 经常无输出静默失败**——拆成单条逐步执行最稳。
- kimi 登录会过期（报 "Kimi Code is not logged in"，kimi login 设备码）。
- campaign 驱动：story=`cadence/reports/design-weak-model-campaign/story_run_campaign.mjs`，design=`run_campaign.mjs`；输出目录必须分开（两 driver 目录结构相同，混用会互相覆盖）。
- cargo 规则：禁 `-j`；定向测试必带 `--lib`；文件行数红线 1200（large_file_guard 守门，含 web）。
- 模型前缀：bingqi/glm-5.3（审查稳）、my-openai/gpt-5.6-terra（安全类 prompt 会被 usage policy 拦）、tydic-openai/deepseek-v4-pro（慢易 hang）、anthropic-kiro/claude-opus-5（token 池易耗尽，fork oracle 已坏）。

## 真实基线成绩（修正环境后）

**Story（40 样本）**：claude 10/10（均343s）/ codex 10/10（均167s）/ kimi 10/10（均137s）/ pi 10/10（均71s）；结构完整 39/40；choice 落章 40/40；review 16 个 revise 全合理（12=待确认项格式策略）。

**Design（48 样本）**：pi/codex/claude 各 12/12、kimi 11/12（唯一失败为 driver choice 劫持，修复后重跑通过→实际 48/48）；D04 抽象追踪零误伤；D05 越界**题型反转**（判例送达后 author 主动写排除声明，reviewer 判定全对）；D03 决策落章 8/8（旧环境盲写问题自然修复）。

## usage 采集现状（真机验证）

| Provider | 途径 | 状态 |
|---|---|---|
| claude | 协议层 result.usage | ✅ 出数（input 89k/output 8.9k/cache 231k 实测） |
| pi | 本地 sessionFile 尾部 usage（协议 cost 恒 null） | ✅ 实现+出数 |
| codex | 协议 turn/completed 优先 + 本地 rollout token_count 兜底 | ✅ 实现 |
| kimi | 本地 wire.jsonl usage.record（含 subagent 汇总） | ✅ 出数（746/1183/45k 实测） |

气泡：标题行琥珀色标签 `作者 · Kimi Code ┃ Tokens 输入 6,406 · 输出 4,797 · 缓存 43,392`（MessageGroupView + ChatEntryContainer titleSuffix；ProviderStreamEntry 同方案；历史回放从 detail execution_events 恢复；初始加载拉全部已完成节点 detail）。

## 遗留事项（终审 triage 后的全部清单）

**建议优先**：
1. **workitem/coding 段全流程核查**——总目标（issue→story→design→workitem→coding→交付）最后两段从未实测
2. **issue 编号并发竞态**——并发创建撞号（codex/kimi 曾撞 issue_0035；后端 next_id 非原子）
3. **terminal 并发 flaky 测试**（≥3 个：output_is_capped/concurrent_streams_share_budget/timeout_terminates_process_group，单跑全过）
4. **kimi MCP gateway 生产接线**（with_mcp_bundle* 生产不可达，mcpServers 恒 []）
5. **compact_history 在反馈入口启用评估**（当初暂缓等 usage 数据——现在数据有了，可以决策）

**可排期**：
6. kimi 官方 ACP usage（issue kimi-cli#2394 / kimi-code#1855）——届时可切协议途径，本地文件采集是过渡
7. supervised 无沙箱 argv-form grammar bypass（story 时代已知安全残留）
8. 恢复路径 nonce 不绑定签发值（story 终审 Important 转遗留）
9. coding 子系统 severity 旧值兼容（coding_models/review.rs:94）
10. 杂项 minor：指纹 partial 照抄理论绕过 / fs canonicalize 失败错误语义 / 判例 ID 三处同源靠约定 / abort+Completed 理论竞态残留

## 下一步建议

按序：① workitem/coding 段实测核查（沿用 story/design 的 campaign 模式）→ ② issue 编号竞态修复 → ③ compact_history 启用决策（usage 数据已到位）。每个都从「真实环境单案例先行」开始——别再批量撞环境坑。
