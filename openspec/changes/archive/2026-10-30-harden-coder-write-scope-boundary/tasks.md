# Tasks: harden-coder-write-scope-boundary

> 状态：**实现完成**（2026-10-30）。回归项见 §5，独立审查结论见 §6.2。

## 1. 写入范围回到编译后 Work Item 上下文（唯一载体）

- [x] 1.1 `compiled_work_item_markdown` 渲染规范范围段：标题 `## 写入范围（硬边界）` + 5 条元规则（判定基准 / 匹配语义非 glob / 多仓 attempt / 空列表语义 / 默认拒绝句）+ 两个子节列表（逐字取 record）
- [x] 1.2 空列表时两个子节分别写 `（未声明：exclusive 为空 = 不限制可写范围）` / `（未声明：forbidden 为空 = 无额外禁止）`
- [x] 1.3 定义共享常量 `WRITE_SCOPE_SECTION_HEADING`（渲染器与 coder Prompt 兜底判定共用，改名两处同步）；子节标题另立 `WRITE_SCOPE_EXCLUSIVE_LIST_HEADING` / `WRITE_SCOPE_FORBIDDEN_LIST_HEADING`
- [x] 1.4 撤销 `CodingExecutionContext` 的 `exclusive_write_scopes` / `forbidden_write_scopes` 两字段及 `load_work_item_write_scopes` / `WorkItemWriteScopes`；撤回各测试文件为它们加的 `..Default::default()`
- [x] 1.5 `prompts.rs::write_scope_section` 改为：markdown 含共享标题 → 输出空串；否则输出 fail-closed 兜底句（逐字）；`build_coding_delta_prompt` 不追加
- [x] 1.6 group 路径 `bound_coder_work_item_context` 追加同一标题段（数据取 `projection.write_policy`，与 `gates/schema_v2.rs` 同源）
- [x] 1.7 用例：`compiled_work_item_markdown` 单测（非空 / 两列表皆空 / 多仓成员前缀项）+ group 带标题段用例
- [x] 1.8 用例：coder 与 reviewer 的 Prompt 内范围段**恰好一次**；fail-closed 时含兜底句且不含 `## 写入范围`

## 1b. group 主路径的范围语义（k3 审查 F1 追加）

> k3 独立审查发现：group（schema v2）主路径走投影渲染，`build_coding_prompt` **永不被调用** →
> §1 的范围段进不了 group 的 coder/reviewer prompt → **契约违反**。已补齐。

- [x] 1b.1 抽出唯一渲染器 `write_scope_section_text(exclusive, forbidden)`（legacy markdown 与 group prompt 共用，杜绝两处文案漂移）
- [x] 1b.2 `unit_run_write_scope_section(attempt, role)`：group 取 `coder_projection.write_policy` / `reviewer_projection.scope_policy`（同为 `canonical_contract.write_policy` 克隆 = 机器门同一取值）；非 group 返回 `None`
- [x] 1b.3 group coder 最终 prompt 追加该段（`coding.rs`，位置在 render→bind 哈希之后，**哈希面不受影响**）
- [x] 1b.4 group reviewer prompt 追加（新增 `build_reviewer_prompt` 统一 group 渲染路径与 legacy 回落路径）
- [x] 1b.5 返修轮 full 臂同源追加（`rework.rs`）；delta 轮**不追加**
- [x] 1b.6 用例（`tests/write_scope_prompt.rs`）：coder 段逐字一致 + 唯一份 + **不进哈希 envelope**；reviewer 段；delta 不带段；组级终审（markdown 路径）自带该段

## 2. 本地产物写入仓库本地 exclude

- [x] 2.1 新建 `src/product/local_artifacts.rs`：`PLATFORM_LOCAL_ARTIFACT_DIRS`（`.aria` / `.codegraph` / `.verify` / `.worktrees`）+ `is_platform_local_artifact_path` + `exclude_lines()` + `notice_text()`
- [x] 2.2 `evidence_token.rs::ensure_repo_exclude` 改用单源的 `exclude_lines()`；落点改为 `git rev-parse --git-common-dir`（linked worktree 的 `.git` 是文件）
- [x] 2.3 并发安全：对 common gitdir 加文件锁（复用 `coding_attempt_store/locking.rs` 的 `ExclusiveFileLock::acquire_at`，锁文件 `<common gitdir>/aria-exclude.lock`），持锁范围覆盖「读 → 算缺失 → 写」；写用 temp + `rename` 原子替换
- [x] 2.4 保持幂等：只追加缺失行，`.aria` 与 `.aria/` 等价，仓库既有行与注释原样保留
- [x] 2.5 调用顺序：`issue_evidence_token` 先落 exclude 再写令牌文件（令牌写失败不连带跳过 exclude）
- [x] 2.6 用例：真实 git 仓端到端（`git status` 干净 + `.gitignore` 字节不变）、`git worktree add` 端到端、幂等三连调、用户内容保留、linked worktree 落点、并发互斥、notice 单行且无 temp 残留

