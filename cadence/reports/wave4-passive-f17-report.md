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
