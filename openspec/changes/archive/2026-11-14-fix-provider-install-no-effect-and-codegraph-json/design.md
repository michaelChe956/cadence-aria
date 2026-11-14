# 设计：修复「页面点安装没有效果」与「聚合索引 codegraph 截断报错」

> 依据：`cadence/plans/2026-11-13_计划文档_修复Provider安装无效果与聚合索引截断_v1.0.md`（`c2929b02` / `d06c1d16` / `8b9012c9`）；owner 决策见 `.openspec.yaml` 的 `x-owner-decisions`（D1/D2/D4 + magpie 澄清 + 更新语义 + 库 API 覆盖验收 + 暴露终端 PATH + agent 专属环境变量注入）。
> 本文件是 OpenSpec 层的**架构边界与权衡记录**：决策 / 理由 / 代价。契约文本见 `specs/**/spec.md`，工作包见 `tasks.md`。
> **实施状态（2026-11-13）**：契约已获批并**已实施**；本文件记录的是**已落地的决策与取舍**，各决策的「实现落点」给出仓库内位置（未闭环项见 `tasks.md` 的实施状态对照）。

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
| `provider-cli-installation` | MODIFIED REQ-PCI-03 | 「可写前缀」的「可写」由权限位判定改为**真实写入探测**；既有正文与既有场景逐字保留，只补三条场景（安装参数形态按实施口径补 `--allow-scripts=<pkg>` 与 `--include=optional` 重试） |
| `provider-cli-installation` | MODIFIED REQ-PCI-05 | 装后重新探测的判定口径（平台可见即判成功）、安装提示句原样呈现、Kimi 安装目录可被平台识别；装后验证补「三重验证 + 绝对路径兜底探测」；既有正文与既有场景逐字保留，其余只做增补 |
| `provider-cli-installation` | ADDED REQ-PCI-08 | **新增**：「装了但不在终端 PATH 上」的显式状态（编号延续既有序列，既有为 REQ-PCI-01 … REQ-PCI-07） |
| `provider-cli-installation` | ADDED REQ-PCI-09 | **新增**：更新落点必须与被更新对象当前所在的前缀一致（已在 PATH 上 ⇒ 装回同一前缀；已安装但不在 PATH 上 ⇒ 从已安装包真实路径反推 npm 前缀，不得改装到别的前缀） |
| `provider-cli-installation` | ADDED REQ-PCI-10 | **新增**：更新后版本未变须明确报告「命令仍是旧版」（不得静默成功）；Kimi 更新沿用官方脚本原地替换并经装后验证 |
| `provider-cli-installation` | ADDED REQ-PCI-11 | **新增**：**用户显式触发**的「暴露到终端 PATH」动作 —— 不改 shell 配置文件，只在「已存在 + 真实可写 + 确实在终端 PATH 上」的目录里建单个软链，失败如实报告 |
| `provider-cli-installation` | ADDED REQ-PCI-12 | **新增**：把启动期从**交互式登录 shell**探测到的 agent 专属环境变量（`PI_CODING_AGENT_DIR` / `CODEX_HOME` / `CLAUDE_CONFIG_DIR`）**补进平台子进程 env** —— 调用方显式覆盖层与平台进程 env 优先，缺失才补；2s 硬上限、失败不阻塞；两处命令汇聚点都生效（决策 13） |
| `logical-codebase-aggregate-index` | MODIFIED REQ-IND-01 | 专用抓取配额（64 MiB）+ 专用截断码 `codegraph_files_output_truncated`；**成员覆盖验收改走 codegraph 库 API**（逐成员目录级查询，不依赖全量文件清单）；既有正文与既有场景逐字保留（成员覆盖条款按实施口径更新），只做增补 |
| `logical-codebase-aggregate-index` | MODIFIED REQ-IND-01（场景） | 新增「库 API 逐成员查询 / 目录边界 / 排除性查询不动 / 回退留痕 / 回退非常规 / 健康闸只告警 / 回退路径仍受专用配额与专用截断码」相关场景 |
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

