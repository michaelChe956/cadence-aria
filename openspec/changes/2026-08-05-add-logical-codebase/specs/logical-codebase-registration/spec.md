# logical-codebase-registration Specification

## Purpose

以 Project 为逻辑代码库，批量登记同一非 git 公共父目录下的多个真实 git 仓库为成员（attach-only、预检、批次状态、幂等），并定义旧数据迁移与聚合根准入。

## ADDED Requirements

### Requirement: 逻辑代码库成员登记（REQ-REG-01）
系统 SHALL 支持将同一非 git 公共父目录下的多个真实 git 仓库登记为 Project（逻辑代码库）成员；每个成员对应一个 `RepositoryRecord`（物理 git 仓，兼容投影）与一个 `CodebaseMemberRecord`（稳定 UUID 逻辑身份 `LogicalRepositoryId`）与一个 `RepositoryCheckoutRecord`（可用 checkout）；聚合根绝不注册为 RepositoryRecord。三层身份映射：`LogicalRepositoryId` →（member 解析）→ `RepositoryCheckoutId` →（物理定位）→ `RepositoryRecord.id` + canonical_path + git_dir_identity。

#### Scenario: 扫描公共父目录登记成员
- **WHEN** 用户提供公共父目录并触发登记
- **THEN** 系统 SHALL 扫描发现全部子 git 仓，逐项校验（canonical path、git root、非嵌套、非重复），并为每个成员创建 CodebaseMemberRecord 与 RepositoryRecord，全程不修改任何仓库内容

#### Scenario: 成员身份与去重
- **WHEN** 同一仓库以不同路径或别名重复提交
- **THEN** 系统 SHALL 按 canonical git-dir/source identity 去重，不产生重复成员，并返回重复原因

### Requirement: 登记零 Git 副作用（REQ-REG-02）
登记阶段系统 SHALL NOT 执行任何 `git add`/`git commit`/`git push`，SHALL NOT 操作任何成员主 checkout；该承诺适用于登记阶段（每仓最小指针的受控发布属于受控副作用，见 session-policy-envelope REQ-ENV-07）。

#### Scenario: 登记前后仓库无变化
- **WHEN** 完成 50 仓登记后
- **THEN** 每个成员仓库的 `git status --porcelain`、HEAD、refs、worktree list、`.git/config`、hooks、index 与登记前完全一致；聚合根的 `.aria`/`.codegraph` 允许写入，成员仓内禁止写入运行文件

### Requirement: 批量预检与分类展示（REQ-REG-03）
系统 SHALL 在批量登记提交前执行预检并按类别展示：可登记 / 非 git / 重复 / 嵌套 / 脏仓 / 路径不存在 / 越界，由用户确认后才实际登记；有效项不被无效项阻塞；脏仓默认可登记但标记 `needs_attention`（可经用户显式确认后登记）。

#### Scenario: 混合清单预检
- **WHEN** 提交含有效项与非 git/重复项的 manifest
- **THEN** 预检结果 SHALL 按类别分组展示；用户确认后仅登记有效项与显式确认的脏仓

#### Scenario: 确认后的 TOCTOU 复验
- **WHEN** 用户确认到实际登记之间成员状态可能变化
- **THEN** 系统 SHALL 冻结 preflight revision，登记前对每个成员复验 canonical path 与 git root，变化者标记为需重新确认

### Requirement: 批次状态与幂等（REQ-REG-04）
系统 SHALL 提供批次状态（queued/running/partial_failed/completed/cancelled）与逐项状态（pending/skipped/completed/failed/needs_attention）、失败原因、重试计数与幂等键（project + canonical manifest digest + revision）；支持取消、重启恢复与并发批次仲裁；删除后重导使用稳定 UUID 与 tombstone/source identity 映射，不产生 ID 冲突。

#### Scenario: 相同 manifest 重放
- **WHEN** 同一 manifest 重复提交
- **THEN** 系统 SHALL 通过幂等键识别为同一批次或返回既有批次，不产生重复成员

#### Scenario: 批次中断后重启
- **WHEN** 批次执行中服务重启
- **THEN** 系统 SHALL 从持久化状态恢复，未完成项可重试，已完成项不重跑；成员已创建但 manifest 未写完成时执行补偿

#### Scenario: 删除后重导
- **WHEN** 删除成员后重新导入同一仓库
- **THEN** 系统 SHALL 通过 tombstone/source identity 映射复用或安全重建逻辑身份，不产生 ID 冲突或引用错配

### Requirement: 前端工程差异化（REQ-REG-07）
系统 SHALL 在成员 manifest 支持 `repo_type`（backend/frontend/lib）与前端初始化 profile（package.json/pnpm/Vite 探测），前端成员不套用 Java 六步初始化。

#### Scenario: 登记含前端仓库的 manifest
- **WHEN** manifest 含 repo_type=frontend 的成员
- **THEN** 系统 SHALL 按前端 profile 处理其检测与初始化命令，与 Java 后端差异化

### Requirement: 旧数据迁移与 repo_id 双读双写（REQ-REG-08）
系统 SHALL 提供既有单仓数据向逻辑代码库的迁移：单仓 Project 自动生成默认 member；`IssueRecord.repo_id` 迁移为 focus/primary 投影并在双读双写窗口内保持兼容；提供 feature flag 回退到单仓行为；历史 attempt/worktree/session 在 target 快照缺失时 display-only 或人工处置，不得静默恢复至错误仓库。

#### Scenario: 既有单仓 Project 迁移
- **WHEN** 开启逻辑代码库功能的既有单仓 Project 被访问
- **THEN** 系统 SHALL 自动为其生成默认 member 与投影，旧 Issue/Story/Design/WorkItem 均可读，不破坏既有数据

#### Scenario: feature flag 关闭回退
- **WHEN** 逻辑代码库 feature flag 关闭
- **THEN** 系统 SHALL 回退到单仓行为；已生成的多仓 Work Item/attempt 标记为兼容投影或阻塞，不静默改写到错误仓库

### Requirement: 聚合根目录所有权与准入（REQ-REG-09）
系统 SHALL 在登记前对聚合根执行准入 preflight：canonical 非 git、成员必须位于其下、拒绝路径越界/symlink 逃逸/嵌套 worktree、检测已有用户 CLAUDE.md/AGENTS.md/`.aria` 冲突、检测与其他逻辑代码库根重叠；冲突时返回稳定错误并阻塞。

#### Scenario: 聚合根准入失败
- **WHEN** 公共父目录为 git super-repo 或包含非成员目录/凭据/构建产物
- **THEN** 系统 SHALL 拒绝登记并返回分类错误（准入 preflight 范围限于聚合根属性校验；索引范围构造见 REQ-IND-02）
