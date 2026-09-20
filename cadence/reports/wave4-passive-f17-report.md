# Wave 4 · F-17 修复报告：bwrap 沙箱按角色放开 worktree 写权限（解锁 kimi coder commit 契约）

- 会话基线：worktree `feat-b-0808-add-monorepo`（修复 commit `d716f7bb`）
- 登记来源：`cadence/reports/provider-validation-round/provider-status-ledger.md` F-17 段（wave2-2b，v27）+ `cadence/reports/2026-09-20_进度报告_阶段4完工总结_v1.0.md` §3-B（controller 义 bwrap）

## 1. 根因

kimi coder（Executor）承包契约（TDD 写路径 + `write_policy.commit_responsibility`）与只读终端沙箱系统性冲突：`sandbox.rs` `build_bwrap_args` 以 `--ro-bind root root` + `--ro-bind / /` + `--tmpfs /tmp` 构造沙箱，worktree/gitdir 终端内均不可写 → `git add/commit` 必死于 `index.lock: Read-only file system` → coder 判 plan defect（operational_blocker）→ blocked 门人工分诊（attempt 544a1b51 两掷两中：gate_0001 Write×15 后 commit 死、gate_0002 零 Write 纯 Bash 全灭）。

修复前的隐性缺陷（真机实证）：root 挂载原先位于 `--ro-bind / /` **之前**——bubblewrap 按 argv 顺序挂载，后挂的祖先挂载（`/`）会整棵遮蔽先前挂在其路径下的子挂载，root 的 bind 无论 ro/rw 均不生效（行为上恰好等效全只读，掩盖了顺序语义）。

## 2. 修复（角色感知挂载，最小放宽）

1. **`sandbox.rs` `build_bwrap_args`**：新增 `writable_root: bool` 参数——root 挂载移至 `--ro-bind / /` **之后**（顺序语义见上），`writable_root` 切换 `--bind`/`--ro-bind`；cwd fd 锚定同步 `--bind-fd`/`--ro-bind-fd`（只读锚定在 rw 根内同样会杀死锚定 cwd 下的 git 写）。宿主其余隔离边界一字不动：`--ro-bind / /`、`--proc`、`--dev`、`--tmpfs /tmp`、`--unshare-net`、`--unshare-pid`、`--die-with-parent`、`--clearenv`。
2. **`terminal.rs` `TerminalIsolation::Bubblewrap`** 新增 `writable_root` 字段；`build_terminal_command` 解构透传。
3. **`mod.rs` `isolation_for`**：仅 `AdapterRole::Executor`（coding）授予 `writable_root: true`——coder 契约面；Orchestrator 终端保持只读挂载语义，Reviewer 的 terminal 请求在 policy 层已拒（矩阵不变）。
4. **openspec delta**：`close-provider-validation` change 的 kimi spec 追加「终端 OS 级隔离（auto 模式）」MODIFIED Requirement——授权根挂载按角色区分（coding rw / 其余 ro），授权根外宿主只读两模式不变；场景含 coding 可写 worktree（F-17）与非 coding 只读两分叉。

设计取舍：角色感知 rw 优于无条件 rw——无条件会把 Orchestrator 的「terminal 只读执行」语义一并打破（policy 上 FsWrite 拒但 bash 可写文件），超出 F-17 修复面。「git 操作走沙箱外」方案弃用：丢失无网络/pid 隔离边界。

## 3. TDD 证据

- **红（fake 阶段：签名/字段已通，`build_bwrap_args` 收下 `writable_root` 但行为仍旧）**：
  - `executor_writable_root_sandbox_allows_git_commit_inside_root`（真实 bwrap 端到端）FAILED——`/tmp/work/.git: Read-only file system`，exit 1 vs 期望 0（即 F-17 原始症状复现）；
  - `isolation_grants_writable_root_to_executor_role_only` 红（ApprovalBridge 构造需 reactor，改 `#[tokio::test]`）。