同一对汇聚点也是**agent 专属环境变量注入**的唯一落点（决策 13）：两条通道都必须补注入，缺一条就会出现「平台探测读到了用户配置、真正拉起 provider 时却读默认位置」的分裂口径。

**明确不覆盖**：git / ast-grep 工具的裸 `Command::new`（`src/cross_cutting/git_command.rs`、`src/cross_cutting/ast_grep_tool.rs`）—— 与 provider CLI 安装无关，不扩大范围。

**理由**：两个点覆盖了平台「命令可见性」的全部对外出口；每个点只需极少改动，回归面可控。

**代价**：overlay 的生效范围严格等于「经这两个点发起的命令」；任何未来新增的裸命令通道需要显式接入合并（登记在 `design.md` 的边界声明里，不在本轮实现）。

## 决策 3：启动期登记**实际存在**的用户工具目录

**背景**：overlay 只解决「本会话刚装的东西」。用户此前用 fnm / nvm / volta / pnpm / bun / cargo 等装的 CLI 在平台进程 PATH 上同样看不到。

**决定**：服务启动时把**实际存在**的用户工具目录登记进 overlay（落点 `src/cross_cutting/path_overlay.rs` 的 `register_startup_overlay` → `register_existing_candidates` → `user_tool_dirs` + `host_absolute_tool_dirs`），目录清单沿用 magpie 的 `userDirs` 口径：固定清单（`~/.local/bin`、`~/.npm-global/bin`、`~/.npm/bin`、`~/.bun/bin`、`~/.volta/bin`、`~/.asdf/shims`、`~/.local/share/mise/shims`、`~/.cargo/bin`、`~/.deno/bin`、`~/Library/pnpm`、`~/.local/share/pnpm`、fnm 目录（`~/.local/share/fnm`、`~/Library/Application Support/fnm`、`$FNM_DIR`））+ 环境驱动项（`~/.npmrc` 的 `prefix=` bin、nvm 默认 node 的 bin、mise 的 node installs）+ 宿主绝对落点（`/opt/homebrew/bin`、`/usr/local/bin`）。**npm 全局前缀（`npm prefix -g`）的 bin、`NPM_PREFIX/bin`、`<home>/.kimi-code/bin` 不在启动清单内** —— 它们由**装后登记**（决策 5）按「该目录确实含刚装好的可执行文件」这一事实条件覆盖：登记条件以事实为准，才不会在默认目录被改过时误登记，也不必在启动期为一个可能尚不存在的目录做外部探针。

**约束**：
- **只登记、不写盘、不创建目录** —— 不存在的目录一律不登记（登记一个不存在的目录只会让 PATH 里出现幽灵条目）。
- **启动路径不执行外部探针**（只 stat 目录 + 读 `~/.npmrc`）：`npm prefix -g` 探针只发生在**安装作业**路径（`runner.rs` 的 `CommandNpmGlobalPrefixProbe`，短超时），不落在启动关键路径 ⇒ 启动零阻塞。
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

**真实命令的选择**（实施口径，落点 `runner.rs` 的 `post_install_verification_argv`）：codegraph 用 `status --json`（实测 1.6.2 在未初始化目录 exit 0，而一切命令都经平台包自带 node + dist 启动 ⇒ 平台包缺失时 exit 1 ⇒ 既证明 bundle 真能用、又不把「未建索引」误判成失败）；provider CLI 用 `--help`（真正加载入口与依赖，缺依赖即非零退出）。MUST NOT 用 `codegraph files`（未初始化目录 exit 1 ⇒ 会把新装的健康 codegraph 判失败），MUST NOT 附带元数据抑制旗标（与 REQ-IND-01 已裁决「只带 `--json`」同类）。

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
4. **文件清单调用不带额外旗标**：只下发 `files --json`。**owner 2026-11-13 裁决去掉**曾经考虑过的额外旗标（实测该旗标在 `--json` 模式下载荷**逐字节不变**、零收益，却引入「未来 CLI 移除该旗标 ⇒ 文件清单全线失败」的风险）⇒ 保留它的理由不成立，予以去掉。不做版本特判、不做运行时去旗标降级（降级会掩盖真实失败）；`files --json` 是本仓支持区间内已验证的调用形态。
5. **不进入致命匹配集**：截断码**不是**工具缺失码 ⇒ 不进 `bootstrap_projector.inc.rs:104` 的致命匹配集（该集合只含 `codegraph_missing` / `codegraph_version_mismatch`）⇒ 自动走「有 LKG ⇒ degraded」一般规则；**不改**该投影文件。

