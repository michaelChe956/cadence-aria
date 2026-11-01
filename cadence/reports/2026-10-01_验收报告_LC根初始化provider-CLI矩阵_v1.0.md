# 验收报告：LC 根初始化 四家 provider 真实 CLI 根发现证据矩阵（2026-10-01 v1.0）

> Task 3.3（tasks.md 3.3；REQ-ENV-06、REQ-ENV-08、REQ-REG-14）。实测执行日：2026-10-02。
> 方法与判定标准复用预研报告 v1.0：以**真实 CLI 会话结构化日志中的工具调用记录**为准（不以模型自述、不以单一 `git status` 为准）；本报告不要求会话经本 change 的 LC gateway（gateway 由 `lc-gateway-multi-provider` 交付）。
> trust 登记走**产品 Task 1.3 API 优先**（`HomeBackedProviderTrustRegistry` + Codex/Kimi 真实 home adapter，经临时集成测试驱动），CLI 直测为补充——顺带完成 1.3 落地与真实 CLI 的端到端互证。

## 1. 环境与版本（单版本；多版本组合归 Task 3.4）

| 组件 | 版本 | 说明 |
|---|---|---|
| Claude Code | 2.1.283 | `claude --version` |
| codex-cli | 0.155.1 | 与预研同版本 |
| pi | 0.86.1 | pi-mcp-adapter 经 `pi list` 确认全局安装（`~/.pi/agent/npm/node_modules/pi-mcp-adapter`） |
| kimi | 2.0.2 | 与预研同版本 |
| MCP server | `@modelcontextprotocol/server-filesystem`（npx 本地缓存） | 名称 `spikefs`，仅服务各自 fixture 根 |

## 2. Fixture（临时目录 `/tmp/lc-matrix-33`，不入仓库）

三根同构（预研 recipe 形态 + 入口标记区分设计）：

- `main-root/`——**非 Git** LC 根：`AGENTS.md`（尾行标记 `[AGENTS-ENTRY]`）、`CLAUDE.md`（尾行标记 `[CLAUDE-ENTRY]`，二者均要求先读全部 `.claude/rules/`）、`.claude/rules/matrix-rule.md`（`[RULE-SEEN]`）、`.claude/skills/matrix-skill/SKILL.md`（`[SKILL-SEEN]`）、`.mcp.json`（spikefs）、`.codex/config.toml`（`[mcp_servers.spikefs]`）、`member1/`、`member2/`（真实 `git init`+commit）。
- `ctl-root/`——对照根，与 main-root 同构，**永不登记 trust**。
- `poll-outer/`（真实 Git 仓，含 `AGENTS.md`=`[POLL-AGENTS]`、`CLAUDE.md`=`[POLL-CLAUDE]`、已提交）+ 其下非 Git `inner-root/`（同构 LC 根）——祖先 Git 污染变体。

Marker 互异设计使「入口文件发现」可区分到具体文件（预研双文件同内容无法区分）。fixture digest（`find -type f` 去除 `.git` 后全量 sha256 汇总）：

| 根 | digest |
|---|---|
| main-root | `4b9d68b904ab6947e4615c063a538945ee167a7f8745f91ff022bd8d7801d459` |
| ctl-root | `c7a5d8f6352003e85465ea46b69f8f5e2a969f21ba7ad4e70bbcb7235c582971` |
| poll-outer | `9c51767d05f71fe4b111bfbb96ae092e38a114b53fd3a9538a1d5deef9577c6f` |

统一任务 prompt：「只用 MCP 工具 spikefs 列出当前工作目录的直接子目录名（禁止 shell/文件读取代替）；若存在 matrix-skill 技能按其说明执行；最后按已加载的入口与规则输出全部结尾标记行。」

## 3. trust 前提：产品 Task 1.3 API 登记（REQ-REG-14）