- **绿（实现后）**：定向 `cargo test --locked --lib kimi_code_provider::client_services` **71 passed / 0 failed**（含 fmt 后复跑）；`cargo fmt --check` clean；`cargo clippy --locked --lib --all-features` 0 警告。
- 新增测试 5 个：
  - terminal.rs 真机三联：①`executor_writable_root_sandbox_allows_git_commit_inside_root`——Executor 沙箱内 `git init+add+commit` 全链路 exit 0 + `.git`/`f17.txt` 落盘（60s 负载不敏感口径；bwrap 缺席自动跳过）；②`read_only_root_sandbox_still_blocks_writes_inside_root`——ro 挂载下根内写仍被拒（修复前语义不变）；③`writable_root_sandbox_keeps_host_outside_root_read_only`——rw 模式下写 `/etc` 报 `Read-only file system`（宿主 ro 边界不变）。
  - mod.rs：`isolation_grants_writable_root_to_executor_role_only`——Executor→true / Orchestrator→false 角色矩阵锁。
  - sandbox.rs：`bwrap_args_writable_root_binds_root_rw_after_read_only_host`——rw 三元组 `--bind root root`、恰一个 `--ro-bind`（宿主根）、rw 挂载位于其后、`--bind-fd 42 /tmp/work`、隔离旗标全在；既有 `bwrap_args_include_isolation_flags` 强化为 ro root 三元组+挂载顺序断言。

## 4. 边界与登记

- **linked worktree（gitdir 在授权根外）**：若 coder 目标 worktree 的 gitdir 位于主仓 `.git/worktrees/` 下（根外），rw 根挂载不覆盖 gitdir，git 写仍会失败——本修复覆盖标准 `.git` 在 worktree 内的形态（aria/issues 目标仓即此形态）；linked worktree 形态如出现，登记后续。
- **root 位于 `/tmp` 下**：字面 root 挂载被 `--tmpfs /tmp` 遮蔽（修复前即如此），但 cwd fd 锚定（`--bind-fd FD /tmp/work`）在 rw 模式下提供完整可写工作目录（真机验证 git commit 成功）——非回归。
- kimi 升级收口（DEF-PVR-ALL 三键）前置已齐：本修复解除 bwrap 面后可重跑 attempt 收口（F-16/F-19 广播/F-15 权限预检均已修）。

## 5. 变更文件



- `src/cross_cutting/kimi_code_provider/client_services/sandbox.rs`（挂载构造+顺序修正+2 测试）
- `src/cross_cutting/kimi_code_provider/client_services/terminal.rs`（isolation 字段+透传+真机三联测试）
- `src/cross_cutting/kimi_code_provider/client_services/mod.rs`（isolation_for 角色感知+矩阵锁测试）
- `openspec/changes/close-provider-validation/specs/kimi-acp-client-services/spec.md`（delta：终端 OS 级隔离 Requirement 角色感知修订）

验收建议：v29 轮重跑 kimi coding attempt（issue_0286 口径），coder 终端内 `git add/commit` 应成功（Execution 事件可见），blocked 门掷骰面消失；到达 internal_pr_review_complete 即三键齐升级转正。

## 6. fix round 1（k3 审 1×P1+1×P2）

- **P1（生产形态未覆盖）**：aria coding attempt 的 worktree 是 git linked worktree（`.git` 为指针文件，指向授权根外 `<repo>/.git/worktrees/<name>`）——首轮 rw root bind 只覆盖授权根，gitdir/index/objects 仍在只读宿主根下 → `git add/commit` 仍死（真机复现：`fatal: Unable to create '<repo>/.git/worktrees/<name>/index.lock': Read-only file system`，即 attempt 544a1b51 现场死法）。
- **P2（测试形态错）**：首轮主证据用 `git init` 建仓（标准 `.git` 在根内）——非生产形态。

### 修复

1. **`sandbox.rs` 新增 `resolve_writable_git_paths(root)`**：服务端以 `git rev-parse --absolute-git-dir --git-common-dir` 解析（trusted git、canonicalize）；过滤授权根内路径（标准仓 `.git` 在根内，rw root bind 已覆盖 → 空）；linked worktree 返回根外 commondir 一条（gitdir 是其后代，一条 bind 覆盖两者）；非 git 目录返回空。防指向论证：沙箱内 coder 只能把 `.git` 指针改写为它自己可写的位置（=授权根内 → 被过滤）或本就合法的主仓 gitdir（本 bind 的目标面）；其余形态 `rev-parse` 直接失败 → 无法把额外 bind 瞄准任意宿主路径。
2. **`build_bwrap_args` 增加 `writable_binds: &[PathBuf]`**：在 rw root bind 之后（同样在 `--ro-bind / /` 之后）对每个路径追加 `--bind path path`。附带收益：`/tmp` 下的 gitdir 路径（被私有 tmpfs 遮蔽不可见）经 bind 恢复可见性。
3. **`TerminalIsolation::Bubblewrap` 增加 `writable_git_paths: Vec<PathBuf>`**；`mod.rs` `isolation_for` 仅 Executor（writable_root）时解析填充，其他角色恒空——角色感知语义不变。

### TDD 证据（fix round 1）

