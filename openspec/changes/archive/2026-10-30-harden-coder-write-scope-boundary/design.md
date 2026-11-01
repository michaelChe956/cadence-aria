# 设计：coder 写入范围可见性与工作树本地产物屏蔽

## 1. 范围信息的唯一载体：编译后 Work Item 上下文

### 1.1 数据通路（实测）

| 消费方 | 通路 | 是否经过 `CodingExecutionContext` |
|---|---|---|
| coder Prompt | `build_coding_prompt` → `prompts.rs` 嵌「已确认 Work Item」段 | **是**（额外挂了一段 coder 专用范围渲染） |
| reviewer Prompt | `build_code_review_prompt`（`prompts.rs`）→ `work_item_markdown_for_attempt` | **否** |
| 组级 review Prompt | 同上 | **否** |
| 评估上下文 | `coding_evaluation_context/builder.rs` → `load_coding_work_item_context` | **否** |

→ **范围信息若只经 `CodingExecutionContext` 携带，reviewer 与评估上下文将失去范围**（越界审查能力被删）。这正是「新增 coder 专用字段 + 删 markdown 旧段」方案的致命缺陷。

### 1.2 裁定：范围段回到编译后 Work Item 上下文

- **承载点**：`compiled_work_item_markdown`（`src/product/coding_work_item_context.rs`）
- **标题**：共享常量 `WRITE_SCOPE_SECTION_HEADING = "写入范围（硬边界）"`，渲染器与 coder Prompt 的兜底判定**共用同一常量**（改名必须两处同步）
- **三方同源**：coder / reviewer / 评估上下文都读同一份 markdown → 同一 Prompt 内不会出现第二份范围

**否决的备选**

| 备选 | 否决理由 |
|---|---|
| 新增 `CodingExecutionContext` 字段 + 删 markdown 旧段 | reviewer / 评估上下文拿不到范围 → 越界审查能力丢失（已实证为 3 个用例红） |
| 新增字段 + 保留 markdown 旧段 + 在 coder Prompt 里字符串抑制 | 抑制逻辑与标题文案强耦合，标题一改即静默失效；coder Prompt 内仍存在「两份来源」的执行路径 |
| 新增字段 + 保留 markdown 旧段不抑制 | 同一 Prompt 两份范围，agent 读到两套口径 |
| 给 reviewer 单独加一段范围 | 单一数据源但两个渲染点，措辞漂移只能靠双份快照测试约束；reviewer 本来就已拿到 markdown，再插一段等于同一 Prompt 两份范围 |

### 1.3 机器门禁与文案的对应（文案不得与判定脱节）

| 判定事实 | 位置 | Prompt 文案 |
|---|---|---|
| 基准 = worktree 根相对路径 | `cross_cutting/worktree.rs` `validate_write_path` → `normalize_relative_path` | 「判定基准 = 工作树根相对路径，即 `git status --porcelain` 输出的路径」 |
| 前缀匹配，非 glob | `worktree.rs` `ScopePattern::parse` | 「前缀匹配，不是 shell glob 展开…含 `*` 的项取第一个 `*` 之前的前缀做 starts_with…`**/x` 机器判定实际等于不限制」 |
| 空列表语义不对称 | `gates.rs` `validate_changed_files_for_work_item` | 「exclusive 为空 = 不限制可写范围；forbidden 为空 = 无额外禁止」 |
| forbidden 优先 | 同上 | 「命中任一 forbidden 项即违规，即使同时命中 exclusive」 |

**取值为空或记录缺失时，MUST NOT 渲染成「无限制」**：新增字段默认空 → 构造点遗漏会渲染成「不限制」，比现状更危险。裁定采用**标题缺失判定**：markdown 不含共享标题常量 → coder Prompt 追加 fail-closed 兜底句。

## 2. 本地产物屏蔽：本地 exclude，不动 `.gitignore`

### 2.1 为什么是 `info/exclude`

- `.gitignore` 是**被跟踪文件** → 改它 = 改仓库内容 = **正是要消灭的那个越界动作本身**
- `info/exclude` 在**版本库之外** → 写它不产生 diff，agent 看不到「谁动了配置」
- 优先级：命令行 > `.gitignore` > `$GIT_DIR/info/exclude` > `core.excludesFile` → **`.gitignore` 优先级更高**，目标仓可用 `!` 反选，所以告知句必须带兜底