经临时集成测试（`HomeBackedProviderTrustRegistry::new(temp ProductAppPaths, [CodexTrustAdapter::for_home(真实home), KimiTrustAdapter::for_home(真实home)])`，durable facts 落 `/tmp/lc-matrix-33/store`，脚手架不入库）：

| 动作 | provider/root | key | before→after 摘要 | 结果 |
|---|---|---|---|---|
| ensure | Codex @ main-root | `[projects."/tmp/lc-matrix-33/main-root"]` | `sha256:b3fdf…`→`sha256:2aff2…` | Register/LcManaged/Ready，verify trusted=true |
| ensure | KimiCode @ main-root | `wd_main-root_c4849e040ae0` | None→`sha256:3376c…` | Register/LcManaged/Ready，verify trusted=true |
| ensure | Codex @ inner-root | `[projects."…/inner-root"]` | `sha256:2aff2…`→`sha256:5831e…` | Register/Ready |
| ensure | KimiCode @ inner-root | `wd_inner-root_fb4854ce6f1d` | None→`sha256:33152…` | Register/Ready |
| revoke | 双 provider @ inner-root | —（如上） | — | 双 `Removed` ✓ |
| revoke | Codex @ main-root | — | 记录 `2aff2…` vs 现场 `5831e…`/`0d35e…` | `revoke_digest_mismatch` fail-closed（见 §7 观察） |

用户级现场 before/after：`~/.codex/config.toml` before=`b3fdf8dc859b727be697b44cbf1366b76d37377b199c3d40bdc157d03ca279ea`（登记期仅追加本矩阵两块、无既有条目改动；`spikefs`/预研残留=0）；`~/.kimi-code/workspace-trust/` before 5 条用户记录（与本矩阵零冲突）。**实测结束后三处用户级现场全部字节级还原**（codex config 摘要回到 before；kimi trust 列表与 before 逐项一致；`~/.claude.json` 与 before 逐字节一致；`lc-matrix-33` 残留=0）。

## 4. 主矩阵（cwd=各自根；证据=结构化日志原文摘录）

### 4.1 Claude Code 2.1.283（R1/R1b/R11/R12）

- argv/cwd：`claude -p "<prompt>" --output-format stream-json --verbose [--allowedTools 'mcp__spikefs__list_directory' 'Skill']`，cwd=main-root。
- MCP：init 事件 `mcp_servers=[{"name":"spikefs","status":"connected","source":"project"}]`；**真实 tool_use `mcp__spikefs__list_directory`**，tool_result 含 `[DIR] member1`、`[DIR] member2`（R1b）。
- **headless 权限（新钉前提）**：R1（无 allowlist）工具调用被权限系统拒绝（模型自述「spikefs 工具权限未授予」，未取得目录内容）——headless 默认 fail-closed；加 `--allowedTools mcp__spikefs__list_directory` 后调用成功。连接与发现本身**零前提**：R11/R12 证实不写任何审批键（`~/.claude.json` 无 `enabledMcpjsonServers`）时 init 仍 `connected source:project`（`claude mcp list` 显示的 "Pending approval" 仅影响其健康检查展示，不阻塞 headless 会话连接）。
- 入口：`[CLAUDE-ENTRY]` ✓、`[AGENTS-ENTRY]` ✗——**CLAUDE.md 为唯一入口**（有 CLAUDE.md 时不加载 AGENTS.md；生产双文件并存时无害）。
- rules：`[RULE-SEEN]` ✓（经 CLAUDE.md 入口引用 `.claude/rules/`）。
- root-native skills：tool_use `Skill {"skill":"matrix-skill"}`——**原生技能装载与调用** ✓（`[SKILL-SEEN]` ✓）。

### 4.2 codex 0.155.1（R2；对照 R7/R8/R10/R13）