## 3. 单一事实源与提交过滤

- [x] 3.1 `git_workspace_service.rs::should_exclude_from_work_item_commit` 改用 `is_platform_local_artifact_path`，并补上 `.aria/evidence-token`、`.codegraph/`、`.verify/`、`.worktrees/` 漏网路径（含嵌套层级）
- [x] 3.2 `__pycache__` / `*.pyc` 仅留在提交过滤（用户代码产物，不上升为仓库级 exclude 策略）
- [x] 3.3 用例：令牌文件不会被 `git add -A` 暂存进 Work Item 提交
- [x] 3.4 **（k3 审查 F3 追加）**`is_platform_local_artifact_path` 与 git 尾斜杠目录语义对齐：同名组件**非末段**才命中；末段带尾斜杠（git 目录形态）视为目录同样命中。用例覆盖 `.verify`（同名普通文件）不命中、`.aria/evidence-token` 命中

## 4. Prompt 告知句

- [x] 4.1 平台指令段（`platform_directives_section`）追加告知句：平台已在 worktree 准备阶段把 4 个本地产物目录写入 `info/exclude` → 不需要也不得为它们改 `.gitignore`；`.gitignore` 反选时仍不得由 agent 修改；其他未跟踪文件同样不得擅自加入
- [x] 4.2 目录清单**运行时从单源常量派生**（不写死字面量，清单变更自动跟随）
- [x] 4.3 用例：共享断言 `assert_platform_directives` 覆盖三个调用点（段本身 / group 渲染路径 / legacy 兜底路径）
- [x] 4.4 **（k3 审查 F2 追加）**reviewer 协议锚点与材料标题同源：协议句点名材料实际段标题（`WRITE_SCOPE_FORBIDDEN_LIST_HEADING`），旧称作括注保留

## 5. 回归与门禁

- [x] 5.1 三个既有失败用例断言**一字不改**地转绿：`evaluation_context_uses_compiled_work_item_without_artifact_version`、`code_review_prompt_uses_compiled_work_item_without_artifact_version`、`coding_execution_context_uses_final_compile_work_item_when_workspace_artifact_missing`
- [x] 5.2 `cargo test --locked --lib` 全绿、`cargo test --locked --test it_core` / `--test it_web` 全绿
- [x] 5.3 `cargo clippy --all-targets --all-features --locked -- -D warnings` 零告警、`cargo fmt --all -- --check` 干净
- [x] 5.4 `cargo test --locked --test it_core large_file_guard` 绿（单文件行数上限）
- [x] 5.5 前端 test / build 绿（245 文件 / 2483 用例；`pnpm build` 通过）
- [ ] 5.6 归档前最终重跑（controller 在归档前统一跑一次，上面的数字为实现完成时的结果）

## 6. 契约与留痕

- [x] 6.1 本 change 的 proposal / design / 两份 spec delta / tasks（`openspec validate --strict` exit 0）
- [x] 6.2 独立审查结论：k3 双裁定（spec 合规 10 条 / 质量 3 条缺陷 F1·F2·F3），**3 条缺陷已全部修复**并补入契约场景
- [x] 6.3 独立票据：`cadence/notes/2026-10-30_独立票据_写入范围判定两处既有缺陷_v1.0.md`（`**/x` 全放行、门禁硬编码大小写敏感 —— 判定语义缺陷，不混入本 change）
- [x] 6.4 归档清账台账：`cadence/notes/2026-10-30_待办_活跃change归档清账_v1.0.md`（其余 17 个活跃 change 的准确缺口 + 分批建议）

## 7. 真机复验

- [ ] 7.1 重发后跑一次完整 Work Item，确认 coder **不再修改 `.gitignore`**，`git status` 无本地产物噪音
- [ ] 7.2 确认 reviewer 能把越界文件报成 finding
- [ ] 7.3 确认 group（多仓）路径的 coder/reviewer 确实看到范围规范段