**理由**：截断是「抓取预算不够」这一**平台侧**事实，不是「上游协议变了」。把它伪装成协议异常会让用户去查 CLI 版本，而真正要做的要么是缩小聚合根、要么是提高配额。

**代价**：64 MiB 抓取上限提高大聚合根的峰值内存（实测载荷约 100~195 B/文件）；上限只作用于 `files`，其余调用不变。超大聚合根仍会触发专用截断码（后续议题：落盘 + 流式解析，本轮不做）。

## 决策 9：前端**原样渲染** `path_hint`，并新增「不在终端 PATH 上」显式状态

**背景**：后端 `path_hint` 已是完整句子，前端再包一层 ⇒ 破碎双句（已提交 `6cd5075e` 修复）。另，平台可见 ≠ 用户终端可见：即使平台能跑 `claude`，用户在终端里敲 `claude` 仍可能找不到 ⇒ 用户仍会以为「没效果」。

**决定**：
1. `path_hint` **原样渲染**（整句逐字呈现，不二次包裹）；后端文案缺失时不渲染该节点；前端类型注释随包 2 的后端文案一并更正（`path_hint` 是完整句子，不是「提示目录」）。
2. 新增**显式状态**：安装目标视图增加 `off_path` 标志 —— 「平台能找到该 CLI，但它的 bin 目录不在登录 shell PATH 上」时为真，界面显式呈现该状态。（**实施状态：已落地** —— 后端 `src/web/handlers/provider_installation.rs` 的 `off_path` DTO 字段与 `resolve_off_path`（登录 shell PATH 快照 ∪ 平台登记目录；提交 `d7d759c7`），前端 `web/src/api/types/provider.ts` + `web/src/components/lifecycle/ProviderCapabilityCard.tsx` 的「已安装，但不在你的终端 PATH 上」徽标（提交 `621d3269`）。）
3. **本轮只加状态与文案，不自动改用户 shell profile**（不自动创建 symlink、不写 `.bashrc` / `.zshrc`）；用户**显式触发**的「暴露到终端 PATH」动作见决策 12。

**理由**：显式状态让「平台可用 / 终端不可见」这件事在产品里说清楚，替代「一句提示」；自动改 shell profile 属于越界副作用（且用户未必想要）。

**代价**：用户仍需自行把目录加入终端 PATH（文案给出可复制内容），或显式点【暴露到终端 PATH】让平台建单个软链（决策 12）。

## 决策 10：更新语义 —— **落点必须与被更新对象一致**，更新后版本未变必须显式报告

**背景**：owner 2026-11-13 明确本轮范围**不止安装，还要包含更新**。更新与安装复用同一条异步作业链（REQ-PCI-05 的作业 / 轮询 / durable 语义），但两者对「落点」的要求不同：安装可以**选**一个真实可写的前缀；更新**不能** —— 更新装到别的前缀，就变成「更新装到 A、用户跑的还是 B」（`src/product/provider_installation/runner.rs` 的前缀解析注释记录过同型的 2026-09-09 老 bug）。

**决定**（契约见 REQ-PCI-09 与 REQ-PCI-10）：

1. **更新落点 = 被更新对象当前所在的前缀**：
   - 已安装且命令在宿主 PATH 上 ⇒ 更新装回**同一个前缀**（决策 5 的 visible 链即满足）。
   - 已安装但命令**不在**宿主 PATH 上 ⇒ 从**二进制的真实路径反推 npm 前缀**（形如 `.../node_modules/<pkg>/...` ⇒ 其 `lib` 目录的父目录即该前缀）⇒ **不得**改装到别的前缀。
   - 前缀候选与「可写」判定沿用决策 4 / 决策 5 的口径（真实写入探测、三级候选链），MUST NOT 为更新另开一套判定。
