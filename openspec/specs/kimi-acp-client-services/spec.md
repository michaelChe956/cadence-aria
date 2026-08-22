# kimi-acp-client-services Specification

## Purpose

约束 kimi ACP adapter 作为 ACP 客户端必须提供给 kimi 服务端的宿主能力（终端执行、文件读写、MCP 受控注入），使其 bash/grep/文件工具在 aria 编排下可用，且不突破权限与沙箱边界。

## Requirements

### Requirement: wire 契约冻结（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

实现前 SHALL 以真实 kimi ACP transcript 固化每种 server→client 请求的 JSON fixture（method、params、result、error、输出通道），fixture 作为集成测试基线；实现 SHALL 不偏离 fixture。fixture SHALL 在专用非敏感 fixture worktree 采集，或字段级脱敏（保持 wire shape），禁止提交真实凭据/绝对路径/工作区内容，并对提交 fixture 做 secret 扫描与脱敏断言。

#### Scenario: fixture 驱动测试
- **WHEN** 实现反向请求处理
- **THEN** fake kimi peer 按 fixture 回放请求，断言响应结构逐字段一致

#### Scenario: fixture 脱敏
- **WHEN** fixture 提交前扫描
- **THEN** 不含真实凭据/绝对路径/敏感内容；无法脱敏的字段以占位符替代且保持结构

### Requirement: ACP 能力声明与请求分发（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

kimi adapter SHALL 在 `initialize` 声明 `clientCapabilities`：`fs.readTextFile=true`、`fs.writeTextFile=true`、`terminal=true`（kimi 布尔 dialect，作为 adapter 内常量，不进通用 ACP 层）。反向请求 SHALL 经独立 `KimiClientServiceDispatcher` 分发：每个请求快速派发到并发任务，不阻塞主 prompt 响应队列；`wait_for_exit` SHALL 与 `kill`/`release`/会话取消并发。

#### Scenario: bash/grep 可用
- **WHEN** kimi 会话内调用 bash 或 grep
- **THEN** 请求被分发执行并回传真实结果，无 "ACP terminal capability is unavailable"

#### Scenario: 并发不死锁
- **WHEN** `wait_for_exit` 挂起期间收到 `kill`
- **THEN** kill 生效，wait 以被终止状态返回，后续请求继续处理

### Requirement: 终端生命周期与资源边界（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

terminal 状态机 SHALL 为：created→running→(exited|killed)→released；重复 kill/release 幂等成功；未知 terminalId 返回可区分错误。执行方式 SHALL 默认 pipe + 独立 stdout/stderr + 进程组 + 每命令超时；仅当 fixture 证明 kimi 依赖 TTY 时引入 pty（`TERM=dumb`、窗口与 EOF 行为显式设定），运行时模式记入 capability/审计，不做静默降级。资源上限（默认值，常量可配置）：每会话最大并发终端数 `MAX_TERMINALS=4`（第 1~4 个成功，第 5 个拒绝）、单终端最大输出字节 `MAX_TERMINAL_OUTPUT_BYTES=1048576`（stdout+stderr 合计；恰为 1048576 完整返回，1048577 截断并只标记一次）、命令超时 `TERMINAL_COMMAND_TIMEOUT_SECS=120`（超时后进程组终止且 RPC 队列继续服务）；输出回传 backpressure 不阻塞 RPC 队列；会话结束/取消时按进程组清理全部终端，无残留进程。终端执行作为 execution event 记录供用户审计。

#### Scenario: 正常链路
- **WHEN** create→输出流→wait_for_exit→release
- **THEN** 全链路成功，事件时间线可见命令与输出摘要

#### Scenario: 清理
- **WHEN** 会话取消时存在运行中终端
- **THEN** 进程组被终止，无残留子进程

### Requirement: 终端执行策略与每命令封闭 grammar（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

terminal 执行 SHALL 由服务端按子命令构造 argv（模型不提供可歧义位置参数）；用户提供的值 SHALL 经词法校验：路径/模式/ref 等操作数不得以 `-` 开头（除非前有固定位置的 `--` 分隔符）、`<N>` 必须为数字；二进制 SHALL 从可信绝对路径或固定可信 `PATH` 解析，不继承调用方 `PATH`。仅接受以下封闭模板（模板外一律拒绝）：

- `git`：`git --no-pager status [--short]`、`git --no-pager log [--oneline] [-n <N>] [--] [<path>...]`、`git --no-pager diff --no-ext-diff --no-textconv [--stat|--name-only] [--] [<path>...]`、`git --no-pager show [<ref>] [--] [<path>...]`（`--no-ext-diff`/`--no-textconv` 仅 diff 使用）；禁止 `-C`/`--git-dir`/`--work-tree`/`--config`/`-c`/`--output` 及一切写出/执行选项。
- `rg`：`rg [-n|-l|-i|--no-heading] [-g <glob>] -- <pattern> <path>...`（`--` 固定存在，pattern 与 path 位置确定）；禁止 `--pre`/`--pre-glob`/`--follow`/`-L`。
- `find`：`find <path> [-name <glob>] [-type <t>] [-maxdepth <N>] [-mindepth <N>] [-print]`；禁止 `-exec`/`-execdir`/`-ok`/`-okdir`/`-delete`/`-fprint`/`-fprint0`/`-L`。
- `sed`：`sed -n '<script>' <file>`，script 仅允许 `p`/`=`/`d` 指令；禁止 `-i`/`--in-place` 与 `e`/`r`/`w` 指令。
- `grep`：`grep <pattern> <file>...`（无 `-R`/`-r`）。
- `cat`/`ls`/`head`/`tail`/`wc`：`<cmd> <path>...`（`head`/`tail` 允许 `-n <N>`）。

