//! Task 1.5（REQ-BOOT-03、D2、REQ-REG-09）：root recipe receipt 与
//! filesystem auditor——recipe 副作用的可审计生产安全边界。
//!
//! 每条 root recipe 命令执行前后，[`RootRecipeFilesystemAuditor`] 对
//! canonical 聚合根做全量快照并 diff：变更只允许落在 allowlist
//! （`.aria/aggregate/**`，canonical 相对路径）内；未知路径、成员
//! `.git`/worktree 变化、symlink 逃逸、绝对/父路径越界与不可观测写入
//! 一律 fail-closed，拒绝事实与证据随 receipt durable 保留——允许记录，
//! 不允许静默。auditor 对聚合根只读，绝不覆盖或回滚用户文件。
//!
//! 快照 gitignore 感知（Bug2 2026-10-16，
//! cadence/notes/2026-10-16_bug-filesystem-audit-不感知gitignore.md）：
//! 被仓库 `.gitignore`（含嵌套成员仓）与 `.git/info/exclude` 忽略的
//! 路径不计入快照规模预算、不进快照（node_modules/target 等非版本
//! 构建面不属于审计观测面）；观测面内的预算/可观测性约束不变。
//!
//! [`RootRecipeReceiptStore`] 把每条命令的审计事实
//! （[`RootRecipeCommandReceipt`]）与最终 receipt（[`RootRecipeReceipt`]，
//! 仅在四命令全部审计通过且关联 policy/rule digest 后由 `finalize`
//! 落盘）持久化到 per-LC scope 的 `aggregate-recipe-receipts/`，原子写
//! 含 rename 后父目录 fsync，用户冲突不覆盖。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::product::app_paths::ProductAppPaths;
use crate::product::audit_snapshot_budget::{
    AuditSnapshotBudgetFailure, BudgetGroup, MemberBudgetSummary,
};
use crate::product::coding_attempt_store::locking::with_exact_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id};

use super::aggregate_initialization::{
    AGGREGATE_SKILLS_MARKER_ENTRY, AGGREGATE_SKILLS_MARKER_LANGUAGE_RULE,
    AggregateInitializationStepKind, AggregateSkillsInitialization, AggregateSkillsMarker,
    detect_skills_initialization,
};
use super::aggregate_initialization_store::root_recipe_command_index;

// 本文件按仓库惯例拆入 `.inc.rs`（同模块命名空间，符号路径与可见性不变），
// 保持每个文件低于 large_file_guard 的 1200 行上限：
// - root_recipe_receipt_types.inc.rs：allowlist 常量与快照/变更/verdict/receipt
//   契约类型。
// - root_recipe_receipt_gitignore.inc.rs：快照 gitignore 感知过滤——
//   `.gitignore`（含嵌套）与 `.git/info/exclude` 的解析/匹配（Bug2
//   2026-10-16：被忽略路径不计预算、不进快照）。
// - root_recipe_snapshot.inc.rs：快照扫描与预算计费——元数据差分（两遍
//   全仓扫描只记元数据、只对变更文件读内容）、`.git` 子树整棵跳过、流式
//   读取（边读边计费、越限立刻停）、大项分组与成员仓贡献统计
//   （REQ-AUD-01/02/03/04）。
// - root_recipe_receipt_auditor.inc.rs：RootRecipeFilesystemAuditor——命令前后
//   全量扫描与 diff 的只读审计器。
// - root_recipe_receipt_store.inc.rs：RootRecipeReceiptStore 与命令/receipt 校验、
//   allowlist/classify/digest 及 durable 原子写入辅助函数。
// - root_recipe_receipt_tests.inc.rs：tests 模块。
// - root_recipe_zero_change_tests.inc.rs：REQ-BOOT-17 零动作 turn 判失败的
//   单测（零动作 ⇒ step 失败且失败事实含 provider / step / 零变更）。
include!("root_recipe_receipt_types.inc.rs");
include!("root_recipe_receipt_gitignore.inc.rs");
include!("root_recipe_snapshot.inc.rs");
include!("root_recipe_receipt_auditor.inc.rs");
include!("root_recipe_receipt_store.inc.rs");
include!("root_recipe_receipt_tests.inc.rs");
include!("root_recipe_zero_change_tests.inc.rs");