2. **更新后版本未变 ⇒ 明确报告**：更新执行完成、但探测到的命令版本与更新前相同 ⇒ 报「命令仍是旧版」并给可行动诊断，**不得**静默判成功、不得把该结果呈现为「更新已完成」。**前后两次取数 MUST 同源同口径**（同一探测通道、同一解析口径）—— 口径不一致时「版本未变」恒不成立，这条判定在生产上永不触发。
3. **Kimi 更新**：沿用官方脚本的**原地替换**通道（REQ-PCI-05 既有条款），并经决策 6 的装后三重验证确认更新后的命令真实可用。

**理由**：更新语义的价值在于「用户敲的命令真的变新了」；落点漂移或版本未变被静默吞掉，都会让用户回到「点了没效果」的老症状，而这两点恰恰是安装链与更新链语义差异最大的地方。

**代价**：反推前缀需要从已解析出的二进制真实路径推导包前缀（新增一处推导逻辑，且推导失败时必须显式报错而不是回退到「随便选个前缀」）；版本未变的判定需要保留更新前的版本快照用于比对。

### 实现边界（决策 10）

| 边界 | 口径 |
|---|---|
| 落点反推 | 「已安装但不在 PATH 上」的反推口径 = 从**已安装包的真实路径**反推（`<prefix>/lib/node_modules/<pkg>` ⇒ 其 `lib` 目录的父目录即该前缀）；候选来自 npm 全局前缀与平台自身前缀，MUST NOT 靠「命令是否在 PATH 上」猜 |
| 反推失败 | MUST 显式报错并给出可行动诊断，MUST NOT 静默改装到别的前缀 |
| 版本比对 | 更新**前**与更新**后**的版本取数 MUST **同源同口径**（同一探测通道、同一解析口径）：npm 类更新前读该前缀下包的 `package.json`（不执行进程），脚本类（Kimi 等）更新前走与装后同一套 `--version` 探测口径；更新后再取一次。**口径不一致 ⇒ 「版本未变」恒不成立 ⇒「命令仍是旧版」判定在生产上永不触发**（终审 R10）。两次相同 ⇒ 报「命令仍是旧版」并给可复制的修复命令，作业判失败（不得静默成功） |
| Kimi 更新 | 走官方脚本原地替换，不进 npm 路径；更新后同样经装后三重验证 |
| 实现落点 | `src/product/provider_installation/effective_prefix.rs`（`npm_prefix_from_real_path` / `prefix_bin_is_on_path` / `is_writable_dir`）、`src/product/provider_installation/runner.rs`（`resolve_effective_prefix` / `installed_prefix` / 版本快照与「仍是旧版」判定） |

## 决策 11：聚合索引成员覆盖验收改走 **codegraph 库 API（SDK）**

**背景**：成员覆盖验收要回答的是「每个登记成员目录**有没有**被索引的文件」，而原做法是让 codegraph 吐出**整个仓库的文件清单**（`codegraph files --json`）再全量读进内存。载荷与仓库规模**线性相关**（每文件约 110~200 B；`codegraph.db` : JSON ≈ **13 : 1**）⇒ 用户现场 **3.1 GB 索引 ⇒ 清单约 230 MB**，抓取上限（1 MiB / 64 MiB）**无论如何都装不下** ⇒ 验收必失败（现场报 `codegraph_files_invalid_json`）。**索引本身是好的，只是平台读不出来、因此不承认它。**

**决定**：

