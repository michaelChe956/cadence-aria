# 设计：聚合 LC 登记门禁与生命周期修正

## 决策 1：登记预检不再检查聚合根的用户规则文件

**背景**：`AggregateRootPreflight::validate` 原先调用 `reject_owned_root_files`，对 `CLAUDE.md` / `AGENTS.md` / `.aria` 三者任一存在即拒。豁免通道只有「回执证明产品自己发布过、且字节未变」，而回执只覆盖 `AGENTS.md`（`ROOT_RULE_ENTRY_FILE`）—— `CLAUDE.md` 永不可豁免。

**决定**：删除该检查与整条豁免通道（`reject_owned_root_files`、`validate_recipe_replay`、`proven_recipe_entry_digests`、`proven_digests_from_receipt`、`ReplayEvidenceBreak`），登记只保留：project id 合法、根存在且是目录、**根非 git 仓**、成员在根内且非嵌套 worktree、根不被其它 LC 占用。

**理由**：
- 这些文件是 Cadence-skills 初始化的**正常产物**，不是"用户脏文件"
- **登记本身不写任何文件**，不需要"根必须干净"这个前提；该前提只属于「执行 skills 命令」那一步
- 保留的检查足以防止误登记（git 根、成员越界、symlink 逃逸、重叠）

**代价**：`.aria` 也不再是门禁 → 理论上可把聚合根指向另一个 aria 状态根。判定为该场景代价可接受（成员必须在根内且为 git 仓，实际很难误指），如需恢复该保护应作为独立检查而非"所有权冲突"。

## 决策 2：同一目录全局唯一，按权威存储键判身份

**背景**：重叠检查原先 `if manifest.project_id == project_id { continue }`，同项目可重复占用同根；且 `list_manifests()` 只读 legacy `logical-codebase/manifest.json`，**看不见 v1.3 `logical-codebases/<lc>/` 子树**。

**决定**：
- 去掉同项目跳过；改为「跳过自己」
- **身份比对不用 `manifest.logical_codebase_id`（Uuid），用权威存储键**：legacy 根 ⇒ `legacy_logical_codebase_id(project)`；per-LC 子树 ⇒ 目录名。理由：manifest 内 Uuid 与 API 层 `logical_codebase_<hex>` 不同域、永不相等
- 枚举扩到**所有项目的全部 LC 子树**（legacy 同名迁移副本跳过，避免重复计数）；读取/解析失败 fail-closed
- 软删（墓碑）LC 的 manifest **仍占用该根**（否则软删期间可再登记，恢复后出现双活）

**注入点**：`AggregateRootPreflight` 增加 `lc_id: Option<String>` 与 `for_lc(...)`；登记入口与聚合初始化复验入口都传具体 LC id。

## 决策 3：跳过 skills 用「新增 `Skipped` 状态 + 回执双语义」

**两条路线对比**：

| | (a) 加 `Skipped` + 回执双语义 | (b) 动态步骤表 |
|---|---|---|
旧记录兼容 | ✅ `skipped` 是新增 serde 变体，旧 JSON 可读 | ❌ 旧记录步骤缺失，与"损坏"不可区分 |
校验影响 | 只在枚举上做加法，状态形状校验按「前缀可为 Completed\|Skipped」放宽 | 需放宽 store 形状校验、续跑、DTO 投影、命令索引四处 |
可审计性 | ✅ 跳过必须带判定依据 | ❌ 压成"步骤不存在" |

**选 (a)**，并加三条互证不变量：

1. 任一 `Skipped` 步骤 ⟺ operation 有 `skills_initialization` 判定记录（含两文件 sha256）
2. 判定与三个步骤在**同一次 durable 写**里落盘（无中间态）
3. 跳过式回执签发时**重读盘复验**两文件摘要；若该 operation 已有任何命令审计事实则**拒绝签发**（反之完整 `finalize` 遇跳过式回执也冲突）

**判定窗口**：仅在「三个 provider-turn 步骤全 Pending 且无命令审计事实」时才允许改判跳过；已有命令事实即关窗。这样旧版本中断后续跑的 operation 不会被"事后改判"成跳过。

**判定口径**：`AGENTS.md` 存在 **且** `.claude/rules/language.md` 存在 → 已初始化。只读这两个文件，**不看 `<聚合根>/.aria/`**（该目录无任何代码创建）。IO 失败 fail-closed。

**跳过时仍需发布**：政策合成/发布链抽为共用函数（正文 → locator → current artifact → 来源与 rule digest 复验一条不少），由 `SkippedAggregatePolicyPublisher` 驱动；无发布通道则 fail-closed。

## 决策 4：LC 生命周期三段式（软删 / 恢复 / 硬删）

**软删不清政策正文**：`restore` 后 gateway 靠 `verify_published_policy_body` 读工作区正文，缺失即 fail-closed。软删只清成员仓 `.aria-pointer.md`（可再生、无恢复不变量）。

**硬删需先软删**：存活 LC 直删会绕过全部可恢复性保障，返回 `logical_codebase_not_tombstoned`（409），强制走「软删 → 决定 → 恢复 / 彻底删除」。

**硬删顺序**：先清工作区产物（依赖子树里的 manifest/members/checkouts）→ 再删子树 → **墓碑最后删**。中途失败墓碑仍在，同一入口重试可继续，不留悬空态。legacy 别名 LC 需连旧根一起清，否则 `migrate_legacy` 会复活。

**列表**：`?include_deleted=true` 追加墓碑项（`deleted` + `deleted_at`），不传参行为完全不变（避免墓碑污染既有选择/派生语义）。

## 决策 5：不做全文备份

用户明确要求。替代：跳过判定记录两文件摘要（本就为回执漂移检测所需）；硬删前无需备份（不可恢复是显式语义，且有二次确认）。

## 迁移与兼容

| 项 | 兼容性 |
|---|---|
operation 记录 | 新增 `full_recipe_requested`（创建期冻结）与 `skills_initialization`（`Option`），旧记录读作 `None` = 完整 recipe 语义不变 |
回执 | 新增 `skills_initialization: Option<...>`（`serde(default)`，`None` = 完整 recipe） |
列表接口 | 不传 `include_deleted` 时与旧行为完全一致 |
错误码 | `aggregate_root_ownership_conflict` 已无生产者，但其映射臂与 `aggregate_root_overlap` 共用，保留 |

## 未决 / 留给后续

- 成员仓 `.aria-pointer.md` 仍由独立端点 `pointer-publications` 发布，**未**并入聚合初始化（无设计依据，需 owner 裁决归属）
