# 设计：修复「页面点安装没有效果」与「聚合索引 codegraph 截断报错」

> 依据：`cadence/plans/2026-11-13_计划文档_修复Provider安装无效果与聚合索引截断_v1.0.md`（`c2929b02` / `d06c1d16` / `8b9012c9`）；owner 决策见 `.openspec.yaml` 的 `x-owner-decisions`（D1/D2/D4 + magpie 澄清）。
> 本文件是 OpenSpec 层的**架构边界与权衡记录**：决策 / 理由 / 代价。契约文本见 `specs/**/spec.md`，工作包见 `tasks.md`。
> **本 change 只立项契约，不实施** —— 记录的是**待批准的设计决策**，不是已完成实现。

## 0. 分层边界（先划清谁负责什么）

本轮全部改动只落在三层，**禁止触碰四家 provider 目录**（`src/cross_cutting/{claude_code,codex,pi,kimi_code}_provider/**` —— 其中无任何安装逻辑）：

| 层 | 职责 | 本轮落点 |
|---|---|---|
| **共享安装层** `src/product/provider_installation/` | 安装源表、前缀解析、安装计划、装后探测、作业编排、失败分类 | 前缀候选与可写性判定、装后登记与三重验证、`path_hint` 文案、npm 参数 |
| **共享命令层** `src/cross_cutting/` | 平台发起命令的统一通道 | 新增共享 PATH overlay 模块；两个命令汇聚点合并 overlay |
| **共享索引层** `src/product/logical_codebase/aggregate_index/` | codegraph 调用与输出解析 | `files` 专用抓取配额、截断专用码、截断标志透传 |
| **provider 层** `src/cross_cutting/{claude_code,codex,pi,kimi_code}_provider/` | 各 provider 的协议适配 | **零改动**（overlay 经 `ProcessManager::spawn` 自动覆盖它们的进程拉起，不需要碰 provider 代码） |

关键推论：**四家 provider 的进程拉起全部经 `ProcessManager::spawn`**，因此把 overlay 合并做在共享命令层，provider 侧天然受益 —— 这正是「不触碰四家 provider 目录」仍能修好四家 provider 的原因。

## 0.1 契约变更一览（delta 组成）

| capability | delta 条目 | 性质 |
|---|---|---|
| `provider-cli-installation` | MODIFIED REQ-PCI-02 | 「探针结果」口径由「宿主 PATH」扩展为「宿主 PATH ∪ 平台登记目录（平台自身命令可见 PATH）」；既有正文与既有场景逐字保留，只补两条场景 |
| `provider-cli-installation` | MODIFIED REQ-PCI-03 | 「可写前缀」的「可写」由权限位判定改为**真实写入探测**；既有正文与既有场景逐字保留，只补三条场景 |
| `provider-cli-installation` | MODIFIED REQ-PCI-05 | 装后重新探测的判定口径（平台可见即判成功）、安装提示句原样呈现、Kimi 安装目录可被平台识别；既有正文与既有场景逐字保留，只补三条场景 |
| `provider-cli-installation` | ADDED REQ-PCI-08 | **新增**：「装了但不在终端 PATH 上」的显式状态（编号延续既有序列，既有为 REQ-PCI-01 … REQ-PCI-07） |
| `logical-codebase-aggregate-index` | MODIFIED REQ-IND-01 | 专用抓取配额（64 MiB）+ 专用截断码 `codegraph_files_output_truncated`；既有正文与既有场景逐字保留，只补四条场景 |
| `logical-codebase-aggregate-index` | MODIFIED REQ-IND-04 | 截断属构建失败路径 ⇒ 保留 last-known-good 并标 degraded、不伪造 active；既有正文与既有场景逐字保留，只补两条场景 |
| `logical-codebase-aggregate-index` | MODIFIED REQ-IND-05 | **正文语义不改**（致命 / 降级分界不动），只补一条场景把既有分界钉死：截断码不是工具缺失码、不进入致命匹配集 |

## 决策 1：平台命令可见 PATH 用**共享 overlay**，不改进程真实 env

**背景**：owner D1 要求「装完 CLI 仍不在宿主 PATH 上 ⇒ 判成功，并把安装目录纳入平台自身的命令可见 PATH」；D2 进一步把**实现口径**修订为共享 overlay。