1. **逐成员目录级查询**：成员覆盖判定改走 codegraph **库 API** —— `CodeGraph.open(root)` 后逐成员 `hasFilesUnder(member)`，直接回答是/否，载荷是**成员级布尔事实**（几百字节量级）。
2. **目录边界精确**：`hasFilesUnder` 是**目录语义**（`member-al` 对 `member-alpha` 返回 false），优于 CLI 清单路径的裸前缀匹配。
3. **Node helper + codegraph 自带 Node**：库 API 是 npm 包的 CJS 入口（`npm-sdk.js`），普通脚本无法直接 import ⇒ helper 按**绝对路径** `import(<npm-sdk.js>)` 并做 CJS interop（`mod.default ?? mod` 取 `CodeGraph`）。helper 源码以 `include_str!` 内嵌随源码走，运行时落唯一临时文件、执行后清理（超时 60s；stdout 上限 1 MiB / stderr 上限 64 KiB）。
   - **Node 解析顺序**：codegraph **平台包自带的 node** 优先（⇒ 不要求用户装 Node）→ 回退 PATH 上的 node，且实测版本 ≥ **22.5** 才采用。自带 node 的解析 MUST 覆盖**当前平台 / 版本下**的三种布局（平台包内 nested、与主包同级的 hoisted、上游自愈缓存），并按平台名（`codegraph_platform_target()`）择优；node 不可用 ⇒ 映射为 `codegraph_sdk_incompatible` 并回退 CLI 清单验收。
   - **SDK 路径解析顺序**：① 平台自身安装前缀（npm 标准布局）→ ② `npm root -g` 探针（10s / 4096 B）→ ③ 从 `codegraph` 可执行真实路径反推包根。
4. **回退保底**：SDK 找不到（`codegraph_sdk_not_found`）或与受支持版本不兼容（`codegraph_sdk_incompatible`）⇒ 回退既有 CLI 文件清单路径（**保留** 64 MiB 专用配额与 `codegraph_files_output_truncated` 专用截断码），并在索引激活记录上留**可审计 warning**。回退**不是**常规路径。
5. **健康闸只告警不阻断**：`index.stale` / `index.pending > 0` / `index.state` 异常 / `stats.fileCount == 0` ⇒ 记 warning（随激活记录留存），MUST NOT 单独阻断激活（保持既有「不因健康问题阻断」的语义）；未知 state 值也计入信号 —— 宁可多一条可审计告警也不静默。
6. **排除性查询不动**：非成员目录 / `.worktrees` / `.aria` / 构建产物未被索引的负查询**保持既有 CLI `query --json` 路径**（载荷小，MUST NOT 一并改走库 API）。

**取舍**：

| 维度 | 取舍 |
|---|---|
| 依赖 SDK 的**目录级查询**能力 | SDK 不含 `hasFilesUnder` ⇒ 直接判不兼容并回退，MUST NOT 自己做「拼路径前缀」的猜测式降级 |
| **版本区间锁定** | 库 API 的可用性绑定 codegraph 受支持区间（`>=1.6.0, <2.0.0`，语义 `^1.6`）；区间外由既有版本门先行判 `codegraph_version_mismatch`，不进入本路径 |
| **回退保底** | SDK 路径失败不拖垮索引构建 —— 退回旧路径（小仓照常工作）；代价是大仓仍会撞 64 MiB 截断，以**专用码**如实报告，MUST NOT 伪装成协议不符 |
| 代价 | 新增一条 Node 子进程通道（helper）+ 临时文件生命周期管理；helper 失败事实以专用码区分（`codegraph_sdk_not_found` / `codegraph_node_unavailable` / `codegraph_coverage_failed` / `codegraph_sdk_incompatible`） |

**实现落点**：

| 位置 | 内容 |
|---|---|
| `src/product/logical_codebase/aggregate_index/codegraph_coverage.rs` | 覆盖探针 trait / 生产实现、SDK 与 node 解析、helper 落盘与清理、输出解析 |
| `src/product/logical_codebase/aggregate_index/codegraph_coverage_helper.mjs` | Node helper（`CodeGraph.open` + 逐成员 `hasFilesUnder` + 健康字段） |
| `src/product/logical_codebase/aggregate_index/operation.rs` | 验收编排：库 API 优先 → 回退 CLI 清单 → 健康闸 warning → 排除性查询（保持 CLI） |
| `src/product/logical_codebase/failure_copy_table.inc.rs` | 四个新失败码的用户文案 |