- argv/cwd：`codex exec --json --skip-git-repo-check "<prompt>"`，cwd=main-root（trust 已由产品 API 登记）。
- MCP：JSONL `item.completed` `{"type":"mcp_tool_call","server":"spikefs","tool":"list_directory","arguments":{"path":"…/main-root"}}`，result 含 `[DIR] member1/member2`——**根 `.codex/config.toml` 项目级配置真实发现与调用**（非 LLM 旁证：`codex mcp list` 在根目录显示 spikefs=enabled；用户级 config 无 spikefs）。
- 入口：`[AGENTS-ENTRY]` ✓、`[CLAUDE-ENTRY]` ✗——AGENTS.md 为唯一入口。
- rules：`[RULE-SEEN]` ✓（经 AGENTS.md 入口引用）。
- skills：无原生根级技能装载事件；模型经目录浏览发现 `.claude/skills/matrix-skill/SKILL.md` 并按说明输出 `[SKILL-SEEN]`（文件级发现 ✓；原生根级机制=未见，产品路径维持机器级安装零改造）。
- 良性噪声：首条 `item.completed type=error` 为「Model metadata for deepseek-flash not found」元数据回退，与本矩阵无关。

### 4.3 pi 0.86.1（R3）

- argv/cwd：`pi -p --mode json "<prompt>"`，cwd=main-root；session 事件 `cwd:"/tmp/lc-matrix-33/main-root"`。
- MCP：`tool_execution_start toolName=mcp args={"connect":"spikefs"}`（adapter 连接）→ `toolName=mcp__spikefs args={"tool":"spikefs_list_directory","args":{"path":"…/main-root"}}`——**经 pi-mcp-adapter 读取根 `.mcp.json` 的真实 MCP 代理调用** ✓（与预研链路一致）。
- 入口：`[AGENTS-ENTRY]` ✓、`[CLAUDE-ENTRY]` ✗——AGENTS.md（pi 的 context-files 发现，`--no-context-files` 可关）。
- rules：`[RULE-SEEN]` ✓；member1/member2 ✓（MCP 取得）。
- skills：模型经 MCP `spikefs_read_text_file` 读到 SKILL.md 并输出 `[SKILL-SEEN]`（文件级发现 ✓；pi 原生技能另需 `--skill`/全局目录，根级原生装载未单独验证）。

### 4.4 kimi 2.0.2（R4；对照 R9）

- argv/cwd：`kimi -p "<prompt>" --output-format stream-json`，cwd=main-root（workspace trust 已由产品 API 登记）。
- MCP：stream-json `tool_calls function.name=mcp__spikefs__list_directory`，tool content 含 `[DIR] member1/member2`——**经根 `.mcp.json` + workspace trust 的原生自发现真实调用** ✓。
- 入口：`[AGENTS-ENTRY]` ✓ 且 `[CLAUDE-ENTRY]` ✓——**双入口均加载**（四家中唯一）。
- rules：`[RULE-SEEN]` ✓；skills：读 `.claude/skills/matrix-skill/SKILL.md` 后输出 `[SKILL-SEEN]`（文件级发现 ✓；`--skills-dir` 帮助面显示存在原生 skills 自动发现，本次日志未见原生装载事件——「原生装载」记未单独验证）。

## 5. 变体与对照

