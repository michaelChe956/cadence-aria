//! 知识库用户输入：准备判定 + 平台侧确认记录（输入草稿与两段式初始化）。
//!
//! 准备判定口径（D3）：以插件模板 `cadence-init/skills/knowledge-base-bootstrap/
//! user-input/` 的 9 份输入为基线**逐字比对**——全部缺失或全部与模板逐字相同
//! （未填写的占位）⇒ **未准备**（需要起草）；任一份与模板不同 ⇒ **已准备**
//! （不再起草）。平台与 `/knowledge-base-user-input` skill 自身的「已填写/
//! 未填写」判据同源，不会各判各的。
//!
//! 确认记录（D5）：per-LC 持久化在**平台侧** `.aria`（LC scope 的
//! `knowledge-base/user-input-confirmations.json`），记录
//! `path → { digest, confirmed_at }`；**绝不写进 `user-input/`**（skill 的
//! 生成后自检要求该目录只有那 9 份输入文件）。失效语义：查询时逐份比对记录
//! 摘要与文件当前摘要，不一致即视为**未确认**（编辑后自动失效，无需清理
//! 记录，也无需后台任务）；确认请求携带客户端读到的摘要，服务端重算当前
//! 摘要比对，不一致即拒（过期确认，整批原子拒绝）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cross_cutting::document_ops::compute_sha256;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::ExclusiveFileLock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::lc_scope_root;

/// 模板基线 skill 名（与 bootstrap 输入模板同一来源）。
const BOOTSTRAP_SKILL: &str = "knowledge-base-bootstrap";
/// 知识库内输入目录名（相对知识库根）。
const USER_INPUT_DIR: &str = "user-input";
/// LC scope 下确认记录子目录与文件名（平台侧存储，绝不写进 user-input/）。
const CONFIRMATIONS_DIR: &str = "knowledge-base";
const CONFIRMATIONS_FILE: &str = "user-input-confirmations.json";

/// 单份输入文件的状态投影（GET /user-input 的一行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInputFileStatus {
    /// 相对知识库根的路径（`user-input/base-info.md` 形态，`/` 分隔）。
    pub path: String,
    /// 已填写：文件在场且与插件模板**逐字不同**（缺失或与模板相同 = 未填占位）。
    pub filled: bool,
    /// 已确认：已填写且确认记录摘要与当前内容摘要一致（编辑后自动失效）。
    pub confirmed: bool,
    /// 待确认：已填写但未确认。
    pub pending: bool,
}

/// 9 份输入的汇总投影（`confirmed_count` 只计「已填写且摘要一致」的文件）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInputSummary {
    pub files: Vec<UserInputFileStatus>,
    pub confirmed_count: usize,
    pub total_count: usize,
}

/// 一条确认记录：确认时刻的文件内容摘要（sha256 小写十六进制）与时间。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserInputConfirmationRecord {
    pub digest: String,
    pub confirmed_at: String,
}

/// `path → 确认记录`（序列化形态即设计文档的
/// `{ "<path>": { "digest": ..., "confirmed_at": ... } }`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserInputConfirmations(BTreeMap<String, UserInputConfirmationRecord>);

impl UserInputConfirmations {
    /// 取某路径的确认摘要。
    pub fn digest_of(&self, path: &str) -> Option<&str> {
        self.0.get(path).map(|record| record.digest.as_str())
    }

    /// 覆盖写入一条确认记录。
    pub fn record(
        &mut self,
        path: impl Into<String>,
        digest: impl Into<String>,
        confirmed_at: impl Into<String>,
    ) {
        self.0.insert(
            path.into(),
            UserInputConfirmationRecord {
                digest: digest.into(),
                confirmed_at: confirmed_at.into(),
            },
        );
    }

    /// 是否没有任何确认记录。
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// 确认被拒的语义（纯函数层）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserInputConfirmationError {
    /// 过期确认：请求摘要与文件当前摘要不一致（含文件缺失——不存在的内容
    /// 取不到当前摘要，同样无法确认）。
    Stale { path: String },
}

/// 确认 store 的统一失败面：过期确认原样透出，其余按 durable store 故障
///（`ProductStoreError` 未实现相等比较，故本枚举不派生 PartialEq/Eq）。
#[derive(Debug)]
pub enum UserInputConfirmError {
    Stale { path: String },
    Store(ProductStoreError),
}