## 决策 12：「暴露到终端 PATH」是**用户显式触发**的单个软链，不是自动改 shell

**背景**：PATH overlay 只让**平台自己**看见新装的 CLI；用户的终端（Terminal / VS Code 终端）有自己那套 PATH（由 `.zshrc` 等决定）⇒ 用户在自己终端里敲 `claude` 仍可能 `command not found` ⇒ 依然会觉得「装了没用」。

**决定**（契约见 REQ-PCI-11）：

1. **只做显式动作**：用户点击【暴露到终端 PATH】才动作；平台 MUST NOT 自动创建软链、MUST NOT 写 `.bashrc` / `.zshrc` / 等价文件、MUST NOT 改写 PATH 环境变量。
2. **候选目录准入三条件**：① 该目录**确实在登录 shell PATH 上**（目录成员语义）；② 目录存在且**真实可写**（复用决策 4 的「mode 预筛 + 写入探测」）；③ 目录内无同名残留（同名且已指向同一目标 ⇒ 视为已暴露、直接成功；同名但指向别处 ⇒ 跳过该目录，**不覆盖用户文件**）。
3. **动作 = 建单个软链** `<dir>/<command>` → 已安装二进制；只建这一个。
4. **事后重验**：软链建好后重验「目录仍在登录 shell PATH 上」且「软链可解析到目标」；失败 ⇒ 只清理**本次新建**的软链（不动用户其它文件）并如实报错。
5. **失败如实报告**：全部候选不合格 ⇒ 汇总各候选的不合格原因；未安装对象 ⇒ 明确拒绝；Windows 本轮不支持 ⇒ 明确报错，不静默降级。

**理由**：自动改用户 shell profile 属越界副作用；「显式动作 + 严格准入 + 事后重验」把「用户终端能不能直接跑」从一句提示变成**可验证的事实**。

**代价**：用户仍需点一次按钮；用户终端 PATH 取不到（服务启动 ≤2s 内或探测失败）时无法证明任何目录「在终端 PATH 上」⇒ 明确让用户稍后重试，不瞎猜。

**实现落点**：

| 位置 | 内容 |
|---|---|
| `src/product/provider_installation/expose.rs` | seam 核心（候选准入 / 建软链 / 事后重验）+ 生产入口 |
| `src/web/handlers/provider_installation.rs` | `POST /api/providers/{target}/expose` 端点与错误码 |
| `web/src/api/provider-install.ts` + `web/src/components/lifecycle/ProviderInstallActionButton.tsx` | 前端按钮与结果 / 失败文案呈现 |

## 决策 13：登录 shell 探测到的 agent 专属环境变量**补进子进程 env**（2s 上限、缺失才补）

**背景**：平台进程不是登录 shell ⇒ 用户写在 `.zshrc` / `.profile` 里的 `PI_CODING_AGENT_DIR`（Pi 的 `models.json` 就靠它落在 `~/.pi/agent`）、`CODEX_HOME`、`CLAUDE_CONFIG_DIR` 对平台**不可见**。平台拉起 `pi` / `codex` / `claude` 时这些变量缺失 ⇒ 子进程去读**默认配置位置** ⇒ 用户看到的是「模型配了却不生效 / 找不到配置」。owner 2026-11-13 裁决「本轮做」（方案 §五 2.12），撤回此前「不在本轮范围」的界定。

**决定**（契约见 REQ-PCI-12）：