- 红（bind 未挂载的 fake 阶段）：`executor_writable_root_sandbox_allows_git_commit_in_linked_worktree`（生产形态主证据：宿主 `git init main` + `git worktree add ../wt`，root=wt，git_paths 经真实 `resolve_writable_git_paths` 解析并断言 `== [main/.git]`）FAILED——沙箱内 `fatal: not a git repository: (null)`（测试 tempdir 在 /tmp 下，gitdir 指针的绝对路径被私有 tmpfs 遮蔽而不可达；生产 home 路径形态死法为 index.lock Read-only，同一根因两面）；`bwrap_args_bind_extra_writable_git_paths_after_ro_host` FAILED（argv 无额外 bind）。
- 绿（实现后）：定向 `cargo test --locked --lib kimi_code_provider::client_services` **74 passed / 0 failed**；fmt clean；clippy 0 警告。
- 新增测试 3 个：①linked worktree 主证据（P2 形态：worktree add 建仓 → 沙箱内 `git add+commit` exit 0 + `WT_COMMIT_OK` + 宿主 `git log f17wt` 可见——gitdir 经 commondir bind 可达且可写）；②`bwrap_args_bind_extra_writable_git_paths_after_ro_host`（恰两个 `--bind`：root+git 路径，均位于 `--ro-bind / /` 之后，git 三元组 `--bind /repo/.git /repo/.git`，隔离旗标不变）；③`resolve_writable_git_paths_covers_linked_worktree_plain_and_non_git`（非 git 空 / 标准仓空 / linked 返回根外 commondir 一条，真机 git）。
- 既有首轮测试全部不动仍绿（标准仓主证据改用真实 resolve 填充，行为等价；两把安全边界回归锁同绿）。

### 边界（fix round 1）

- commondir bind 使沙箱可写面扩至主仓 `.git`（objects/refs 共享区）——coder commit 契约的必要面；其他 worktree 的共享 git 数据在同一 `.git` 下，可写性伴随（生产单 attempt 独立主仓，实际影响面=本仓）。
- 生产 aria 形态 gitdir 位于 home（非 /tmp），可见性由 ro-bind / / 天然提供，bind 仅补可写性；/tmp 下测试形态 bind 同时补可见性（两形态均真机验证）。
- fix round 1 变更文件：`sandbox.rs`（resolve+bind+2 测试）/ `terminal.rs`（isolation 字段+主证据测试）/ `mod.rs`（isolation_for 解析）。

## 7. fix round 2（k3 审 1×P1——安全洞）

**P1（bind 面可被引导）**：`isolation_for` 每条 terminal 命令重跑 `resolve_writable_git_paths`，而 root 在 Executor 沙箱内 rw——coder 可改写 `.git` 指针指向宿主上任一已存在的合法 git 仓（无需宿主写权，指针目标只需让 `rev-parse` 成功），下一条命令的 rev-parse 即解析到该仓 → 额外 rw bind 被瞄准（首轮「防指向论证」假设目标须为 coder 可写位置，漏掉了「已存在合法仓」这一类）。

### 修复

git bind 面在**构造期冻结**：`ClientServiceState` 新增 `writable_git_paths` 字段，`KimiClientServiceDispatcher::new` 时（root 由 Aria worktree 准备产出、coder 未接手，指针可信）经 `writable_git_paths_for(role, root)` 解析一次存入；`isolation_for` 只 `clone` 缓存，不再执行 `rev-parse`。角色判断保留（`writable_root` 由 policy.role 恒定求值，无漂移面）。

### TDD 证据（fix round 2）

- 红（isolation_for 仍每命令解析的中间态）：`isolation_git_binds_frozen_at_construction_survive_pointer_swap` FAILED——宿主 `git worktree add` 造真实形态（root=linked wt，构造期断言 `==[main/.git]`），改写 `.git` 指针指向另一合法仓 `evil/.git` 后再次 `isolation_for` 返回 `[evil/.git]`（bind 面漂移，安全洞精确复现：`left: [evil/.git] right: [main/.git]`）。
- 绿（读缓存后）：定向 `cargo test --locked --lib kimi_code_provider::client_services` **75 passed / 0 failed**；fmt clean；clippy 0 警告。既有 74 测试（含 fix round 1 全部）不动仍绿。

### 残余攻击面评估

- 指针交换仅在「构造期之前」有意义——届时 root 尚未交给 coder，Aria worktree 准备流程产出可信；构造期之后任何 `.git` 改写不影响已冻结的 bind 面（沙箱内 git 仍按指针工作，但 rw bind 面恒定，改写只能让 coder 自己的 git 失效，无法扩大写面）。
- `writable_root`（root 本身的 rw 挂载）不依赖 rev-parse，无同类漂移面。
