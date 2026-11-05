# 设计：聚合初始化 root recipe 对 provider 无关

> 本文件是 OpenSpec 层的**决策记录**：背景 / 决定 / 理由 / 代价 / 冲突裁决。所有落点均已在本仓（`.worktrees/bugfix-b-11-02`）核对过行号与语义。
> 精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`；本文件不写实施步骤。

## 0. 现状事实（决策的事实基础，全部已核对）

| # | 事实 | 位置 |
|---|---|---|
F1 | root-recipe 相位准入**硬拒非 ClaudeCode** | `src/product/logical_codebase/production_policy_resolvers.rs:366-371`（`provider.provider_type != ProviderRefType::ClaudeCode` ⇒ `PROVIDER_ROOT_RECIPE_REQUIRES_FIXED_CLAUDE`） |
F2 | 该准入其余三道检查（顺序 fail-closed）：重核验通道在场 → 版本钉定 → 凭据对 durable Running operation 重核验 | 同文件 `:373-404` |
F3 | 选择器按 owner 顺序 `Pi > KimiCode > ClaudeCode > Codex`，**只**判 L1 命令可用 + L2 capability Confirmed，**不感知** root-recipe 准入 | `src/product/logical_codebase/provider_selection.rs:249+`（`decide_selection` 只调 `launch_cell_state`） |
F4 | 四条 recipe 命令是**一处写死的裸字面量** | `src/product/repository_store/types.rs:140-148`（`RepositoryInitializationStepKind::command()` 返回 `&'static str`） |
F5 | prompt **逐字透传**到各 adapter，**无任何按 provider 翻译的层** | `src/product/logical_codebase/coordinator_provider_turn.inc.rs:196-207` → Claude `src/product/claude_code_provider/mod.rs:615-617`、Codex `src/product/codex_provider/session.rs:480`、Kimi `src/product/kimi_code_provider/session.rs:472-474`、Pi `src/product/pi_provider/session.rs:280-282` |
F6 | 代码里**不存在** skill 调用语法映射；`src/` 全树 grep `skill:` / `/skill` / `$cadence-init` ⇒ **0 命中**；git 全历史亦无生成痕迹 | 全仓检索实测 |
F7 | **没有「命令未被 provider 识别」的失败码**：命令不被识别 ⇒ turn 正常 Completed 但**零文件变更**，或 provider 自报 failure 归入 `provider_status_failed` | `coordinator_provider_turn.inc.rs:565-574`（cause 子码清单）、`:296-301`（Failed 事件） |
F8 | 四条 skill 命令名固定 | `src/product/logical_codebase/environment_probe.rs` 的 `AGGREGATE_SKILL_COMMANDS = ["pre-check", "rule-config", "mcp-configuration", "project-rules-examples"]` |
F9 | skills 分**三层**链接，探测要求四条在 shared / codex / claude 三层都解析到 `SKILL.md` | `environment_probe.rs:121-140`；roots 定义 `src/product/cadence_skills/paths.rs:34-42`（shared=`~/.agents/skills`、codex=`~/.codex/skills/skills`、claude=`~/.claude/skills`） |
F10 | skills 源目录的**插件命名空间就是 `cadence-init`** | `paths.rs:20-21`（`source_root = ~/.agents/Cadence-skills/cadence-init/skills`） |
F11 | 本机三层实况：四条命令在 shared / codex / claude 三层**都在** | 实测 `~/.agents/skills`、`~/.codex/skills/skills`、`~/.claude/skills` |
F12 | Claude 的裸形态**已被真机验证**逐条执行成功 | `cadence/reports/2026-10-02_验收报告_LC根初始化单仓对照与终验_v1.0.md:9` |
F13 | pi 把 skills 暴露为**名为 `skill:pre-check` 的 tool** | `cadence/reports/provider-validation-round/dialect-transcript-rep1.txt:11` |
F14 | 单仓路径**同样直接消费** `command()` 的裸字面量 | `src/product/repository_store/initializer.rs:81-85` |
F15 | 冲突被**实施时已知并刻意绕开**：实施 notes 与两处测试夹具都写明「四家全 Confirmed 会让选择器选中 Pi 并在 recipe 相位被拒」，夹具只种 ClaudeCode | `cadence/notes/2026-11-04_实施_provider能力核验全局化_v1.0.md:251-255`；`src/web/handlers/aggregate_initialization/tests_parts/l2_global_capability.inc.rs:28-32`；`tests/web_logical_codebase_entrypoints/capability_fixture.rs:101-105` |
F16 | 「root recipe 能力证据不得被普通会话复用」的隔离契约 | `openspec/specs/logical-codebase-registration/spec.md:274-276` |
F17 | root-recipe 相位的立身之本（主 spec 原文）是**凭据**，不是 Claude 专属 recipe 事实 | `openspec/specs/logical-codebase-registration/spec.md:266`（「持有由 durable Running aggregate operation 派生且重验有效的 bootstrap credential」） |

