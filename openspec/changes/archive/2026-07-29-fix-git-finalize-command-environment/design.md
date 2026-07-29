# Design: fix-git-finalize-command-environment

## Context

真实故障：`git_finalize_commit: git command exited Some(128): Author identity unknown … unable to auto-detect email address`。因果链：

1. `git_finalize` 的所有 git 命令经 `RepositoryRegistrationCoordinator::run_git`（`registration.rs:764-783`）执行，`environment` 仅 `LC_ALL=C`；
2. `BoundedCommandRunner::run` → `ProcessManager::spawn_isolated` → `process_manager.rs:222` `env_clear()`，子进程只剩注入变量 → 无 `HOME`；
3. 目标仓库无仓库级 identity，`~/.gitconfig` 因 HOME 缺失不可读 → commit 失败；push 同样需要 `HOME`（`~/.ssh`）与 `SSH_AUTH_SOCK`；
4. 既有单测使用录制假 `GitFinalizeRunner`，真实 git + 隔离环境链路零覆盖。

已核实的结构事实：

- coordinator（`registration.rs:251-264`）目前不持有 home；构造点：`new` / `new_with_operations`；
- 生产构造在 `src/web/handlers/repository_registration.rs:196-239`，该处 `:196` 已有 `validate_user_home(self.home)?` 验证过的 home；
- `run_git` 是 coordinator 私有方法，改动影响面仅限 git_finalize 链路。

## Goals / Non-Goals

**Goals:**

- `git_finalize` 的 commit 能用宿主 `~/.gitconfig` 身份成功提交；
- push 子进程携带 `HOME` 与（存在时的）`SSH_AUTH_SOCK`，ssh 凭证可用；
- 其余变量保持隔离；注入 HOME 来自验证过的解析结果，不硬编码。

**Non-Goals:**

- 不放宽 `spawn_isolated` 的 `env_clear` 语义，不改 bounded runner；
- 不自动重试历史失败 operation（用户手动 commit/push 或重新初始化）；
- 不在目标仓库写入仓库级 git config；
- 不变更 git_finalize 的步骤语义（失败仍只产生 warning，不改变 operation 终态）。

## Decisions

### D1: 注入方式 = coordinator 持有 `git_environment` 字段

新增字段 `git_environment: BTreeMap<String, String>`，`run_git` 用它替代内联 `{LC_ALL: C}`。构造函数内部默认组装：`LC_ALL=C` + 进程环境存在时的 `HOME`、`SSH_AUTH_SOCK`；另提供 `pub(crate) fn with_git_environment(self, env: BTreeMap<String, String>) -> Self` 注入点。

理由：比起给 `new`/`new_with_operations` 加参数（测试构造点数十处， churn 大），builder 风格注入点改动最小；默认从进程环境尽力填充保证生产路径（`repository_registration.rs:226`）即使不显式调用注入点也行为正确。

备选（已否决）：构造函数加参（churn 大）；`run_git` 内每次读 `std::env`（隐式、不可测）；`run_inherited` 全量继承（放弃隔离）。

### D2: Web 层显式注入验证过的 home

`repository_registration.rs` builder 在 :196 已有 `validate_user_home(self.home)?`；构造 coordinator 后链式调用 `with_git_environment`，显式传入 `{LC_ALL: C, HOME: <验证过的 home>, SSH_AUTH_SOCK?}`。fake 运行时路径同样注入（home = workspace_root，行为与现状一致、无害）。

理由：与 spec「注入 HOME 来自 Web 层验证结果」一致；显式注入使行为不依赖进程环境的偶然性。

### D3: 测试策略（真实 git 回归）

- 新增真实链路测试（`registration/tests/cases/git_finalize.rs` 或新文件）：TempDir 造 HOME 并写入含身份的 `.gitconfig`，真实 git 初始化目标仓库（无仓库级 identity），coordinator 使用真实 `TokioBoundedCommandRunner` + `with_git_environment` 注入该 HOME，直接驱动 `git_finalize`（或经 `run_git` 执行 add/commit），断言提交成功且 `git log` 作者匹配 `.gitconfig`；
- 隔离保持测试：注入环境不含某标记变量时，断言子进程环境仅有约定键（可通过 `git var -l` 不可行，改用 helper 命令或审查注入 map 构造逻辑的单测）；
- 既有录制假 Runner 测试保持绿色（行为兼容）。

## Risks / Trade-offs

- [HOME 透传扩大子进程可见信息面] → 仅 `HOME` 与 `SSH_AUTH_SOCK` 两个键，且 git 操作本就需要它们；其余变量仍隔离。
- [用户 `.gitconfig` 使用 includeIf 等依赖更多变量] → 边缘情况；Git 主要依赖 HOME 定位配置，XDG_CONFIG_HOME 未透传时 git 回退 HOME 下路径，可接受。
- [fake 运行时注入 workspace_root 作为 HOME 改变测试行为] → fake 路径不执行真实 git_finalize（FakeCadenceSkillsPreparation 等），无实际影响。