/// 模板基线根目录：`<插件源根>/knowledge-base-bootstrap/user-input`。
///
/// 源根既接受 `CadenceSkillsPaths::source_root()` 形态
/// （`~/.agents/Cadence-skills/cadence-init/skills`），也接受仓库根形态
/// （`~/.agents/Cadence-skills`，内部再拼 `cadence-init/skills`）——与
/// `templates.rs` 两类模板的同源两形态口径一致；两种形态都探测不到 ⇒
/// `ProductStoreError::Io`（由上层映射为 skills 不可用）。
pub fn user_input_template_root(skills_source_root: &Path) -> Result<PathBuf, ProductStoreError> {
    let candidates = [
        skills_source_root
            .join(BOOTSTRAP_SKILL)
            .join(USER_INPUT_DIR),
        skills_source_root
            .join("cadence-init")
            .join("skills")
            .join(BOOTSTRAP_SKILL)
            .join(USER_INPUT_DIR),
    ];
    for candidate in &candidates {
        if candidate.is_dir() {
            return Ok(candidate.clone());
        }
    }
    Err(ProductStoreError::Io(format!(
        "知识库输入模板源不可用：{} 下找不到 {BOOTSTRAP_SKILL} 的 {USER_INPUT_DIR} 目录",
        skills_source_root.display()
    )))
}

/// 枚举模板基线的输入文件（模板目录下的常规文件，名字升序），返回相对
/// 知识库根的路径（`user-input/<name>`）。真实插件模板为 9 份平铺文件；
/// 枚举以模板为驱动，`user-input/` 内的外来文件（如 updates/ 变更包）不参与
/// 准备判定与确认面。
pub fn list_user_input_templates(template_root: &Path) -> Result<Vec<String>, ProductStoreError> {
    let entries = std::fs::read_dir(template_root).map_err(|error| {
        ProductStoreError::Io(format!(
            "读取知识库输入模板目录 {} 失败：{error}",
            template_root.display()
        ))
    })?;
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    Ok(names
        .into_iter()
        .map(|name| format!("{USER_INPUT_DIR}/{name}"))
        .collect())
}

/// 单份输入的当前事实：`(是否与模板逐字不同, 当前内容摘要)`。
///
/// 任一侧读取失败（缺失、权限、IO 故障）都证明不了「已填写」，fail-safe
/// 返回 `(false, None)`；摘要只在文件可读时计算。
fn file_state(kb_root: &Path, rel_path: &str, template_root: &Path) -> (bool, Option<String>) {
    let Some(name) = rel_path
        .strip_prefix(USER_INPUT_DIR)
        .and_then(|rest| rest.strip_prefix('/'))
    else {
        return (false, None);
    };
    let current = std::fs::read(kb_root.join(USER_INPUT_DIR).join(name)).ok();
    let template = std::fs::read(template_root.join(name)).ok();
    match (current, template) {
        (Some(current), Some(template)) => {
            let digest = compute_sha256(&current);
            (current != template, Some(digest))
        }
        _ => (false, None),
    }
}

/// 「输入已准备」判定（D3）：任一份与模板逐字不同 ⇒ 已准备；全部缺失或
/// 全部与模板相同（未填写的占位）⇒ 未准备。模板枚举或文件读取失败按
/// 「无法证明已填写」处理（fail-safe 向未准备侧收敛；起草 skill 会再次
/// 面对真实文件系统，不会静默丢失用户内容）。
pub fn inputs_prepared(kb_root: &Path, template_root: &Path) -> bool {
    let Ok(templates) = list_user_input_templates(template_root) else {
        return false;
    };
    templates
        .iter()
        .any(|rel_path| file_state(kb_root, rel_path, template_root).0)
}