## 决策 1：放开 root-recipe 相位的 Claude-only 硬拒

**决定**：删除 F1 的硬拒；准入的其余三道检查（F2）**一条不减**；凭据缺失 / 版本漂移 / 凭据重核验失败**继续 fail-closed**。

**理由**：

1. **现行 spec 本来就要求不固定 Claude**（REQ-BOOT-03「recipe provider SHALL NOT 固定为 Claude Code」、REQ-BOOT-07 的 owner 顺序场景），实现是**上一代契约的残留**（`2026-11-02-lc-root-initialization` change 的当期设计 + `2026-10-17-lc-gateway-multi-provider` 的 Non-Goal），二者必须对齐 —— owner 2026-11-05 裁定选实现侧对齐 spec 侧。
2. **立身之本是凭据不是 Claude**（F17）⇒ 硬拒 provider 类型没有语义支撑。
3. **失败位置太靠后**：硬拒发生在第一个 turn 的 spawn 前预检（F1 位于 gateway validate / spawn 前复验路径），导致 operation 已创建、durable Running，却零 spawn、后续 turn 与索引构建全未启动 —— 失败面只说「provider 的第一步没完成」，看不出真实原因。

**代价**：

- 非 Claude provider 真的会去执行那四条命令 ⇒ 它**能不能执行**没有既有证据（F12 只证明了 Claude）。这是 owner 选择 B 时已知的代价；决策 2 + 决策 3 就是为了让这个代价**变得可见而不是静默**。

## 决策 2：命令按 provider 的 skill 调用语法生成

**决定**：四条命令的**调用前缀**按 provider 生成（REQ-BOOT-16 的映射表），**命令名**与**参数**逐字不变。

**理由**：

1. F4/F5/F6 证明当前是**一处写死、发给四家同一串** ⇒ 对 Pi/Codex/Kimi 就是错的形态。
2. owner 提供的形态与代码事实互相印证：Claude 的裸形态已被真机验证（F12）；pi 把 skill 暴露为 tool `skill:<name>`（F13）⇒ `/skill:<name>`；Codex 的插件命名空间就是 `cadence-init`（F10）⇒ `$cadence-init:<name>`；shared 层由 pi 与 Kimi 共用（F9/F11）⇒ Kimi 与 Pi 同形。
3. **形态不对 ⇒ 失败静默**（F7）⇒ 只做决策 1 会把「干脆的拒绝」换成「看不懂的假成功」。

**代价**：

- 生成点是 `types.rs` 的 `command()`（目前 `&'static str`、无 provider 参数）⇒ 签名要改；两个调用点（F5 的 `recipe_commands`、F14 的单仓路径）都要改。
- **单仓路径（F14）也会跟着变** —— 它现在同样把裸 Claude 形态发给四家。单仓初始化走的是同一套 `command()` ⇒ 形态生成放这里，单仓自动受益；这是**副作用即收益**，不是扩范围。

## 决策 3：root recipe turn 零变更必须显式失败

**决定**：root recipe 的 step 结束时若 receipt 判为零动作 ⇒ 判该 step 失败、保留可诊断失败事实；SHALL NOT 记为成功。

**理由**：F7 —— 「命令未被识别」在 provider 侧**没有错误码**，表现是零变更的正常完成。决策 1 放开门之后，这条静默路径的暴露面从「Pi 跑不了」扩大到「任何 provider 的形态/参数错、skill 缺失、CLI 行为变化」。

**代价**：可能误伤**合法零变更**的 step ⇒ 只对 root recipe 的四个 step 生效，且 SHALL 复用既有 receipt auditor 的零变更判据（不新增第二套判定）。

## 决策 4：归档顺序

**决定**：本 change MUST 在 `globalize-provider-capability-verification` **之后**归档。

**理由**：两个 change 都 MODIFIED `non-interrupt-repository-bootstrap` 的 REQ-BOOT-03 / REQ-BOOT-07；delta 以**全文替换**语义应用，先归档者的措辞会被后者覆盖。本 change 的 delta 已按「机器级」措辞书写（即以 globalize 的结果为基础），故必须后归档。

## 5. 明确不做的事（防越界）

- 不改 capability 记录结构、不改 provider 优先级、不改安装机制
- 不改「不点击不运行探针」契约、不改五步结构
- 不做「非 Claude provider 的 recipe 证据 per-provider 化」（方案 C，独立立项）
- 不改 root-recipe 相位对普通会话的隔离契约（F16）
- 不新增第二套「有无变更」判定逻辑
- 不在本仓生成 `$cadence-init:` 之外的任何新 skill 命名空间
