# 项目个性化规则文档

## 强制约束

> **🔴 以下规则必须遵守**

### 规则目录划分

| 目录 | 用途 | 管理者 |
|------|------|--------|
| `.claude/rules/` | 框架内置规则文件 | 框架维护者 |
| `cadence/project-rules/` | 用户自定义规则文件 | 用户 |

### 禁止行为

- ❌ **禁止**在 `.claude/rules/` 目录中添加用户自定义规则
- ❌ **禁止**直接修改 `.claude/rules/` 目录下的框架内置规则文件
- ❌ **禁止**在项目根目录创建规则文件

### 正确做法

- ✅ 用户自定义规则放在 `cadence/project-rules/` 目录
- ✅ 从 `examples/` 目录复制模板并修改
- ✅ 在 `CLAUDE.md` 中添加规则引用以启用

## 📖 目录说明

本目录用于存放项目个性化的规则文档，包括模板、规范、约定等。

## 🎯 使用方法

### 步骤 1：浏览示例

查看 `examples/` 目录中的示例文件，了解可以定制的内容。

### 步骤 2：创建您的规则

1. 复制 `examples/` 中的模板到本目录
2. 根据您的项目需求修改内容
3. 重命名为合适的文件名（不含 `examples/` 前缀）

### 步骤 3：在 CLAUDE.md 中启用

在项目根目录的 `CLAUDE.md` 中添加规则，指导 Claude 使用您的定制文档。

**示例：**

```markdown
## 项目个性化规则

### 需求文档格式
使用 `cadence/project-rules/requirement-template.md` 作为需求文档格式，
不要使用 requirement skill 中的通用格式。

### 设计文档格式
使用 `cadence/project-rules/design-template.md` 作为设计文档模板。

### 代码开发规范
所有代码开发必须遵循 `cadence/project-rules/coding-standards.md` 中的规范。
```

## 📁 文件说明

### requirement-template.md
需求文档模板，定义需求文档的格式和内容结构。

### design-template.md
设计文档模板，定义设计文档的格式和内容结构。

### coding-standards.md
代码开发规范，包括命名规范、代码风格、注释规范等。

### test-standards.md
测试规范，包括测试覆盖率要求、测试类型要求、测试命名规范等。

## 💡 提示

- 只创建您需要的规则文档，不必全部创建
- 规则文档可以根据项目需求随时调整
- 在 `CLAUDE.md` 中明确说明何时使用哪个规则文档

## 📝 示例文件

所有示例文件都在 `examples/` 目录中，包含详细的注释和说明。

## 项目补充

> 以下为本仓库既有内容（含已启用的强制项目规则），按原顺序完整保留；标题层级整体降一级。

### 项目个性化规则（强制规则）

> **🔴 强制规则**
>
> - 用户自定义规则**只能**存放在 `cadence/project-rules/` 目录
> - **禁止**在 `.claude/rules/` 目录中添加用户自定义规则
> - **禁止**直接修改 `.claude/rules/` 目录下的框架内置规则文件
> - 框架内置规则由维护者管理，详见 `.claude/rules/README.md`

- **规则目录**：`cadence/project-rules/`
- **使用方法**：
  1. 查看项目初始化时创建的示例文件（`examples/` 目录）
  2. 根据需要复制和修改示例文件到 `project-rules/` 目录
  3. 在 `CLAUDE.md` 或 `AGENTS.md` 中添加规则引用，指导 Agent 使用您的定制文档

#### 已启用项目规则

> **🔴 以下规则已启用，必须遵守。**
>
> 本仓库已在 `AGENTS.md` 与 `CLAUDE.md` 中引用 `cadence/project-rules/README.md`，因此本节列出的规则不是示例规则，而是当前项目的强制规则。