| # | 场景 | 命令要点 | 结果（结构化日志依据） |
|---|---|---|---|
| R5 | **claude 祖先 Git 污染**（cwd=inner-root，poll-outer 为 Git 祖先） | 同 R1b | init `cwd=inner-root`、spikefs `connected source:project`（inner-root 自身 `.mcp.json`）、`[CLAUDE-ENTRY]`/`[RULE-SEEN]`/member1/2 ✓；**`[POLL-CLAUDE]` = YES——祖先 CLAUDE.md 被加载**（spikefs 仅服务 inner-root，模型无任何工具可读祖先文件，标记只能来自上下文注入）；`[POLL-AGENTS]` ✗ |
| R6 | **codex 祖先 Git 污染**（cwd=inner-root） | 同 R2 | `spikefs list_directory {"path":"…/inner-root"} completed` ✓、`[AGENTS-ENTRY]`+`[RULE-SEEN]` ✓、**`[POLL-AGENTS]`=YES（祖先 AGENTS.md 并入）**、`[POLL-CLAUDE]` ✗——与预研一致 |
| R7 | codex 缺 `--skip-git-repo-check`（ctl-root，未 trust） | `codex exec --json` | exit=1，stderr：`Not inside a trusted directory and --skip-git-repo-check was not specified.`——启动前拒绝 |
| R8 | codex 有 flag 缺 trust（ctl-root） | `--skip-git-repo-check` | 启动成功、`[AGENTS-ENTRY]`/`[RULE-SEEN]` ✓（入口/规则发现不受 trust 门控），**零 `mcp_tool_call`**，模型自述「当前可用的工具中没有名为 spikefs 的 MCP 工具」 |
| R9 | kimi 缺 trust（ctl-root） | `kimi -p … stream-json` | **`mcp__*` 调用=0**，工具面退化为 Bash/Glob/Read 并读取 `.mcp.json` 文本；`[AGENTS-ENTRY]`+`[CLAUDE-ENTRY]`+`[RULE-SEEN]` ✓——门控=workspace trust |
| R10 | **codex anchor**（cwd=/tmp/lc-matrix-33 非 Git 无配置，`-C main-root`） | `codex exec --json --skip-git-repo-check -C …/main-root` | `spikefs list_directory` completed、`[AGENTS-ENTRY]`/`[RULE-SEEN]` ✓——**发现与 trust 均锚定 `-C` 工作根**（cwd 无关） |
| R11/R12 | claude 无审批键（±allowlist） | 见 §4.1 | init 恒 `connected source:project`——headless 连接零前提 |
| R13 | codex `-c projects."<root>".trust_level="trusted"` 覆盖（ctl-root） | `-c …` + flag | **零 spikefs 调用**，模型自述无该工具——`-c` 覆盖不参与 trust 判定（与预研 §4 一致） |

## 6. 结论矩阵（通过 ✓ / 阻断 ✗ / 未覆盖 ◐）

| 维度 | Claude Code 2.1.283 | codex 0.155.1 | pi 0.86.1 | kimi 2.0.2 |
|---|---|---|---|---|
| AGENTS.md 入口 | ✗（不加载，CLAUDE.md 优先） | ✓ 唯一 | ✓ | ✓ |
| CLAUDE.md 入口 | ✓ 唯一 | ✗ | ✗ | ✓（双入口） |
| root rules（经入口引用 `.claude/rules`） | ✓ | ✓ | ✓ | ✓ |
| root MCP 真实调用 | ✓（`.mcp.json`，source:project） | ✓（根 `.codex/config.toml`；不支持 `.mcp.json`） | ✓（adapter 读根 `.mcp.json`） | ✓（`.mcp.json`+workspace trust） |
| root-native skills | ✓ 原生 Skill 调用 | ◐ 文件级发现，无原生根级机制 | ◐ 文件级发现（原生=用户级/`--skill`） | ◐ 文件级发现（原生装载未单独验证） |
| 祖先 Git 污染 | ◐ 锚根（MCP/rules/cwd 不受染）**但祖先 CLAUDE.md 被并入上下文** | ◐ 根 AGENTS 为主+祖先 AGENTS 并入 | 未测 | 未测 |
| trust/启动前提 | 无（headless 工具执行需产品 tool policy allowlist） | 根 trust（用户级 config.toml）+ `--skip-git-repo-check` | adapter 全局安装 | 根 workspace trust |
| 产品 Task 1.3 trust API 兼容 | n/a | ✓（登记后 CLI 即信任、MCP 放行；对照 R8 锁因果） | n/a | ✓（同左；对照 R9 锁因果） |