/// 9 份输入的状态汇总：`filled`（与模板逐字不同）、`confirmed`（已填写且
/// 记录摘要 == 当前摘要——未填写的占位即使有历史记录也不算已确认）、
/// `pending = filled && !confirmed`（待确认）。
pub fn summarize_user_input(
    kb_root: &Path,
    template_root: &Path,
    confirmations: &UserInputConfirmations,
) -> UserInputSummary {
    // 模板枚举失败 ⇒ 空汇总（fail-safe：全未填，fork 走起草分支）。
    let templates = list_user_input_templates(template_root).unwrap_or_default();
    let mut files = Vec::with_capacity(templates.len());
    let mut confirmed_count = 0;
    for rel_path in &templates {
        let (filled, digest) = file_state(kb_root, rel_path, template_root);
        let confirmed = filled
            && digest
                .as_deref()
                .is_some_and(|current| confirmations.digest_of(rel_path) == Some(current));
        if confirmed {
            confirmed_count += 1;
        }
        files.push(UserInputFileStatus {
            path: rel_path.clone(),
            filled,
            confirmed,
            pending: filled && !confirmed,
        });
    }
    UserInputSummary {
        files,
        confirmed_count,
        total_count: templates.len(),
    }
}

/// 确认目标的路径形状：必须是 `user-input/<单段文件名>`（与模板基线的平铺
/// 布局一致；拒绝绝对路径、`..`、反斜杠、空白与控制字符）。
pub fn validate_user_input_rel_path(path: &str) -> Result<(), String> {
    let Some(name) = path.strip_prefix("user-input/") else {
        return Err(format!("确认路径必须位于 user-input/ 下：{path:?}"));
    };
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name == "."
        || name == ".."
        || name.chars().any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err(format!(
            "确认路径必须是 user-input/ 下的单个文件名：{path:?}"
        ));
    }
    Ok(())
}

/// 逐份确认：服务端**重算**当前内容摘要与请求携带的摘要比对，一致 ⇒ 记录
/// 确认（digest 取服务端当前摘要，confirmed_at 取当前时刻）；不一致或文件
/// 缺失 ⇒ [`UserInputConfirmationError::Stale`]（过期确认，不写入任何记录）。
///
/// 路径形状由调用方（HTTP 层）先经 [`validate_user_input_rel_path`] 校验；
/// 本函数信任调用方传入的 `user-input/<文件名>` 相对路径。
pub fn confirm_user_input(
    kb_root: &Path,
    confirmations: &mut UserInputConfirmations,
    path: &str,
    expected_digest: &str,
) -> Result<(), UserInputConfirmationError> {
    let current =
        std::fs::read(kb_root.join(path)).map_err(|_| UserInputConfirmationError::Stale {
            path: path.to_string(),
        })?;
    let digest = compute_sha256(&current);
    if digest != expected_digest {
        return Err(UserInputConfirmationError::Stale {
            path: path.to_string(),
        });
    }
    confirmations.record(path, digest, chrono::Utc::now().to_rfc3339());
    Ok(())
}

/// 确认记录的 per-LC durable store（平台侧 `.aria`，与聚合根内的
/// `user-input/` 目录完全解耦）。绑定 LC 后所有读写只作用于该 LC 自己的
/// `knowledge-base/user-input-confirmations.json`，天然与其它 LC 隔离。
#[derive(Debug, Clone)]
pub struct UserInputConfirmationStore {
    paths: ProductAppPaths,
    lc_id: Option<String>,
}

impl UserInputConfirmationStore {
    /// 把 store 绑定到一个 LC；之后的全部读写都限定在该 LC 的 scope 内。
    pub fn for_lc(paths: ProductAppPaths, lc_id: String) -> Self {
        Self {
            paths,
            lc_id: Some(lc_id),
        }
    }

    /// 确认记录文件路径（LC scope 下的 `knowledge-base/` 子目录）。
    fn confirmations_path(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        Ok(lc_scope_root(&self.paths, project_id, &self.lc_id)?
            .join(CONFIRMATIONS_DIR)
            .join(CONFIRMATIONS_FILE))
    }

    /// 读取确认记录（文件不存在 ⇒ 空记录；首次使用前不强制存在）。
    pub fn load(&self, project_id: &str) -> Result<UserInputConfirmations, ProductStoreError> {
        let path = self.confirmations_path(project_id)?;
        if !path.exists() {
            return Ok(UserInputConfirmations::default());
        }
        read_json(&path)
    }