1. **探测方式**：启动期用用户的 `SHELL` 起**交互式 + 登录** shell（`-i -l -c`，`stdin` 置空、`stderr` 丢弃），以唯一标记行 `__aria_shell_probe__` 打印 `PATH` 与收敛清单里的 agent 专属变量；rc 文件的 banner 污染由「只认标记行」+ 空值过滤兜底。变量清单集中在一处常量（当前 `PI_CODING_AGENT_DIR` / `CODEX_HOME` / `CLAUDE_CONFIG_DIR`），扩展只需追加。
2. **2s 硬上限 + 不阻塞启动**：探测与 PATH 快照共用同一次 `tokio::spawn` 异步任务，`tokio::time::timeout` 2s 硬上限 + `kill_on_drop` 兜底回收子进程；超时/失败 ⇒ 空结果，启动流程一次都不等待。**同一次探测同时服务两个用途**：终端 PATH 快照（REQ-PCI-08 / REQ-PCI-11 的前提）与 agent 变量注入 —— 不为注入多起一个 shell。
3. **合并优先级（缺失才补）**：① 调用方显式传入的环境覆盖层已有该变量 ⇒ 用调用方值；② 否则平台进程自身 env 已有 ⇒ 用平台已有值；③ 都没才补探测值。平台 MUST NOT 用探测值覆盖用户显式指定的值；探测到空值的变量**直接丢弃**（MUST NOT 生成空值 env 条目）。
4. **落点 = 决策 2 的两个命令汇聚点**：进程拉起通道（`process_manager.rs` 的 `spawn_with_environment`）与同步捕获通道（`cli_adapter.rs` 的 `run_command_capture_with_stream`）都要补注入；缺一条就会出现「探测读到了用户配置、真正跑 provider 时读默认位置」的分裂口径。
5. **失败不阻塞**：探测超时 / 失败 / 不可用（未设 `SHELL`、shell 不可执行、输出不含标记行）⇒ 注入是**空操作**；安装、装后探测、终端 PATH 判定与索引验收 MUST NOT 因它失败。

**与 REQ-PCI-08 / REQ-PCI-11 的关系（两者独立，不互相替代）**：

| 能力 | 解决什么 | 作用对象 |
|---|---|---|
| REQ-PCI-12（本决策） | provider **读到用户的配置**（配置目录该类环境变量） | 平台发起的**子进程 env** |
| REQ-PCI-08 / REQ-PCI-11 | 用户终端里**能不能敲出命令**（PATH 可见性与单个软链） | 用户的**登录 shell**（人） |
| 决策 1 / 2（overlay） | 平台自己**找不找得到**新装的 CLI | 平台子进程的 `PATH` |

三者互不蕴含：注入 agent 变量不会让用户在终端敲出 `claude`，暴露终端 PATH 也不会让 provider 读到配置目录变量。因此 REQ-PCI-12 的注入 MUST NOT 改变 REQ-PCI-08 的判定，也 MUST NOT 触发 REQ-PCI-11 的动作。

**理由**：只补「平台缺失的」变量，等价于把用户 shell 里的配置意图**透传**给 provider，同时不夺走平台/调用方更权威的取值（安装作业自己设的、测试注入的覆盖层都得赢）。

**代价**：启动多一次 shell 启动开销（已异步 + 2s 上限，不落在启动关键路径）；rc 文件若有副作用（打印、慢命令）会拖慢探测 —— 超时兜底保证不会拖住启动。

**实现落点**：

| 位置 | 内容 |
|---|---|
| `src/cross_cutting/path_overlay.rs` | `SHELL_PROBE_VARS` 变量清单 + `probe_login_shell_env` 探测 + `parse_shell_probe_stdout` 解析 + `shell_env_overrides` 合并优先级 + `register_startup_overlay` 异步启动 |
| `src/cross_cutting/process_manager.rs` | `spawn_with_environment` 内合并注入（进程拉起通道） |
| `src/cross_cutting/cli_adapter.rs` | `run_command_capture_with_stream` 内合并注入（同步捕获通道，含 `environment = None` 的继承路径） |

## 权衡汇总