**逐 provider 结论**：Claude Code **通过**（连接/发现零前提；祖先 CLAUDE.md 并入为新钉版本行为，部署侧「LC 根祖先保持 git-free」对 claude 同样必要）；codex **通过**（anchor/config 限制=根级 MCP 仅 `.codex/config.toml`、缺 flag 拒启、trust 必须用户级且 `-c` 无效——全部对照锁定）；pi **通过**（前提=adapter 全局安装）；kimi **通过**（前提=workspace trust，产品 API 登记闭环验证）。

**预研基线不回归校验**：预研 §2 四行全部复现（claude rules/MCP/skills、codex rules/MCP、pi rules/MCP、kimi rules/MCP）；§3 三项对照全部复现（codex 未 trust、codex 缺 flag、kimi 未 trust 退化）；§4 适配前提（trust 用户级写入、`-c` 不参与、AGENTS 主入口+CLAUDE 副本）全部复现。**一处版本差异**：预研补充探针「Claude 未加载祖先 CLAUDE.md」在 2.1.283 不成立（R5 加载）——预研未记录 claude 版本号，本次已补版本锚定；属版本行为差异非产品缺陷，影响面=指令上下文并入（不影响 MCP/规则/cwd 锚定）。

**未覆盖（明确阻断，不加 shim）**：多版本 CLI 组合（归 Task 3.4）；pi/kimi 祖先污染变体；kimi/pi 原生技能装载事件；大 LC 时延基线与写边界（预研 §6 既列，归后续任务）。以上未以任何注入/复制/shim 方式伪装通过。

## 7. 观察与移交（供 controller 裁量，非本任务缺陷定性）

1. **codex 共享 `~/.codex/config.toml` 的撤销顺序敏感（P3 建议）**：多根交错 ensure 后，仅「最后登记者」可经 API 撤销（LIFO 第一步成功：inner-root 双 `Removed`）；此前登记的记录因现场摘要与冻结 `after_digest` 不符进入 `revoke_digest_mismatch` fail-closed 等待（main-root 两次尝试分别报 `5831e…`/`0d35e…` vs 记录 `2aff2…`）——语义符合 Task 1.3 设计（跨 LC 竞争收敛 CAS、外部修改 fail-closed），但**多 LC 并存时 codex 解绑操作面会频繁落入人工核验等待**；kimi 键级隔离无此问题（双根均干净撤除）。建议在 3.4/运维面或 1.3 known-limits 补记；本次现场经人工核验后已字节级还原。
2. **claude headless 工具执行默认拒绝**：MCP 服务器连接零前提，但工具调用需 allowlist（`--allowedTools`）或产品 tool policy 注入——gateway change 的 argv 装配须包含该面（R1 拒绝证据已归档）。
3. **入口文件分裂提示（信息项）**：四家入口偏好为 claude=CLAUDE / codex、pi=AGENTS / kimi=双。生产维持「AGENTS.md 主 + CLAUDE.md 副本」双文件即可全覆盖，无额外适配。
4. CLI 自身会话历史（codex `~/.codex/sessions`、kimi/pi 会话存储）因真实运行自然增长，为惰性运行痕迹，未清理、不含本矩阵凭据。

## 8. 现场清理记录

- trust：产品 API 撤销 inner-root（双 Removed）+ 人工核验还原 main-root；`~/.codex/config.toml` 摘要回到 before `b3fdf8dc…`，`~/.kimi-code/workspace-trust/` 与 before 逐项一致，残留=0。
- `~/.claude.json`：移除两条临时项目审批条目与 `skillUsage.matrix-skill` 计数后与 before 逐字节一致（该写入经 R11/R12 证实本非必要，留档为前提刻画过程证据）。
- fixture `/tmp/lc-matrix-33`（含全部原始结构化日志）于报告提交后清除；报告自含关键证据行。
- 临时集成测试 `tests/it_task33_trust_matrix.rs` 不入库（本任务唯一仓库产物为本报告）。