    /// 批量确认（在确认文件的跨进程排他锁内 load → 逐项校验并记录 → 落盘）。
    ///
    /// 原子性：任一项过期 ⇒ **整批拒绝**，不落盘，既有记录保持原样（不会
    /// 出现「批量确认一半成功」的中间态）。返回确认后的完整记录快照。
    pub fn confirm(
        &self,
        project_id: &str,
        kb_root: &Path,
        items: &[(String, String)],
    ) -> Result<UserInputConfirmations, UserInputConfirmError> {
        let path = self
            .confirmations_path(project_id)
            .map_err(UserInputConfirmError::Store)?;
        let lock_path = path.with_extension("lock");
        let lock =
            ExclusiveFileLock::acquire_at(&lock_path).map_err(UserInputConfirmError::Store)?;
        let mut confirmations = self
            .load(project_id)
            .map_err(UserInputConfirmError::Store)?;
        for (item_path, digest) in items {
            confirm_user_input(kb_root, &mut confirmations, item_path, digest).map_err(
                |error| match error {
                    UserInputConfirmationError::Stale { path } => {
                        UserInputConfirmError::Stale { path }
                    }
                },
            )?;
        }
        write_json(&path, &confirmations).map_err(UserInputConfirmError::Store)?;
        drop(lock);
        Ok(confirmations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 模板基线 + 知识库根的 /tmp 夹具（3 份模板文件，与真实 9 份同构）。
    struct Fixture {
        _dir: tempfile::TempDir,
        template_root: PathBuf,
        kb_root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("fixture dir");
            let template_root = dir.path().join("template").join(USER_INPUT_DIR);
            std::fs::create_dir_all(&template_root).expect("template dir");
            for name in ["a.md", "b.md", "c.sql"] {
                std::fs::write(template_root.join(name), format!("模板 {name}\n"))
                    .expect("template file");
            }
            let kb_root = dir.path().join("kb");
            std::fs::create_dir_all(kb_root.join(USER_INPUT_DIR)).expect("kb user-input dir");
            Self {
                _dir: dir,
                template_root,
                kb_root,
            }
        }

        fn kb_file(&self, name: &str) -> PathBuf {
            self.kb_root.join(USER_INPUT_DIR).join(name)
        }

        fn rel(name: &str) -> String {
            format!("{USER_INPUT_DIR}/{name}")
        }

        fn digest_of(_name: &str, content: &str) -> String {
            compute_sha256(content.as_bytes())
        }
    }

    /// D3：9 份（本夹具 3 份）全缺失 ⇒ 未准备。
    #[test]
    fn inputs_prepared_false_when_all_missing() {
        let fixture = Fixture::new();
        assert!(!inputs_prepared(&fixture.kb_root, &fixture.template_root));
    }

    /// D3：全部与模板逐字相同（未填写的占位）⇒ 未准备。
    #[test]
    fn inputs_prepared_false_when_all_verbatim_templates() {
        let fixture = Fixture::new();
        for name in ["a.md", "b.md", "c.sql"] {
            std::fs::write(
                fixture.kb_file(name),
                std::fs::read_to_string(fixture.template_root.join(name)).unwrap(),
            )
            .expect("seed verbatim copy");
        }
        assert!(!inputs_prepared(&fixture.kb_root, &fixture.template_root));
    }

    /// D3：任一份与模板逐字不同 ⇒ 已准备（不再起草）。
    #[test]
    fn inputs_prepared_true_when_any_differs() {
        let fixture = Fixture::new();
        std::fs::write(fixture.kb_file("b.md"), "# 用户填写\n").expect("seed filled");
        assert!(inputs_prepared(&fixture.kb_root, &fixture.template_root));
    }

    /// 汇总三态：未填 / 已填待确认 / 已填已确认；confirmed_count 只计已确认。
    #[test]
    fn summarize_reports_filled_pending_confirmed_states() {
        let fixture = Fixture::new();
        std::fs::write(fixture.kb_file("a.md"), "# 用户填写 a\n").expect("seed a");
        std::fs::write(fixture.kb_file("b.md"), "# 用户填写 b\n").expect("seed b");
        let mut confirmations = UserInputConfirmations::default();
        let digest_b = Fixture::digest_of("b.md", "# 用户填写 b\n");
        confirmations.record(Fixture::rel("b.md"), digest_b, "2026-11-10T00:00:00Z");

        let summary =
            summarize_user_input(&fixture.kb_root, &fixture.template_root, &confirmations);
        assert_eq!(summary.total_count, 3);
        assert_eq!(summary.confirmed_count, 1);
        let by_path = |path: &str| {
            summary
                .files
                .iter()
                .find(|file| file.path == path)
                .unwrap_or_else(|| panic!("missing {path}"))
        };
        // a：已填、未确认 ⇒ 待确认。
        let a = by_path("user-input/a.md");
        assert_eq!((a.filled, a.confirmed, a.pending), (true, false, true));
        // b：已填、摘要一致 ⇒ 已确认（不再待确认）。
        let b = by_path("user-input/b.md");
        assert_eq!((b.filled, b.confirmed, b.pending), (true, true, false));
        // c：缺失 ⇒ 未填（即使无记录也谈不上确认）。
        let c = by_path("user-input/c.sql");
        assert_eq!((c.filled, c.confirmed, c.pending), (false, false, false));
    }

    /// 与模板逐字相同（未填占位）的文件即使有历史确认记录也不算已确认。
    #[test]
    fn summarize_never_confirms_unfilled_placeholder() {
        let fixture = Fixture::new();
        let template = std::fs::read(fixture.template_root.join("a.md")).unwrap();
        std::fs::write(fixture.kb_file("a.md"), &template).expect("seed placeholder");
        let mut confirmations = UserInputConfirmations::default();
        confirmations.record(
            Fixture::rel("a.md"),
            compute_sha256(&template),
            "2026-11-10T00:00:00Z",
        );
        let summary =
            summarize_user_input(&fixture.kb_root, &fixture.template_root, &confirmations);
        let a = &summary.files[0];
        assert_eq!(a.path, "user-input/a.md");
        assert_eq!((a.filled, a.confirmed, a.pending), (false, false, false));
        assert_eq!(summary.confirmed_count, 0);
    }

    /// D5：摘要一致 ⇒ 确认成功，记录服务端当前摘要。
    #[test]
    fn confirm_records_matching_digest() {
        let fixture = Fixture::new();
        std::fs::write(fixture.kb_file("a.md"), "# 用户填写\n").expect("seed a");
        let digest = Fixture::digest_of("a.md", "# 用户填写\n");
        let mut confirmations = UserInputConfirmations::default();
        confirm_user_input(
            &fixture.kb_root,
            &mut confirmations,
            "user-input/a.md",
            &digest,
        )
        .expect("confirm");
        assert_eq!(
            confirmations.digest_of("user-input/a.md"),
            Some(digest.as_str())
        );
    }

    /// D5：摘要不一致 ⇒ 过期确认，不写入任何记录。
    #[test]
    fn confirm_rejects_stale_digest() {
        let fixture = Fixture::new();
        std::fs::write(fixture.kb_file("a.md"), "# 用户填写\n").expect("seed a");
        let mut confirmations = UserInputConfirmations::default();
        let error = confirm_user_input(
            &fixture.kb_root,
            &mut confirmations,
            "user-input/a.md",
            "deadbeef",
        )
        .unwrap_err();
        assert_eq!(
            error,
            UserInputConfirmationError::Stale {
                path: "user-input/a.md".to_string()
            }
        );
        assert!(confirmations.is_empty());
    }

    /// 文件缺失 ⇒ 无法取当前摘要，同样按过期确认拒绝。
    #[test]
    fn confirm_rejects_missing_file() {
        let fixture = Fixture::new();
        let mut confirmations = UserInputConfirmations::default();
        let error = confirm_user_input(
            &fixture.kb_root,
            &mut confirmations,
            "user-input/a.md",
            "deadbeef",
        )
        .unwrap_err();
        assert!(matches!(error, UserInputConfirmationError::Stale { .. }));
        assert!(confirmations.is_empty());
    }

    /// D5 失效语义：确认后编辑文件 ⇒ 摘要漂移 ⇒ 查询时自动视为未确认。
    #[test]
    fn confirmation_invalidates_after_edit() {
        let fixture = Fixture::new();
        std::fs::write(fixture.kb_file("a.md"), "# 第一版\n").expect("seed a");
        let digest = Fixture::digest_of("a.md", "# 第一版\n");
        let mut confirmations = UserInputConfirmations::default();
        confirm_user_input(
            &fixture.kb_root,
            &mut confirmations,
            "user-input/a.md",
            &digest,
        )
        .expect("confirm");
        let summary =
            summarize_user_input(&fixture.kb_root, &fixture.template_root, &confirmations);
        assert_eq!(summary.confirmed_count, 1);

        std::fs::write(fixture.kb_file("a.md"), "# 第二版\n").expect("edit a");
        let summary =
            summarize_user_input(&fixture.kb_root, &fixture.template_root, &confirmations);
        let a = &summary.files[0];
        assert_eq!((a.filled, a.confirmed, a.pending), (true, false, true));
        assert_eq!(summary.confirmed_count, 0);
    }

    /// 路径形状：只接受 `user-input/<单段文件名>`。
    #[test]
    fn validate_rel_path_accepts_only_flat_user_input_files() {
        assert!(validate_user_input_rel_path("user-input/base-info.md").is_ok());
        for invalid in [
            "base-info.md",
            "user-input/",
            "user-input/sub/dir.md",
            "user-input/../manifest.yaml",
            "user-input/a b.md",
            "/user-input/base-info.md",
        ] {
            assert!(
                validate_user_input_rel_path(invalid).is_err(),
                "must reject {invalid:?}"
            );
        }
    }

    /// 模板根两形态：source_root 形态与仓库根形态都能定位。
    #[test]
    fn template_root_resolves_both_source_forms() {
        let dir = tempfile::tempdir().expect("dir");
        // 仓库根形态：~repo/cadence-init/skills/knowledge-base-bootstrap/user-input。
        let repo_root = dir.path().join("Cadence-skills");
        let nested = repo_root
            .join("cadence-init")
            .join("skills")
            .join(BOOTSTRAP_SKILL)
            .join(USER_INPUT_DIR);
        std::fs::create_dir_all(&nested).expect("nested template dir");
        assert_eq!(user_input_template_root(&repo_root).unwrap(), nested);
        // source_root 形态：~repo/cadence-init/skills 直接作为源根。
        let source_root = repo_root.join("cadence-init").join("skills");
        assert_eq!(user_input_template_root(&source_root).unwrap(), nested);
        // 两种形态都探测不到 ⇒ Io 错误（上层映射 skills 不可用）。
        assert!(user_input_template_root(&dir.path().join("nowhere")).is_err());
    }

    /// store：批量确认落盘 + 重读一致；任一项过期 ⇒ 整批拒绝且不落盘。
    #[test]
    fn store_confirm_batch_atomic_roundtrip() {
        let dir = tempfile::tempdir().expect("dir");
        let fixture_root = tempfile::tempdir().expect("kb dir");
        let kb_root = fixture_root.path().join("kb");
        std::fs::create_dir_all(kb_root.join(USER_INPUT_DIR)).expect("kb user-input");
        std::fs::write(kb_root.join(USER_INPUT_DIR).join("a.md"), "# A\n").expect("a");
        std::fs::write(kb_root.join(USER_INPUT_DIR).join("b.md"), "# B\n").expect("b");

        let paths = ProductAppPaths::new(dir.path().join(".aria"));
        let store = UserInputConfirmationStore::for_lc(paths, "lc_0001".to_string());
        let digest_a = compute_sha256(b"# A\n");
        let digest_b = compute_sha256(b"# B\n");

        // 过期批量（第二项摘要不符）：整批拒绝，不落任何记录文件。
        let error = store
            .confirm(
                "project_0001",
                &kb_root,
                &[
                    ("user-input/a.md".to_string(), digest_a.clone()),
                    ("user-input/b.md".to_string(), "stale".to_string()),
                ],
            )
            .unwrap_err();
        assert!(matches!(
            error,
            UserInputConfirmError::Stale { ref path } if path == "user-input/b.md"
        ));
        assert!(store.load("project_0001").unwrap().is_empty());

        // 合法批量：两项全部确认，落盘后重读一致。
        let confirmed = store
            .confirm(
                "project_0001",
                &kb_root,
                &[
                    ("user-input/a.md".to_string(), digest_a.clone()),
                    ("user-input/b.md".to_string(), digest_b.clone()),
                ],
            )
            .expect("confirm batch");
        assert_eq!(
            confirmed.digest_of("user-input/a.md"),
            Some(digest_a.as_str())
        );
        let reloaded = store.load("project_0001").expect("reload");
        assert_eq!(
            reloaded.digest_of("user-input/b.md"),
            Some(digest_b.as_str())
        );
    }
}