- **Rust/Cargo 本地开发与验证规则**
  - 后续本地开发、测试与 CLI 验证必须直接使用宿主机 Rust 环境执行，不使用 Docker 作为默认开发测试环境。
  - 仓库根目录的 `rust-toolchain.toml` 是唯一工具链声明来源；进入仓库根目录或当前 worktree 根目录后直接运行 `cargo` 命令。
  - 常用本地验证命令：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo check --locked`、`cargo test --locked`。
  - 🔴 **禁止**给 `cargo test` 携带 `-j 1`；并行度由 `.cargo/config.toml` 的 `jobs = 8` 统一托管，命令行无需也不应再写 `-j`。完整命令规范、性能预期与历史背景见 `cadence/project-rules/build-test-commands.md`。
  - 定向验证 `src/lib.rs` 内的单元测试时必须限制测试目标，例如 `cargo test --locked --lib <测试过滤名>`；禁止使用 `cargo test --locked <测试过滤名>` 作为快速反馈命令，因为它仍会遍历所有 integration test 二进制。
  - `approval_bridge` 单元测试使用 `cargo test-approval-bridge`（等价于 `cargo test --locked --lib approval_bridge`），避免在 9 个单元测试通过后继续等待无关测试二进制过滤。
  - 若宿主机 Rust 工具链或组件缺失，应按 `rust-toolchain.toml` 修复宿主机环境，而不是改用 Docker 绕过。
  - 目录中旧 Docker 开发指南仅作为历史文档保留，不属于已启用项目规则。


- **Subagents 使用规范**
  - 所有会话的 subagent 派工（实施/审查/勘察/裁决/并行分组）必须遵循 `cadence/project-rules/subagent-usage.md`。
  - 核心要点：能并行的尽量并行（不设并行度上限）；计划必双审；每 Task k3 独立审查+fix loop≤5；worker 显式列文件提交；冲突回退走 stash 三步协议。详见该文件。
- **Rust 构建/测试/检查命令规范**
  - 标准命令、`-j 1` 禁令、定向快反馈命令、性能预期等强制规范，详见 `cadence/project-rules/build-test-commands.md`。

- **Workspace 产物链路 Bug 三模块联动排查规则**
  - 后续遇到涉及 Story Spec、Design Spec、Work Item 任一产物 Workspace 的 Bug、展示异常、状态恢复异常、交互定位异常、审核/返修流程异常时，必须同时评估三种产物类型是否受影响。
  - 若三者复用同一链路，应优先在共享层修复，并补充覆盖 `story`、`design`、`work_item` 的回归测试；完整规则详见 `cadence/project-rules/workspace-artifact-bug-triage.md`。

- **Work Item Draft Prompt 测试提醒规则**
  - 修改 Work Item Draft Prompt、Canonical Contract 投影或 Provider 输出结构约束时，交付前必须提醒操作者授权执行 Case A、Case B 各 10 次的真实 Claude Code 验证。
  - 不得自动调用 Provider，不得新增 CI、Hook、产品 CLI 或持久化评估报告；完整规则详见 `cadence/project-rules/work-item-draft-prompt-validation.md`。


- **测试内容 /tmp 隔离规则（强制）**
  - 一切测试产物（夹具仓/构建 target/日志/快照/补丁/E2E 环境装配）只能创建在 `/tmp` 下；禁止在仓库工作区、父级目录、家目录散落测试内容。
  - 例外：`cadence/reports/` 正式验收证据与仓内测试代码资产。会话结束清理自建 /tmp 产物；所有 subagent brief 必须携带本规则。
  - 完整规则详见 `cadence/project-rules/test-content-tmp-only.md`。

- **main 分支保护规则**
  - main 分支禁止直接 commit（受 shared `pre-commit` hook 与本规则双约束）；实施/交付一律在功能 worktree 分支进行，主仓 main 仅作合并目标；误在 main 产生的 commit 按「备份 tag → 复位 `origin/main` → cherry-pick 回功能分支」处置。完整规则详见 `cadence/project-rules/branch-protection.md`。

- **交付完整性规则（用户可操作能力 ≠ 后端接口）（强制）**
  - owner 需求措辞含「按钮 / 页面 / 界面 / 入口 / 能看到 / 能点 / 交互」时，交付物是**用户在界面上能完成这件事**，接口只是手段；只交付端点视为**未完成**。
  - `tasks.md` 必须同时列出前端实现任务 + **UI 验收用例**（含每个状态下显示哪个按钮的状态矩阵）；禁止把「提供 X 能力」等价为「新增 X 接口」。
  - k3 审查结论里的「**用户可见入口缺失 / 用户路径不可达**」**不得**归入「可接受（C 档）」，必须提到「必须先修」；若确需延后，须显式写明「本批不交付、owner 已知悉」。
  - 自测必须覆盖**真实交互面**：涉及交互路径的改动要有页面级验证（点了什么 → 看到什么），不能只用 curl/API 证明接口通。
  - 前端改动（`web/src/**`）重发前必须重建 `web/dist` **并**重建二进制；重发后必须验证页面实际加载的资源包含本次改动（判断优先级：页面实际 JS > 二进制内字符串扫描；扫描用 `grep -a`，**不要用 `strings`** —— 它默认只抽 ASCII 序列，抽不到二进制里的中文串，会误判「没编进去」）。
  - 违反任一情形视为交付未完成，不得声称完成。完整规则与检查清单详见 `cadence/project-rules/delivery-completeness.md`。

- **本地服务部署与启动规则（4317 共享实例）（强制）**
  - **只经 `/tmp/aria-deploy/start.sh` 启动 4317**，禁止手工拼命令行直起二进制 —— 脚本内的 `ARIA_API_TRUSTED_ORIGINS` 一旦丢失，owner 访问即报 `[request origin not allowed]`（端口在听但服务不可用）。
  - 必须带 TTY（`script -qfc`）：T1 鉴权 bootstrap token 只在 TTY 交付；启动日志首行应含 `TTY="/dev/pts/..."`。
  - 构建顺序：`pnpm -C web build` → `CARGO_TARGET_DIR=/tmp/deploy-target cargo build --release --locked`（前端资源由 `rust-embed` 编译期嵌入，dist 未重建等于没发）。
  - 替换二进制**之前**先备份回滚件到 `/tmp/aria-rollback-<sha>`；重启前看 session 日志最后写入时间确认实例空闲（4317 与 owner 的端到端测试共用）。
  - 启动后必查 6 项：端口在听、`GET /` 与 `/api/health` 均 200、日志含 `governance_complete=true`、`/proc/<pid>/environ` 里 `ARIA_API_TRUSTED_ORIGINS` 非空、**跑的是新件**、新代码指纹在件里。
  - 指纹扫描用 `grep -a`（二进制安全），**不要用 `strings`**（只抽 ASCII，抽不到中文串）；且必须与回滚件对照（旧件应全部不命中）。
  - 「端口在听」不等于「服务可用」；上述任一项缺失即视为部署未完成，不得声称已重发。完整规则与检查清单详见 `cadence/project-rules/local-service-deploy.md`。

**示例规则**（默认不启用，需用户主动添加）：

```markdown
### 需求文档格式
使用 `cadence/project-rules/requirement-template.md` 作为需求文档格式。

### 设计文档格式
使用 `cadence/project-rules/design-template.md` 作为设计文档模板。

### 代码开发规范
所有代码开发必须遵循 `cadence/project-rules/coding-standards.md` 中的规范。

### 测试规范
所有测试必须遵循 `cadence/project-rules/test-standards.md` 中的规范。
```
