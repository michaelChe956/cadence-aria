# Spec Delta: fix-aggregate-lc-registration-gates

## MODIFIED Requirements

### Requirement: 聚合根目录所有权与准入（REQ-REG-09）

系统 SHALL 在登记前对聚合根执行准入 preflight：canonical **非 git 根**、成员必须位于其下、拒绝路径越界 / symlink 逃逸 / 嵌套 worktree、**同一目录不得被第二个逻辑代码库占用（跨项目亦然；同一逻辑代码库自身的复验与重跑不在此列，软删墓碑期间仍占用该根）**。

preflight MUST NOT 因聚合根已存在 `AGENTS.md` / `CLAUDE.md` / `.claude/rules/*` / `.mcp.json` / `.gitignore` 而拒绝登记 —— 这些是 Cadence-skills 初始化的正常产物，且登记本身不写任何文件。冲突时返回稳定错误码并阻塞。

#### Scenario: 聚合根准入失败

- **WHEN** 公共父目录为 git super-repo 或包含非成员目录/凭据/构建产物
- **THEN** 系统 SHALL 拒绝登记并返回分类错误（准入 preflight 范围限于聚合根属性校验；索引范围构造见 REQ-IND-02）

#### Scenario: 已初始化过的聚合根允许登记

- **WHEN** 聚合根目录中已存在 `AGENTS.md`、`CLAUDE.md`、`.claude/rules/language.md`、`.mcp.json` 与 `.gitignore`
- **THEN** 登记预检 SHALL 通过并正常返回候选成员分类；MUST NOT 返回 `aggregate_root_ownership_conflict`

#### Scenario: 同一目录的第二个逻辑代码库被拒绝

- **WHEN** 某聚合根已被一个存活或软删（墓碑）的逻辑代码库占用，另一个逻辑代码库（无论同项目与否）尝试以该目录为聚合根登记
- **THEN** 系统 SHALL 拒绝并返回 `aggregate_root_overlap`，错误信息含被占用根与占用方身份

#### Scenario: 同一逻辑代码库的复验不被自身拦掉

- **WHEN** 同一条逻辑代码库对其既有聚合根重跑预检或聚合初始化复验
- **THEN** 系统 SHALL 通过；MUST NOT 把自身 manifest 判为重叠

## ADDED Requirements

### Requirement: 逻辑代码库生命周期：软删、恢复、彻底删除与已删除列表

系统 SHALL 提供三段式生命周期，语义 MUST 区分「可恢复」与「不可恢复」：

1. **软删**：写墓碑、从存活列表隐藏、释放成员登记；**保留**LC 子树与工作区政策正文（政策正文是恢复后的运行前提）；MAY 清理成员仓的 `.aria-pointer.md`（可再生、无恢复不变量）
2. **恢复**：摘除墓碑、恢复成员登记，id 不变、issue / 成员引用不断裂；幂等（已存活返回现状）
3. **彻底删除（硬删）**：**仅对已软删（墓碑存在）的 LC 可用**；删除 LC 子树，清理 `<聚合根>/policy/<项目>/<逻辑代码库>/` 与各成员仓 `.aria-pointer.md`；MUST NOT 触及 `AGENTS.md` / `CLAUDE.md` / `.claude/rules/*` 等 Cadence-skills 产物；MUST NOT 跨 project

列表 MUST 能带出已删除（墓碑）LC 供前端呈现（含 `deleted_at`）；不带参数时行为与既有「只返回存活」完全一致。

#### Scenario: 存活逻辑代码库不允许硬删

- **WHEN** 对一个存活的逻辑代码库请求彻底删除
- **THEN** 系统 SHALL 拒绝并返回稳定错误码 `logical_codebase_not_tombstoned`（409），且零副作用

#### Scenario: 软删后可恢复

- **WHEN** 逻辑代码库被软删后请求恢复
- **THEN** 系统 SHALL 恢复为存活，id 不变；此前发布在工作区的政策正文 MUST 仍然存在，恢复后 gateway 政策校验 MUST 通过

#### Scenario: 彻底删除清理工作区产物

- **WHEN** 对已软删的逻辑代码库请求彻底删除
- **THEN** 系统 SHALL 删除其 LC 子树、`<聚合根>/policy/<项目>/<逻辑代码库>/` 与成员仓 `.aria-pointer.md`；`AGENTS.md` / `CLAUDE.md` / `.claude/rules/*` MUST 保持原样

#### Scenario: 彻底删除幂等且不可跨项目

- **WHEN** 重复请求彻底删除，或以另一 project 的身份请求
- **THEN** 系统 SHALL 返回 NotFound，不产生任何跨项目副作用