直接实现 D1 的朴素做法是 `std::env::set_var("PATH", ...)`。本仓 edition 2024 下 `std::env::set_var` 是 **`unsafe fn`**，且在多线程进程里改全局 env 存在竞态（本仓是 tokio 多线程运行时 + 后台作业线程）。

**决定**：

1. 新增**共享 PATH overlay**（进程级注册表）：只追加、可去重、可查询、可清空；提供**单一合并函数**把 overlay 与基础 PATH 合成子进程 PATH。
2. **平台进程的真实 env 不变** —— 平台真实 PATH 保持原样；overlay 只在**构造子进程环境**时合并进去。
3. **不做 `std::env::set_var`**：无 unsafe、无全局 env 竞态；语义与 D1 等价 —— 平台自身发起的命令立即可见新装的 CLI。

**理由**：
- D1 的**语义目标**是「平台自己发起的命令能看到新装的 CLI」，不是「改变平台进程的全局环境」。overlay 精确覆盖前者，且不污染后者。
- 共享层落地后，**四家 provider 目录零改动**即可受益（它们的进程经共享命令层拉起）。
- overlay 是进程级共享状态，风险面收敛为「注册表本身」：只追加 + 去重 + 测试可清空 + 全局锁隔离用例。

**代价**：
- overlay 注册表是进程级共享状态 ⇒ 任何触碰它的用例必须持**全局锁**并在结束时清空，否则污染并行的 PATH 断言用例（仓库先例：`OnceLock<Mutex<()>>` / `LazyLock` 模式）。
- 平台**自身进程**的 PATH 仍不含新装目录 ⇒ 只有「平台发起的子进程」可见（这正是 D1 的口径）；用户在自己的终端里敲 `claude` 仍可能找不到 —— 该状态由 REQ-PCI-08 显式建模，不在本轮解决。

## 决策 2：overlay 只在**两个命令汇聚点**合并

**背景**：平台发起外部命令的通道不止一条。若只在某一处合并，会出现「探测说装上了、真正跑 provider 时又说没有」的分裂口径。

**决定**：overlay 合并只做在**两个汇聚点**，且两个点都必须**显式写 PATH**：

| 汇聚点 | 覆盖的调用 |
|---|---|
| `src/cross_cutting/process_manager.rs` 的 `spawn_with_environment` | 四家 provider 进程拉起、`provider_health` 探测、codegraph 装后探测、`install-targets` 现场探测 —— 全部经 `BoundedCommandRunner` → `ProcessManager::spawn(_isolated)` |
| `src/cross_cutting/cli_adapter.rs` 的 `run_command_capture_with_stream` | provider 会话执行路径（同步捕获输出） |

**关键细节**：`cli_adapter` 内部的命令构造在 `environment = None` 时**继承宿主环境**（`env_clear` 已删）⇒ 合并**不能**只写在 `Some(env)` 分支里，否则该分支之外的命令永远看不到 overlay。

**明确不覆盖**：git / ast-grep 工具的裸 `Command::new`（`src/cross_cutting/git_command.rs`、`src/cross_cutting/ast_grep_tool.rs`）—— 与 provider CLI 安装无关，不扩大范围。

**理由**：两个点覆盖了平台「命令可见性」的全部对外出口；每个点只需极少改动，回归面可控。

**代价**：overlay 的生效范围严格等于「经这两个点发起的命令」；任何未来新增的裸命令通道需要显式接入合并（登记在 `design.md` 的边界声明里，不在本轮实现）。

## 决策 3：启动期登记**实际存在**的用户工具目录

**背景**：overlay 只解决「本会话刚装的东西」。用户此前用 fnm / nvm / volta / pnpm / bun / cargo 等装的 CLI 在平台进程 PATH 上同样看不到。

**决定**：服务启动时把**实际存在**的用户工具目录登记进 overlay，目录清单沿用 magpie 的 `userDirs` 口径（`~/.local/bin`、`~/.npm-global/bin`、`~/.npm/bin`、`~/.bun/bin`、`~/.volta/bin`、`~/.asdf/shims`、`~/.local/share/mise/shims`、`~/.cargo/bin`、`~/.deno/bin`、`~/Library/pnpm`、`~/.local/share/pnpm`、nvm 默认 node 的 bin、mise 的 node installs、fnm 目录（`~/.local/share/fnm`、`~/Library/Application Support/fnm`、`$FNM_DIR`）、`/opt/homebrew/bin`、`/usr/local/bin`），**外加本仓自有落点**：`npm prefix -g` 的 bin、`NPM_PREFIX/bin`、`<home>/.kimi-code/bin`。