统一约束：拒绝重定向写（`>`/`>>`/管道写文件）；所有路径操作数 SHALL 在授权根内且 no-follow；任何白名单外命令、选项或模板违反 SHALL 拒绝执行并回传错误（grammar 对 auto 与 supervised 均强制执行；supervised 不提供任意命令执行的旁路，其 ApprovalBridge 只用于批准「合规模板内」的命令执行）。

git 配置隔离：SHALL 使用经验证的只读、净化 gitdir/config 视图或受保护配置层强制禁用所有可执行本地配置项（尤其 `core.fsmonitor`/`core.externalDiff`/`core.textconv`/自定义 helper），并设置 `GIT_CONFIG_NOSYSTEM=1`、`GIT_CONFIG_GLOBAL=/dev/null`、`GIT_CONFIG_SYSTEM=/dev/null`、`GIT_CONFIG_COUNT=0`，清除 `GIT_EXTERNAL_DIFF`/`GIT_TEXT_CONV`/`GIT_PAGER`/`GIT_SSH`/`GIT_SSH_COMMAND`；子进程 SHALL 以 `clearenv` + 显式环境 allowlist 运行（不含任何仓库本地 config 注入）；固定 `--no-pager`（git 全局选项，置于子命令前）。

#### Scenario: 白名单内只读命令
- **WHEN** auto 模式收到 `git status` 或 `rg <pattern>`
- **THEN** 在只读隔离根内执行并回传结果

#### Scenario: 越界与执行型 flag 拒绝
- **WHEN** auto 或 supervised 模式收到 `cat /etc/passwd`、`git -C ~/.ssh log`、`git diff --output=x`、`find . -exec sh -c ... \;`、`sed -n -i '1p'`、`sed -n '1e ...'`、`rg --pre ...`、`rg --follow`、`grep -R`、`find -L`、`>` 写文件、或 option-smuggling（以 `-` 开头的位置参数）
- **THEN** 拒绝执行（grammar 对 auto/supervised 均强制，不因审批放宽），回传错误

#### Scenario: 本地 fsmonitor 探针
- **WHEN** 仓库 `.git/config` 配置 `core.fsmonitor=<程序>` 且执行 `git status`
- **THEN** 本地配置执行型项被禁用或忽略，探针程序不启动（子进程 clearenv + 环境 allowlist）

### Requirement: 终端 OS 级隔离（auto 模式）（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

auto 权限模式下，terminal 执行 SHALL 在 OS 级隔离中运行（优先 bubblewrap/bwrap：以授权根为只读 bind-mount、只读根文件系统、无网络、有限 /tmp、无新特权），从根本上消除外部工具重新按字符串打开路径的 TOCTOU；bwrap 不可用时，auto 模式 SHALL 拒绝一切 terminal 请求（fail-closed），仅 supervised 模式可经 ApprovalBridge 在非隔离下执行（并保留路径/grammar 校验作为 defense-in-depth）。cwd 锚定 SHALL 在隔离根内（fchdir 到已验证目录 FD）。隔离可用性探测结果与每次执行模式 SHALL 记入 capability/审计。

#### Scenario: bwrap 可用
- **WHEN** auto 模式且 bwrap 探测成功
- **THEN** 命令在只读隔离根内执行，根外不可见/不可写、网络隔离、授权根不可写；cwd 在路径验证后即使被替换为 symlink 仍锚定到已验证目录 FD，不逃逸

#### Scenario: bwrap 不可用
- **WHEN** auto 模式且 bwrap 探测失败
- **THEN** terminal 请求被拒绝；supervised 模式经 ApprovalBridge 执行合规模板内命令并记录非隔离模式

### Requirement: 权限与路径沙箱（角色与 fs）（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

所有 terminal/fs 请求 SHALL 经过当前 `ProviderPermissionMode` 与角色 action policy，不因 initialize 声明能力即执行：supervised 模式下经 ApprovalBridge 征求用户；reviewer 角色默认拒绝 terminal 执行与 fs 写；coding 角色限制在目标 worktree。fs 读写路径 SHALL 在授权根内，并用 no-follow 原子语义（`openat`+`O_NOFOLLOW` 等价封装，处理新建文件父目录校验）杜绝 TOCTOU；拒绝绝对越界、`..`、symlink 逃逸；权限拒绝与越界路径返回错误，不执行。

#### Scenario: reviewer 只读
- **WHEN** reviewer 会话的 kimi 发起 terminal/create 或 fs/write
- **THEN** 默认拒绝并返回错误

#### Scenario: 越界拒绝
- **WHEN** fs 请求路径经 symlink 指向授权根外
- **THEN** 拒绝，不读取；同类用例覆盖新建文件父目录越界与 symlink 竞态

### Requirement: MCP 受控注入（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

`session/new` 与 `session/load` 的 `mcpServers` SHALL 来自经 session policy envelope 校验的 Aria-owned 受控 bundle（allowlist、配置 digest、凭据引用脱敏、argv 审计），不从普通 provider 配置透传任意 JSON；未配置时保持 `[]`。resume（session/load）时 SHALL 校验配置 digest：不一致则拒绝加载并报告，随后启动新会话（`session/new`）且旧会话标记 superseded（对齐 `session-policy-envelope` REQ-ENV-04），不静默沿用。

#### Scenario: 受控注入
- **WHEN** envelope 提供 CodeGraph bundle 且 `.codegraph` 存在、路径/命令/版本通过校验
- **THEN** kimi 会话可调用 codegraph 工具，argv 与 digest 记入审计

#### Scenario: resume 配置漂移
- **WHEN** session/load 时 bundle digest 与冻结值不一致
- **THEN** 拒绝加载并报告差异，启动新会话且旧会话标记 superseded
