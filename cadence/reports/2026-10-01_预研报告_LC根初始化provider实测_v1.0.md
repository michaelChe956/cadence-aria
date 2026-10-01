# 预研报告：LC 根初始化 root-cwd 模型 provider 实测（2026-10-01 v1.0）

> 目的：在出 OpenSpec proposal 前，用最小 fixture 验证《LC 根初始化方案设计 v1.1》的核心假设——provider 从 LC 聚合根（非 git、内含多个成员 git 仓）启动时，能否发现并真实使用根级 rules / MCP / skills。
> 结论：**四家 provider 全部成立**；codex 与 kimi 需要两项产品侧可控适配。

## 1. Fixture（按 recipe 真实产物形态）

```text
/tmp/lcspike3/root/          # 非 git 目录
├── AGENTS.md                # 入口：「执行任何任务前必须先读取并遵守 .claude/rules/ 下全部规则」
├── CLAUDE.md                # 与 AGENTS.md 同内容
├── .claude/rules/spike-rule.md     # 规则：回复末尾单独一行输出 [RULE-SEEN]
├── .claude/skills/spike-skill/SKILL.md
├── .mcp.json                # 真实 MCP：@modelcontextprotocol/server-filesystem（spikefs）
├── .codex/config.toml       # 同一 server 的 codex 形态 [mcp_servers.spikefs]
├── member1/  (git 仓)
└── member2/  (git 仓)
```

判定方式：要求 provider「只用 MCP 工具 spikefs 列出成员仓目录」，以**会话结构化日志中的真实工具调用记录**为准（不以模型自述为准）；规则以回复末尾 `[RULE-SEEN]` 为准。

## 2. 结果矩阵

| provider | 规则（入口引用 `.claude/rules`） | MCP 真实调用 | 证据 | 前提 |
|---|---|---|---|---|
| Claude Code | ✅ | ✅ | stream-json init：`mcp_servers=[{name:spikefs,status:connected,source:project}]`；tool_use=`mcp__spikefs__list_directory` | 无 |
| codex 0.155.1 | ✅ | ✅ | `--json` item：`spikefs/list_directory completed` | 根目录 trust + `--skip-git-repo-check` |
| pi | ✅ | ✅ | `mcp({connect:"spikefs"})` → `spikefs_list_directory`（pi-mcp-adapter） | 无（adapter 由 pre-check 全局安装） |
| kimi 2.0.2 | ✅ | ✅ | stream-json tool_calls：`mcp__spikefs__list_directory` | 根目录 workspace trust |

补充探针（前两轮）：
- Claude Code 在**祖先目录为 git 仓**的污染变体下仍锚定根（cwd 优先），未加载祖先 CLAUDE.md。
- codex 在祖先污染变体下加载的是根 `AGENTS.md`；祖先如有 AGENTS.md 会被并入（部署时根祖先保持 git-free 即可规避）。
- Claude Code 根 `.claude/skills` 被发现。

## 3. 对照组（确认前提为必要条件）

| 对照 | 结果 |
|---|---|
| codex 未 trust | 无 spikefs 工具（「当前可用工具里没有 spikefs」） |
| codex 未加 `--skip-git-repo-check` | 非 git 目录直接拒绝启动（`Not inside a trusted directory and --skip-git-repo-check was not specified`） |
| kimi 未 trust（非 git 根） | 无 MCP 工具，退化为 Read 读 `.mcp.json` 文本 |
| kimi 未 trust（git 根对照） | 同上——证明门控是 workspace trust，与根是否 git 无关 |

## 4. 设计须吸收的适配（产品侧可控）

1. **初始化时为 LC 根登记 provider 信任**（codex、kimi）：
   - codex：`~/.codex/config.toml` 追加 `[projects."<lc_root>"] trust_level = "trusted"`（`-c` 命令行覆盖不参与 trust 判定，必须写入用户级配置）。
   - kimi：`~/.kimi-code/workspace-trust/<key>` 写 `{"root":"<lc_root>","trustedAt":<ms>}`；`key = wd_<basename>_<sha256(canonical_root)[:12]>`（由二进制 `trustKey(root)=encodeWorkDirKey(canonicalWorkspaceRoot(root))` 与现网记录 `wd_naruto_4d73fb6dca57` 交叉核实）。
   - 约束：写入用户 home 属跨工作区副作用——只登记该 LC 根、幂等、可撤销（LC 删除/解绑时撤销）、不放宽其他目录；写入失败按 fail-closed 等待面处理。
2. **codex 在 LC 根启动时追加 `--skip-git-repo-check`**。
3. 根级指令入口以 `AGENTS.md` 为主（四家均认），`CLAUDE.md` 为 Claude 兼容副本；`.claude/rules` 经 AGENTS.md 入口引用对 codex/pi/kimi 同样生效（实测）。

## 5. 安装方案核对（零改造）

- Cadence skills 安装 = 机器级：`~/.agents/Cadence-skills` 仓库 + 三层软链（`~/.agents/skills/`、`~/.claude/skills/`、`~/.codex/skills/skills/`）；aria 侧 `CadenceSkillsManager::prepare()`（`src/product/cadence_skills/manager.rs:66-90`）为同一流程的 Rust 实现。
- recipe ②–⑤ 四命令（`src/product/repository_store/types.rs:143-146`）由 provider 在执行目录执行、产物写执行目录。
- 结论：skill 安装/链接/分发与项目目录无关，**LC 根初始化在 skills 侧零改造**——唯一改动是 cwd 指向根；`mcp-configuration` 的 `.gitignore` 步在非 git 根按 skill 自身规则降级提示，不报错。

## 6. 未覆盖项（实施阶段补测）

- 大 LC（数十成员）下的上下文/启动时延成本基线。
- provider 在根 cwd 下对非 target 成员仓的实际写边界（需结合 cross-target 基线检测验收）。
- 多版本 CLI 组合（本次各家单版本）。

测试现场已清理；codex/kimi trust 登记已还原（复核 `lcspike` 残留条目=0）。