### 2.2 落点：common gitdir

`info/exclude` MUST 写在 **common gitdir**（`git rev-parse --path-format=absolute --git-common-dir`），一次写入对主检出与全部 linked worktree 生效。

**不用** `<repo>/.git/info/exclude` 硬编码路径：linked worktree 的 `.git` 是**文件**不是目录，硬编码会静默降级（写入失败 → exclude 缺失 → 故障复现）。

### 2.3 并发：同仓 common gitdir 共享 → 必须串行化

多个工作树可被并发创建 → 读-改-写会丢更新或产生截断。

- 锁文件：`<common gitdir>/aria-exclude.lock`（sidecar）。**不锁 exclude 文件本身**（首次创建会换 inode）
- 持锁范围：「读全文 → 计算缺失 → 写回」的整个临界区
- 写：temp 文件 + `rename` 原子替换
- 实现复用仓内既有的文件锁助手（`src/product/coding_attempt_store/locking.rs`），不引入新依赖

### 2.4 幂等

只追加**缺失**的行；逐行比较时把 `.aria` 与 `.aria/` 视为等价；仓库既有行与注释原样保留。

## 3. 单一事实源

新建 `src/product/local_artifacts.rs`：

- `PLATFORM_LOCAL_ARTIFACT_DIRS`（4 项：`.aria` / `.codegraph` / `.verify` / `.worktrees`）
- `is_platform_local_artifact_path(path)` —— exclude 写入与**提交过滤**共用
- `exclude_lines()` —— exclude 写入用
- `notice_text()` —— Prompt 告知句用（见 §4）

**语义边界**：4 项是**平台自产**目录 → exclude 与提交过滤都查它。`__pycache__` / `*.pyc` 是**用户代码产物**，只留在提交过滤（`.gitignore` 已覆盖，不上升为仓库级 exclude 策略）。

**必须修的实际缺陷**：现提交过滤只匹配 `.aria` 与 `.aria/coding-artifacts/`，**不匹配 `.aria/evidence-token`** → exclude 一旦失效，`git add -A` 会把令牌文件暂存进 Work Item 提交。

## 4. 告知句（P1-3）

放在**平台指令段**——它是唯一在 full / delta / rework 四条路径**无条件**追加的段（coder Prompt、delta Prompt、rework Prompt 共用），且在 render→bind 哈希之后，group 路径也覆盖。

**措辞要点**

1. 陈述平台已做的事（写入 `info/exclude` 的 4 个目录）——对**任意目标仓**都成立（即使目标仓 `.gitignore` 没有这些行）
2. 明确「**不需要也不得**为它们修改 `.gitignore`」
3. 带兜底：若目标仓 `.gitignore` 反选了这些目录（`info/exclude` 优先级更低），**仍不得由 agent 修改 `.gitignore`**，应在交付说明中标注由平台处理
4. 明确「其他未跟踪文件属于仓库工作内容，同样不得擅自加入 `.gitignore`」

**与 fail-closed 兜底句不冲突**：告知句陈述的是平台动作事实，兜底句是禁令，二者相容。

## 5. 覆盖面裁定

平台本地产物清单**只含 4 项**。实测 `git ls-files`：`cadence` 1519、`openspec` 591、`.claude` 52、`.agents` 18、`.codex` 38、`.pi` 12、`.kimi-code` 6、`.omp` 1 —— 全部**已跟踪**，不进入 untracked 列表，**不是 `git status` 噪音源**；把它们纳入 exclude 会让新建文档从 `git status` / `git add -A` 消失。

`.codegraph/`、`.worktrees/`、`.aria/` 与仓库 `.gitignore` 重复（HEAD `.gitignore` 第 2 / 3 / 21 行），但**保留**这 3 行：目标仓未必有这些 `.gitignore` 行；平台自产物的 repo-local 兜底写一次零成本；`.gitignore` 被改动/删除时仍有兜底。

**屏蔽 `.worktrees/` 不影响清理**：清理链路全部是 git 命令/存储驱动（`git worktree remove --force` / `prune` / `list_worktree_paths` / `exists()`），从不读 `git status`；attempt 删除收口判据是 `list_attempts_for_issue`。