| 备选 | 结论 | 理由 |
|---|---|---|
| 改进程 env（`set_var`）vs 共享 overlay | **overlay** | edition 2024 下 `set_var` 是 unsafe + 多线程竞态；overlay 语义等价且更可控 |
| 只在 `cli_adapter` 合并 vs 两个汇聚点都合并 | **两处都合并** | 只合并一处会出现「探测说装上了、真正跑 provider 时没有」的分裂口径 |
| 前缀 bin 在 PATH 的判定：命令文件是否存在 vs 目录成员 | **目录成员** | 命令文件判定的唯一使用场景恰是「CLI 未装」⇒ 必然落空 |
| 可写性：只看 mode vs 只真实探测 vs 两级 | **两级（mode 预筛 + 真实探测）** | 纯真实探测会让 root 环境下的既有测试反转；纯 mode 判据已被实测证伪 |
| 装后验证：只 `--version` vs 三重验证 | **三重** | `--version` 在 bundle 缺失时假绿 |
| 截断：报 `codegraph_files_invalid_json` vs 专用码 | **专用码** | 截断是平台侧抓取预算事实，不是上游协议异常 |
| 文件清单调用：带额外旗标 vs 只带 `--json` | **只带 `--json`**（owner 2026-11-13 裁决去掉额外旗标） | 实测该旗标在 `--json` 模式下载荷逐字节不变（零收益）；保留它只会引入「未来 CLI 移除该旗标 ⇒ 文件清单全线失败」的风险，得不偿失 |
| 终端可见性：自动 symlink 暴露 vs 显式触发的单个软链 | **显式触发的单个软链**（决策 12） | 自动改用户 shell profile 属越界副作用；显式动作 + 严格准入 + 事后重验，把「终端能不能直接跑」变成可验证事实 |
| 成员覆盖验收：全量 CLI 文件清单 vs 逐成员库 API 查询 | **库 API 查询**（决策 11） | 载荷几百字节 vs 现场约 230 MB；后者任何抓取上限都装不下 ⇒ 验收必失败 |
| 库 API 不可用：直接失败 vs 回退 CLI 清单 | **回退 CLI 清单 + 可审计痕迹** | 小仓照常工作；大仓仍撞 64 MiB 截断（专用码如实报告，不伪装成协议不符） |
| 索引健康异常：阻断激活 vs 只告警 | **只告警** | 保持既有「不因健康问题阻断」语义；未知 state 值宁可多一条可审计告警 |
| magpie 的「不代跑安装」 | **不采纳**（owner 澄清） | 目标不变：产品内一键安装必须真的装上并能用 |
| agent 专属环境变量：本轮不注入 vs 补进子进程 | **补进子进程**（决策 13，owner 2026-11-13 裁决「本轮做」） | 平台不是登录 shell ⇒ 用户配置目录变量天然不可见，provider 会读默认位置；「缺失才补、2s 上限、失败不阻塞」既透传意图又不夺走调用方/平台更权威的取值 |

## 明确不改的部分

| 项 | 原因 |
|---|---|
| `src/cross_cutting/{claude_code,codex,pi,kimi_code}_provider/**` | 零改动；overlay 经共享命令层自动覆盖它们的进程拉起 |
| `openspec/specs/**`（主规格） | 本轮不改；归档时由 delta 自动同步 |
| codegraph 版本门（`>=1.6.0, <2.0.0`） | 主规格 REQ-IND-01「exact version 钉定 v1.5.0」与实现的既有漂移本轮不修，归档同步时双审对齐 |
| 致命 / 降级分界（`codegraph_missing` / `codegraph_version_mismatch`） | 不动；截断码不进入致命匹配集 |
| git / ast-grep 的裸命令通道 | 与 provider CLI 安装无关，不扩大范围 |
| 用户 shell profile | 本轮不自动改；用户**显式触发**的暴露动作只建单个软链（决策 12），仍不写 `.bashrc` / `.zshrc` |
| agent 专属环境变量**清单**（收敛三项之外的变量） | 注入本身**已在本轮范围并已实施**（决策 13 / REQ-PCI-12）；本轮只收敛 `PI_CODING_AGENT_DIR` / `CODEX_HOME` / `CLAUDE_CONFIG_DIR` 三项，新增变量须显式追加清单，不擅自扩大捕获面 |