**约束**：
- **只登记、不写盘、不创建目录** —— 不存在的目录一律不登记（登记一个不存在的目录只会让 PATH 里出现幽灵条目）。
- 启动路径上的 `npm prefix -g` 探针用 **≤3s 超时**（生产安装作业里的同类探针是 10s；**不得**在启动路径引入 10s 阻塞）。
- 登记动作挂在启动流程里「bind 成功之后、首次 provider health 刷新之前」—— bind 失败必须零副作用。

**理由**：既让「用户早就装好的 CLI」对平台可见，又不引入写副作用与启动阻塞。

**代价**：启动多一次轻量目录探测（只 stat，不执行 npm 之外的重活）。

## 决策 4：可写性 = **mode 预筛 + 真实写入探测**

**背景**：`effective_prefix.rs:74` 的 `dir_is_writable` 只看 `mode & 0o222`。root 拥有的 `/usr/local`（0755）对任意用户都判「可写」⇒ npm 报 EACCES ⇒ 用户级兜底前缀永远走不到。

**决定**：`is_writable_dir` 改为两级判定：

1. **mode 预筛**：`mode & 0o222 == 0` ⇒ 直接判不可写（保留既有语义，避免 root 环境下既有测试反转）。
2. **真实写入探测**：否则在目录里用 `create_new` 建一个**唯一命名**的临时文件（名含 pid + 计数/随机后缀）⇒ 建成功即删（删失败只记日志）⇒ 建失败即判不可写。目录不存在时沿用「回退最近存在祖先」语义，祖先判定同样走这两级。

**测试 seam**：提供 `is_writable_dir_with(path, probe_fn)`（probe 为闭包）⇒ 单测可**确定性地**模拟「mode 说可写但实际写失败」，不依赖运行身份。

**理由**：真实写入探测才能反映「这个前缀这次真的写得进去」；mode 预筛则把明显不可写的目录（`0o500` 等）提前拒掉，两者组合既准确又便宜。

**代价**：该谓词**有写副作用** ⇒ 只允许用于「候选前缀探测」这一类一次性场景，不得用于热路径。

**明确不改**：既有两条断言（`choose_effective_prefix_skips_unwritable_candidates`、`is_writable_dir_falls_back_to_nearest_existing_ancestor`）在两级判定下**保持通过**（`0o500` 的 mode 预筛直接 false）⇒ 不得改动这两条断言。

## 决策 5：前缀候选判定链收紧为三级

**背景**：`resolve_effective_prefix`（`runner.rs:397`）的回落链不检查「装到的 bin 目录在不在 PATH 上」，导致装到 PATH 之外 ⇒ 装完探测失败 ⇒ 「没效果」。

**决定**：判定链写死为：

1. **visible** —— PATH 上当前生效 CLI 所在前缀（且**真实可写**）。
2. **`npm prefix -g`** —— 且**真实可写**、且 **`<prefix>/bin` 出现在 PATH 中**。
3. **fallback** —— `NPM_PREFIX` / `$HOME/.npm-global`。

**关键细节**：第 ② 级的判定语义**写死为「`<prefix>/bin` 这个目录是否出现在 PATH 中」**，**不得**用「命令文件是否存在」判定 —— 现有 `first_path_dir_with_command` 只在命令存在时返回 `Some`，而 ② 级恰恰发生在 CLI 未装时 ⇒ 用它会成死代码。

**理由**：第 ② 级若不查「bin 在不在 PATH」，就会重演「装到 PATH 之外 ⇒ 探测失败 ⇒ 没效果」；而查目录成员（而非命令文件）恰好覆盖「CLI 未装」这一唯一使用场景。

**代价**：当用户的 npm 全局前缀本来就不在 PATH 上时，平台会落到用户级兜底前缀（行为变化：从「装到看不见的地方」变成「装到 `~/.npm-global`」）—— 这正是期望结果，代价是用户终端里的 `npm` 默认前缀与平台装的位置不同，靠 `path_hint` 告知。

## 决策 6：装后验证必须是**三重验证**，且区分三种失败语义

