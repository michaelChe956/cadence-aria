# 项目个性化规则文档

## 强制约束

> **🔴 以下规则必须遵守 - 无例外**

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
- ✅ 在 `CLAUDE.md` 或 `AGENTS.md` 中添加规则引用以启用

## 项目个性化规则（强制规则）

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

### 已启用项目规则

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

- **Rust 构建/测试/检查命令规范**
  - 标准命令、`-j 1` 禁令、定向快反馈命令、性能预期等强制规范，详见 `cadence/project-rules/build-test-commands.md`。

- **Workspace 产物链路 Bug 三模块联动排查规则**
  - 后续遇到涉及 Story Spec、Design Spec、Work Item 任一产物 Workspace 的 Bug、展示异常、状态恢复异常、交互定位异常、审核/返修流程异常时，必须同时评估三种产物类型是否受影响。
  - 若三者复用同一链路，应优先在共享层修复，并补充覆盖 `story`、`design`、`work_item` 的回归测试；完整规则详见 `cadence/project-rules/workspace-artifact-bug-triage.md`。

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

## 目录说明

本目录用于存放项目个性化的规则文档，包括模板、规范、约定等。

## 使用方法

### 步骤 1：浏览示例

查看 `examples/` 目录中的示例文件，了解可以定制的内容。

### 步骤 2：创建您的规则

1. 复制 `examples/` 中的模板到本目录
2. 根据您的项目需求修改内容
3. 重命名为合适的文件名（不含 `examples/` 前缀）

### 步骤 3：在 CLAUDE.md / AGENTS.md 中启用

在项目根目录的 `CLAUDE.md` 或 `AGENTS.md` 中添加规则引用，指导 Agent 使用您的定制文档。

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

## 文件说明

### requirement-template.md
需求文档模板，定义需求文档的格式和内容结构。

### design-template.md
设计文档模板，定义设计文档的格式和内容结构。

### coding-standards.md
代码开发规范，包括命名规范、代码风格、注释规范等。

### test-standards.md
测试规范，包括测试覆盖率要求、测试类型要求、测试命名规范等。

## 提示

- 只创建您需要的规则文档，不必全部创建
- 规则文档可以根据项目需求随时调整
- 在 `CLAUDE.md` 或 `AGENTS.md` 中明确说明何时使用哪个规则文档

## 示例文件

所有示例文件都在 `examples/` 目录中，包含详细的注释和说明。