**背景**：容器实测 `codegraph --version` 在**平台 bundle 缺失**时仍返回版本号且 exit 0 ⇒ 「装完只跑 `--version`」会假绿。同时 `codegraph --version` 只证明「命令能起」，不证明「命令能干正事」。

**决定**：装后验证三重：

1. **版本读取优先读 npm 包的 `package.json`**（不执行进程、更快更稳）；读不到才跑 `--version`。
2. **必须跑一个真实命令**证明可执行（不能只 `--version`）。
3. **失败时区分三种语义**并给对应可行动提示：「没装上」/「装上但缺平台包」/「装上但不在 PATH」。

**绝对路径兜底探测**：PATH 找不到 CLI 时，用 `<bin>/<command>` 绝对路径探测；该探测**必须经 `BoundedCommandRunner`**（否则 overlay 合并不生效），且其结果**只填 `installed_version`**，**不得**绕过 REQ-PCI-02「已装但探针未过」的语义（能力核验仍走既有通道）。

**理由**：`--version` 在 bundle 缺失时假绿 ⇒ 只跑它会把「装上但跑不起来」判成成功，用户体验仍是「没效果」。

**代价**：装后探测的耗时与命令数上升（版本读取多一次文件读、验证多一次真实命令）；失败语义分类需要人工给出可复制修复命令。

## 决策 7：Kimi 安装目录以「**实际输出 / 实际存在**」为准

**背景**：Kimi 走官方脚本，脚本只改 shell rc、平台进程看不到；且**无 npm 包**⇒ 既有 `npm_path_hint`（只对 npm 行产出）对其完全无输出。

**决定**：
- 安装源表新增 Kimi 安装目录常量（`~/.kimi-code/bin`）与解析函数，**标注为假设** —— 该默认值来自官方脚本的 `KIMI_INSTALL_DIR` 约定（容器实测脚本输出 `Installed to /root/.kimi-code/bin/kimi`），离线无法二次核实。
- **overlay 登记条件以「脚本实际输出 / 实际存在的文件」为准**，不依赖常量本身。
- `path_hint` 对 **Kimi 行也产出**（文案指向该 bin 目录）。

**理由**：常量是假设 ⇒ 以事实为准的登记条件才不会在默认目录被改过时误登记；提示句则是「用户下一步该做什么」的唯一载体。

**代价**：默认目录若被官方改掉，提示文案会指向旧目录（但登记条件以实际文件为准，平台可见性不受影响）。

## 决策 8：`codegraph files --json` 走**专用抓取配额 + 专用截断码**

**背景**：根因 A —— 1 MiB 上限套给所有 codegraph 调用；截断标志在 `command_result` 被丢弃；截断文本被当 JSON 解析 ⇒ 伪装成协议异常。

**决定**：

1. **专用配额**：`files` 使用有文档上限的专用抓取配额（**64 MiB**）；其余命令维持既有 1 MiB。Rust 无默认参数 ⇒ 「默认 1 MiB」必须**显式化**，不得靠签名默认值。
2. **截断可见**：命令结果结构携带 `stdout_truncated` 标志（`CodeGraphCommandResult`，re-export 于 `src/product/logical_codebase/aggregate_index/mod.rs`；不改 `bounded_command_runner` —— 该层已有该字段）。这是本轮唯一的 API 可见性变化。
3. **专用截断码**：解析**之前**先判截断 ⇒ 以专用机器码 `codegraph_files_output_truncated` 报告，消息含上限字节数与「输出被截断」说明；**不得**报成 `codegraph_files_invalid_json`。
4. **`--no-metadata` 无条件传**：1.6.0 / 1.6.2 均接受该 flag；**实测 `--json` 模式下载荷逐字节不变**（该 flag 只影响 tree 文本格式）⇒ 本项**不带来瘦身**，保留它的理由是「flag 被支持区间内的 CLI 接受且不改变 JSON 语义」。不做版本特判、不做静默去旗标降级（降级会掩盖真实失败）。
5. **不进入致命匹配集**：截断码**不是**工具缺失码 ⇒ 不进 `bootstrap_projector.inc.rs:104` 的致命匹配集（该集合只含 `codegraph_missing` / `codegraph_version_mismatch`）⇒ 自动走「有 LKG ⇒ degraded」一般规则；**不改**该投影文件。

**理由**：截断是「抓取预算不够」这一**平台侧**事实，不是「上游协议变了」。把它伪装成协议异常会让用户去查 CLI 版本，而真正要做的要么是缩小聚合根、要么是提高配额。

**代价**：64 MiB 抓取上限提高大聚合根的峰值内存（实测载荷约 100~195 B/文件）；上限只作用于 `files`，其余调用不变。超大聚合根仍会触发专用截断码（后续议题：落盘 + 流式解析，本轮不做）。

## 决策 9：前端**原样渲染** `path_hint`，并新增「不在终端 PATH 上」显式状态

**背景**：后端 `path_hint` 已是完整句子，前端再包一层 ⇒ 破碎双句（已提交 `6cd5075e` 修复）。另，平台可见 ≠ 用户终端可见：即使平台能跑 `claude`，用户在终端里敲 `claude` 仍可能找不到 ⇒ 用户仍会以为「没效果」。

**决定**：
1. `path_hint` **原样渲染**（整句逐字呈现，不二次包裹）；后端文案缺失时不渲染该节点；前端类型注释随包 2 的后端文案一并更正（`path_hint` 是完整句子，不是「提示目录」）。
2. 新增**显式状态**：安装目标视图增加 `off_path` 标志 —— 「平台能找到该 CLI，但它的 bin 目录不在登录 shell PATH 上」时为真，界面显式呈现该状态。
3. **本轮只加状态与文案，不自动改用户 shell profile**（不创建 symlink、不写 `.bashrc` / `.zshrc`）。

**理由**：显式状态让「平台可用 / 终端不可见」这件事在产品里说清楚，替代「一句提示」；自动改 shell profile 属于越界副作用（且用户未必想要）。

**代价**：用户仍需自行把目录加入终端 PATH（文案给出可复制内容）。

## 权衡汇总

| 备选 | 结论 | 理由 |
|---|---|---|
| 改进程 env（`set_var`）vs 共享 overlay | **overlay** | edition 2024 下 `set_var` 是 unsafe + 多线程竞态；overlay 语义等价且更可控 |
| 只在 `cli_adapter` 合并 vs 两个汇聚点都合并 | **两处都合并** | 只合并一处会出现「探测说装上了、真正跑 provider 时没有」的分裂口径 |
| 前缀 bin 在 PATH 的判定：命令文件是否存在 vs 目录成员 | **目录成员** | 命令文件判定的唯一使用场景恰是「CLI 未装」⇒ 必然落空 |
| 可写性：只看 mode vs 只真实探测 vs 两级 | **两级（mode 预筛 + 真实探测）** | 纯真实探测会让 root 环境下的既有测试反转；纯 mode 判据已被实测证伪 |
| 装后验证：只 `--version` vs 三重验证 | **三重** | `--version` 在 bundle 缺失时假绿 |
| 截断：报 `codegraph_files_invalid_json` vs 专用码 | **专用码** | 截断是平台侧抓取预算事实，不是上游协议异常 |
| `--no-metadata`：传 vs 不传 | **无条件传**（owner D3 已拍板） | flag 被支持区间内 CLI 接受且不改变 JSON 语义；未来 CLI 若移除该 flag 需回 owner 裁决，实施侧不得自行删 |
| 终端可见性：自动 symlink 暴露 vs 只加显式状态 | **只加显式状态** | 自动改用户 shell profile 属越界副作用；暴露动作是待裁决议题 |
| magpie 的「不代跑安装」 | **不采纳**（owner 澄清） | 目标不变：产品内一键安装必须真的装上并能用 |

## 明确不改的部分

| 项 | 原因 |
|---|---|
| `src/cross_cutting/{claude_code,codex,pi,kimi_code}_provider/**` | 零改动；overlay 经共享命令层自动覆盖它们的进程拉起 |
| `openspec/specs/**`（主规格） | 本轮不改；归档时由 delta 自动同步 |
| codegraph 版本门（`>=1.6.0, <2.0.0`） | 主规格 REQ-IND-01「exact version 钉定 v1.5.0」与实现的既有漂移本轮不修，归档同步时双审对齐 |
| 致命 / 降级分界（`codegraph_missing` / `codegraph_version_mismatch`） | 不动；截断码不进入致命匹配集 |
| git / ast-grep 的裸命令通道 | 与 provider CLI 安装无关，不扩大范围 |
| 用户 shell profile | 本轮不自动改；「装好后暴露到终端 PATH」是待裁决议题 |
| agent 专属环境变量注入 | 另立议题 |
