//! REQ-REG-14 用户 home/config adapter：Codex projects trust TOML 增改、
//! Kimi workspace-trust 文件写入/撤销、锁/CAS/格式校验。
//!
//! 真实用户 home 路径只作为 adapter 的默认参数进入（`for_home` /
//! `production`）；一切测试都以 tempdir home 构造 adapter，绝不触碰真实
//! home。TOML 增改采用逐块追加语义：只追加缺失块、只插入缺失键、不覆盖
//! 已有用户值；解析/读取失败 fail-closed（建议用户先备份）。跨 LC 并发
//! 可重试等待，绝不盲目覆盖。

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::product::logical_codebase::provider_trust::ProviderTrustError;
use crate::product::models::ProviderName;

/// One provider's user-level trust entry state for a canonical root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderTrustEntryState {
    /// Entry present and trusted for this root.
    Trusted,
    /// Key present but not trusted; `current_value` carries the observed
    /// conflicting value when one is readable.
    Untrusted { current_value: Option<String> },
    /// No entry for this root.
    Missing,
}

/// Outcome of a trusted-entry write against the user-level artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderTrustWriteOutcome {
    /// Entry created; digests of the backing artifact before/after the write.
    Created {
        before_digest: Option<String>,
        after_digest: String,
    },
    /// Entry already trusted; nothing was written (idempotent replay).
    AlreadyTrusted,
}

/// adapter 窄只读工件描述（D4）：进入人工指导的 provider 特定部分——
/// 实际工件路径、该根的稳定 trust key 与人工登记/修复的最小形状。
/// 只描述位置与形状，不读取/返回完整配置内容或凭据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderTrustArtifactHint {
    /// 用户级 trust 工件的实际文件路径（adapter 装配的 home 之下）。
    pub artifact_path: PathBuf,
    /// 该 canonical root 的稳定 trust 键（Codex TOML 表头/Kimi 记录名）。
    pub trust_key: String,
    /// 人工登记/修复该键的最小形状描述（provider 特定）。
    pub manual_shape: String,
    /// 可直接复制执行/粘贴的人工登记命令（owner 2026-10-12 直批：指引卡
    /// 登记按钮配套）。Kimi 给单行 shell（mkdir+printf 记录）；Codex 的
    /// TOML 无法一行安全写，给「config.toml 路径 + 完整表片段」多行文本。
    /// 路径/键/root 全部取 adapter 真实值（同源），凭据永不进入命令。
    pub manual_command: Option<String>,
}

/// 用户级 trust 工件 adapter。所有 home 副作用只发生在这里；registry 与
/// store 永远不直接触碰用户 home 文件。
pub trait ProviderTrustHomeAdapter: Send + Sync {
    fn provider(&self) -> ProviderName;
    /// Stable trust key for `canonical_root`（Codex：
    /// `[projects."<canonical_root>"]`；Kimi：
    /// `wd_<basename>_<sha256(canonical_root)[:12]>`）。
    fn trust_key(&self, canonical_root: &Path) -> String;
    /// Entry state without writing anything.
    fn read_state(
        &self,
        canonical_root: &Path,
    ) -> Result<ProviderTrustEntryState, ProviderTrustError>;
    /// Digest of the backing artifact this adapter would modify for the
    /// root (`None` when the artifact does not exist yet).
    fn digest(&self, canonical_root: &Path) -> Result<Option<String>, ProviderTrustError>;
    /// Write the trusted entry. `expected_digest` is the digest the caller
    /// observed earlier; a mismatch refuses the write (CAS fail-closed).
    fn write_trusted(
        &self,
        canonical_root: &Path,
        expected_digest: Option<&str>,
    ) -> Result<ProviderTrustWriteOutcome, ProviderTrustError>;
    /// Remove the trusted entry under the same CAS contract.
    fn remove_trusted(
        &self,
        canonical_root: &Path,
        expected_digest: Option<&str>,
    ) -> Result<(), ProviderTrustError>;
    /// Stable error code used when the key holds an untrusted value.
    fn conflict_code(&self) -> &'static str;
    /// 窄只读人工指导（D4）：本 adapter 用户级 trust 工件的实际路径、该根
    /// 的稳定 trust 键与人工修复的最小形状；key/转义与 adapter 自身算法
    /// 同源，不手抄第二份。
    fn manual_hint(&self, canonical_root: &Path) -> ProviderTrustArtifactHint;
    /// 本根登记/撤销会修改的稳定用户级工件路径（3.1 sidecar 锁目标）：
    /// Codex=整份 config.toml（所有 root 共锁）；Kimi=单条记录文件。
    /// 只描述位置；锁的获取/释放由 registry 经 `ExclusiveFileLock` 完成，
    /// adapter 自身不持锁。
    fn artifact_path(&self, canonical_root: &Path) -> PathBuf;
}

/// Codex 用户级 projects trust：`~/.codex/config.toml` 中
/// `[projects."<canonical_root>"]` 的 `trust_level = "trusted"`。
///
/// 命令行 `-c` 覆盖不参与 codex 的 trust 判定，因此登记必须落在用户级
/// 配置文件里（预研报告 2026-10-01 实证）。
pub struct CodexTrustAdapter {
    config_path: PathBuf,
}

impl CodexTrustAdapter {
    pub fn for_home(home: &Path) -> Self {
        Self {
            config_path: home.join(".codex").join("config.toml"),
        }
    }

    /// Resolves the adapter against the real user home (`HOME`/`USERPROFILE`).
    pub fn production() -> Option<Self> {
        home_from_env().map(|home| Self::for_home(&home))
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    fn read_content(&self) -> Result<Option<String>, ProviderTrustError> {
        match std::fs::read_to_string(&self.config_path) {
            Ok(content) => Ok(Some(content)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ProviderTrustError::transient(
                "codex_config_unreadable",
                format!(
                    "read {} failed: {error}; 备份并修复该文件的读取权限后重试",
                    self.config_path.display()
                ),
            )),
        }
    }
}

impl ProviderTrustHomeAdapter for CodexTrustAdapter {
    fn provider(&self) -> ProviderName {
        ProviderName::Codex
    }

    fn trust_key(&self, canonical_root: &Path) -> String {
        table_header_for(canonical_root)
    }

    fn read_state(
        &self,
        canonical_root: &Path,
    ) -> Result<ProviderTrustEntryState, ProviderTrustError> {
        let Some(content) = self.read_content()? else {
            return Ok(ProviderTrustEntryState::Missing);
        };
        let lines: Vec<&str> = content.split('\n').collect();
        match locate_projects_table(&lines, canonical_root) {
            None => Ok(ProviderTrustEntryState::Missing),
            Some(table) => match trust_level_in_table(&lines, table) {
                Some(value) if value == "trusted" => Ok(ProviderTrustEntryState::Trusted),
                Some(other) => Ok(ProviderTrustEntryState::Untrusted {
                    current_value: Some(other),
                }),
                None => Ok(ProviderTrustEntryState::Untrusted {
                    current_value: None,
                }),
            },
        }
    }

    fn digest(&self, _canonical_root: &Path) -> Result<Option<String>, ProviderTrustError> {
        Ok(self.read_content()?.map(|content| digest_text(&content)))
    }

    fn write_trusted(
        &self,
        canonical_root: &Path,
        expected_digest: Option<&str>,
    ) -> Result<ProviderTrustWriteOutcome, ProviderTrustError> {
        let content = self.read_content()?;
        let current_digest = content.as_deref().map(digest_text);
        check_cas(
            current_digest.as_deref(),
            expected_digest,
            &self.config_path,
        )?;
        let header = table_header_for(canonical_root);
        let Some(content) = content else {
            let fresh = format!("{header}\ntrust_level = \"trusted\"\n");
            atomic_write_text(&self.config_path, &fresh).map_err(|error| {
                write_failure("codex_trust_write_failed", &self.config_path, error)
            })?;
            return Ok(ProviderTrustWriteOutcome::Created {
                before_digest: None,
                after_digest: digest_text(&fresh),
            });
        };
        let lines: Vec<&str> = content.split('\n').collect();
        match locate_projects_table(&lines, canonical_root) {
            Some(table) => match trust_level_in_table(&lines, table) {
                Some(value) if value == "trusted" => Ok(ProviderTrustWriteOutcome::AlreadyTrusted),
                Some(other) => Err(ProviderTrustError::conflict(
                    self.conflict_code(),
                    format!(
                        "{header} already holds trust_level = {other:?}; \
                         拒绝覆盖用户既有值，请用户自行确认该键取值后重试"
                    ),
                )),
                None => {
                    let mut updated: Vec<String> =
                        content.split('\n').map(str::to_string).collect();
                    updated.insert(table.0 + 1, "trust_level = \"trusted\"".to_string());
                    let updated = updated.join("\n");
                    let after_digest = digest_text(&updated);
                    atomic_write_text(&self.config_path, &updated).map_err(|error| {
                        write_failure("codex_trust_write_failed", &self.config_path, error)
                    })?;
                    Ok(ProviderTrustWriteOutcome::Created {
                        before_digest: current_digest,
                        after_digest,
                    })
                }
            },
            None => {
                let mut updated = content;
                if !updated.is_empty() && !updated.ends_with('\n') {
                    updated.push('\n');
                }
                updated.push_str(&header);
                updated.push('\n');
                updated.push_str("trust_level = \"trusted\"\n");
                let after_digest = digest_text(&updated);
                atomic_write_text(&self.config_path, &updated).map_err(|error| {
                    write_failure("codex_trust_write_failed", &self.config_path, error)
                })?;
                Ok(ProviderTrustWriteOutcome::Created {
                    before_digest: current_digest,
                    after_digest,
                })
            }
        }
    }

    fn remove_trusted(
        &self,
        canonical_root: &Path,
        expected_digest: Option<&str>,
    ) -> Result<(), ProviderTrustError> {
        let content = self.read_content()?;
        let current_digest = content.as_deref().map(digest_text);
        check_cas(
            current_digest.as_deref(),
            expected_digest,
            &self.config_path,
        )?;
        let Some(content) = content else {
            return Ok(());
        };
        let lines: Vec<&str> = content.split('\n').collect();
        let Some((header_index, body_end)) = locate_projects_table(&lines, canonical_root) else {
            return Ok(());
        };
        let mut updated: Vec<&str> = Vec::with_capacity(lines.len());
        updated.extend_from_slice(&lines[..header_index]);
        updated.extend_from_slice(&lines[body_end..]);
        let updated = updated.join("\n");
        atomic_write_text(&self.config_path, &updated)
            .map_err(|error| write_failure("codex_trust_remove_failed", &self.config_path, error))
    }

    fn conflict_code(&self) -> &'static str {
        "codex_trust_key_conflict"
    }

    fn manual_hint(&self, canonical_root: &Path) -> ProviderTrustArtifactHint {
        let trust_key = self.trust_key(canonical_root);
        let manual_command = format!(
            "在 {} 追加（或并入既有表）：\n{trust_key}\ntrust_level = \"trusted\"",
            self.config_path.display()
        );
        ProviderTrustArtifactHint {
            artifact_path: self.config_path.clone(),
            manual_shape: format!(
                "在 {trust_key} 表内设置 trust_level = \"trusted\"\
                 （保留文件内其余用户配置不动）"
            ),
            trust_key,
            manual_command: Some(manual_command),
        }
    }

    fn artifact_path(&self, _canonical_root: &Path) -> PathBuf {
        // 整份 config 共锁（3.1）：同一 config 下不同 root 的增改互斥。
        self.config_path.clone()
    }
}

/// Kimi workspace-trust：`~/.kimi-code/workspace-trust/<key>` 记录
/// `{"root":"<canonical_root>","trustedAt":<ms>}`，
/// `key = wd_<basename>_<sha256(canonical_root)[:12]>`（预研报告与现网
/// 记录 `wd_naruto_4d73fb6dca57` 交叉核实）。
pub struct KimiTrustAdapter {
    workspace_trust_root: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct KimiTrustRecord {
    root: String,
    #[serde(rename = "trustedAt")]
    trusted_at: u64,
}

impl KimiTrustAdapter {
    pub fn for_home(home: &Path) -> Self {
        Self {
            workspace_trust_root: home.join(".kimi-code").join("workspace-trust"),
        }
    }

    /// Resolves the adapter against the real user home (`HOME`/`USERPROFILE`).
    pub fn production() -> Option<Self> {
        home_from_env().map(|home| Self::for_home(&home))
    }

    pub fn workspace_trust_root(&self) -> &Path {
        &self.workspace_trust_root
    }

    pub(crate) fn record_path(&self, canonical_root: &Path) -> PathBuf {
        self.workspace_trust_root
            .join(self.trust_key(canonical_root))
    }

    fn read_record(
        &self,
        canonical_root: &Path,
    ) -> Result<Option<KimiTrustRecord>, ProviderTrustError> {
        let path = self.record_path(canonical_root);
        match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).map(Some).map_err(|error| {
                ProviderTrustError::transient(
                    "kimi_trust_record_invalid",
                    format!(
                        "parse {} failed: {error}; 备份该记录并修复格式后重试",
                        path.display()
                    ),
                )
            }),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ProviderTrustError::transient(
                "kimi_trust_unreadable",
                format!("read {} failed: {error}", path.display()),
            )),
        }
    }
}

impl ProviderTrustHomeAdapter for KimiTrustAdapter {
    fn provider(&self) -> ProviderName {
        ProviderName::KimiCode
    }

    fn trust_key(&self, canonical_root: &Path) -> String {
        let Some(basename) = canonical_root.file_name() else {
            return String::new();
        };
        let digest_hex = format!(
            "{:x}",
            Sha256::digest(root_string(canonical_root).as_bytes())
        );
        format!("wd_{}_{}", basename.to_string_lossy(), &digest_hex[..12])
    }

    fn read_state(
        &self,
        canonical_root: &Path,
    ) -> Result<ProviderTrustEntryState, ProviderTrustError> {
        match self.read_record(canonical_root)? {
            None => Ok(ProviderTrustEntryState::Missing),
            Some(record) if record.root == root_string(canonical_root) => {
                Ok(ProviderTrustEntryState::Trusted)
            }
            Some(record) => Ok(ProviderTrustEntryState::Untrusted {
                current_value: Some(record.root),
            }),
        }
    }

    fn digest(&self, canonical_root: &Path) -> Result<Option<String>, ProviderTrustError> {
        match std::fs::read(self.record_path(canonical_root)) {
            Ok(bytes) => Ok(Some(digest_bytes(&bytes))),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ProviderTrustError::transient(
                "kimi_trust_unreadable",
                format!(
                    "read {} failed: {error}",
                    self.record_path(canonical_root).display()
                ),
            )),
        }
    }

    fn write_trusted(
        &self,
        canonical_root: &Path,
        expected_digest: Option<&str>,
    ) -> Result<ProviderTrustWriteOutcome, ProviderTrustError> {
        match self.read_record(canonical_root)? {
            Some(record) if record.root == root_string(canonical_root) => {
                Ok(ProviderTrustWriteOutcome::AlreadyTrusted)
            }
            Some(record) => Err(ProviderTrustError::conflict(
                self.conflict_code(),
                format!(
                    "workspace-trust key already bound to root {:?}; 拒绝覆盖用户既有记录",
                    record.root
                ),
            )),
            None => {
                let current_digest = self.digest(canonical_root)?;
                check_cas(
                    current_digest.as_deref(),
                    expected_digest,
                    &self.record_path(canonical_root),
                )?;
                let record = KimiTrustRecord {
                    root: root_string(canonical_root),
                    trusted_at: now_millis(),
                };
                let raw = serde_json::to_string(&record).map_err(|error| {
                    ProviderTrustError::transient(
                        "kimi_trust_write_failed",
                        format!("serialize workspace-trust record failed: {error}"),
                    )
                })?;
                let path = self.record_path(canonical_root);
                atomic_write_text(&path, &raw)
                    .map_err(|error| write_failure("kimi_trust_write_failed", &path, error))?;
                Ok(ProviderTrustWriteOutcome::Created {
                    before_digest: current_digest,
                    after_digest: digest_text(&raw),
                })
            }
        }
    }

    fn remove_trusted(
        &self,
        canonical_root: &Path,
        expected_digest: Option<&str>,
    ) -> Result<(), ProviderTrustError> {
        let path = self.record_path(canonical_root);
        let current_digest = self.digest(canonical_root)?;
        check_cas(current_digest.as_deref(), expected_digest, &path)?;
        if current_digest.is_none() {
            return Ok(());
        }
        std::fs::remove_file(&path)
            .map_err(|error| write_failure("kimi_trust_remove_failed", &path, error))
    }

    fn conflict_code(&self) -> &'static str {
        "kimi_trust_key_conflict"
    }

    fn manual_hint(&self, canonical_root: &Path) -> ProviderTrustArtifactHint {
        let trust_key = self.trust_key(canonical_root);
        let shape = serde_json::json!({
            "root": root_string(canonical_root),
            "trustedAt": "<毫秒时间戳>",
        });
        // 单行可复制 shell：目录/记录路径整参单引号（内嵌单引号走 '\''
        // 转义）；root 的 JSON 转义交给 serde（同源，不手抄算法）。
        // printf 格式串语义（k3 P3 2026-10-12）：root 中字面 `%` 一律转义
        // `%%`——占位符在格式字面量中独立插入，不与转义互扰——复制执行
        // 才不写坏 JSON；`date +%s%3N` 需 GNU date，尾部注明 GNU/Linux。
        let root_json = serde_json::to_string(&root_string(canonical_root))
            .unwrap_or_default()
            .replace('%', "%%");
        let payload = format!(r#"{{"root":{root_json},"trustedAt":%s}}"#);
        let manual_command = format!(
            "mkdir -p {} && printf '{}' \"$(date +%s%3N)\" > {} # GNU/Linux（date +%s%3N 需 GNU date）",
            shell_single_quoted(&self.workspace_trust_root.display().to_string()),
            payload,
            shell_single_quoted(&self.record_path(canonical_root).display().to_string()),
        );
        ProviderTrustArtifactHint {
            artifact_path: self.record_path(canonical_root),
            manual_shape: format!("记录 JSON 形如 {shape}"),
            trust_key,
            manual_command: Some(manual_command),
        }
    }

    fn artifact_path(&self, canonical_root: &Path) -> PathBuf {
        // 单记录锁（3.1）：不同 root 的记录互不串行。
        self.record_path(canonical_root)
    }
}

fn root_string(canonical_root: &Path) -> String {
    canonical_root.display().to_string()
}

/// shell 单引号整参（POSIX '\'' 转义内嵌单引号）——可复制命令的路径
/// 参数包装；不含任何求值，凭据永不进入。
fn shell_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn table_header_for(canonical_root: &Path) -> String {
    format!(
        "[projects.\"{}\"]",
        escape_toml_basic_string(&root_string(canonical_root))
    )
}

fn escape_toml_basic_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Locates `[projects."<root>"]`; returns the header line index and the
/// exclusive end of its body (the next table header or end of file).
fn locate_projects_table(lines: &[&str], canonical_root: &Path) -> Option<(usize, usize)> {
    let header = table_header_for(canonical_root);
    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim() == header {
            let mut end = index + 1;
            while end < lines.len() && !lines[end].trim_start().starts_with('[') {
                end += 1;
            }
            return Some((index, end));
        }
        index += 1;
    }
    None
}

fn trust_level_in_table(lines: &[&str], table: (usize, usize)) -> Option<String> {
    lines[table.0 + 1..table.1].iter().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "trust_level").then(|| value.trim().trim_matches('"').to_string())
    })
}

fn check_cas(
    current: Option<&str>,
    expected: Option<&str>,
    path: &Path,
) -> Result<(), ProviderTrustError> {
    if current == expected {
        return Ok(());
    }
    Err(ProviderTrustError::transient(
        "trust_concurrent_modification",
        format!(
            "{} changed since it was read (digest {:?} != {:?}); fail-closed，可重试",
            path.display(),
            current,
            expected
        ),
    ))
}

fn write_failure(code: &'static str, path: &Path, error: std::io::Error) -> ProviderTrustError {
    ProviderTrustError::transient(code, format!("write {} failed: {error}", path.display()))
}

fn digest_text(content: &str) -> String {
    digest_bytes(content.as_bytes())
}

fn digest_bytes(content: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(content))
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 受信 owner home 的环境解析(HOME/USERPROFILE,仅绝对路径):trust
/// 发现(`production()`)的 owner_home 单一入口,禁止多处各自解析造成
/// HOME/CODEX_HOME 发现漂移。
///
/// A1-a(Bug45 批2 环境回退底座):自 `cross_cutting::provider_environment`
/// 迁入 trust 侧(逐字等价实现)——环境白名单模块待 A1-b 删除,owner
/// home 只读发现职责落位于其存活消费方(trust adapter)。
pub(crate) fn resolve_owner_home_from_env() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
}

fn home_from_env() -> Option<PathBuf> {
    resolve_owner_home_from_env()
}

/// Temp-file + rename atomic text write, mirroring `json_store::write_json`.
fn atomic_write_text(path: &Path, contents: &str) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "provider-trust".to_string());
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temp_path = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        timestamp
    ));
    std::fs::write(&temp_path, contents)?;
    if let Err(error) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{
        CodexTrustAdapter, KimiTrustAdapter, ProviderTrustEntryState, ProviderTrustHomeAdapter,
    };

    #[test]
    fn codex_adapter_preserves_unrelated_content_and_inserts_only_missing_key() {
        let home = TempDir::new().unwrap();
        let adapter = CodexTrustAdapter::for_home(home.path());
        let config = home.path().join(".codex").join("config.toml");
        let root = home.path().join("lc");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        let initial = "# user comment\n\n[model]\nmodel = \"gpt-5\"\n\n[projects.\"/other\"]\ntrust_level = \"trusted\"\n";
        std::fs::write(&config, initial).unwrap();

        // 追加缺失块：其余内容逐字节保留（CAS 传入观察到的 digest）。
        let observed = adapter.digest(&root).unwrap();
        adapter.write_trusted(&root, observed.as_deref()).unwrap();
        let appended = std::fs::read_to_string(&config).unwrap();
        assert!(appended.starts_with(initial));
        assert!(appended.contains(&format!(
            "[projects.\"{}\"]\ntrust_level = \"trusted\"\n",
            root.display()
        )));
        assert_eq!(appended.matches("[model]").count(), 1);
        assert_eq!(appended.matches("[projects.\"/other\"]").count(), 1);

        // 已有本表但缺 trust_level 键：只插入缺失键，用户键保留。
        std::fs::write(
            &config,
            format!("[projects.\"{}\"]\nnotify = true\n", root.display()),
        )
        .unwrap();
        let before_digest = adapter.digest(&root).unwrap();
        adapter
            .write_trusted(&root, before_digest.as_deref())
            .unwrap();
        let inserted = std::fs::read_to_string(&config).unwrap();
        assert!(inserted.contains("notify = true"));
        assert!(inserted.contains("trust_level = \"trusted\""));
        assert_eq!(
            inserted
                .matches(&format!("[projects.\"{}\"]", root.display()))
                .count(),
            1
        );
    }

    #[test]
    fn codex_adapter_rejects_conflicting_values_and_cas_drift() {
        let home = TempDir::new().unwrap();
        let adapter = CodexTrustAdapter::for_home(home.path());
        let config = home.path().join(".codex").join("config.toml");
        let root = home.path().join("lc");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        let conflicting = format!(
            "[projects.\"{}\"]\ntrust_level = \"untrusted\"\n",
            root.display()
        );
        std::fs::write(&config, &conflicting).unwrap();

        let observed = adapter.digest(&root).unwrap();
        let error = adapter
            .write_trusted(&root, observed.as_deref())
            .unwrap_err();
        assert_eq!(error.code, "codex_trust_key_conflict");
        assert!(!error.retryable);
        // 不覆盖：文件保持用户原值。
        assert_eq!(std::fs::read_to_string(&config).unwrap(), conflicting);

        // CAS 漂移：expected digest 与现状不一致时 fail-closed。
        std::fs::write(&config, "model = \"gpt-5\"\n").unwrap();
        let error = adapter
            .write_trusted(&root, Some("sha256:stale"))
            .unwrap_err();
        assert_eq!(error.code, "trust_concurrent_modification");
        assert!(error.retryable);
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "model = \"gpt-5\"\n"
        );
    }

    #[test]
    fn kimi_adapter_rejects_root_mismatch_and_invalid_records() {
        let home = TempDir::new().unwrap();
        let adapter = KimiTrustAdapter::for_home(home.path());
        let root = home.path().join("lc");
        std::fs::create_dir_all(&root).unwrap();
        let record_path = adapter.record_path(&root);
        std::fs::create_dir_all(record_path.parent().unwrap()).unwrap();

        // 同 key 不同 root：归属冲突，不覆盖。
        std::fs::write(
            &record_path,
            "{\"root\":\"/somewhere/else\",\"trustedAt\":1}",
        )
        .unwrap();
        assert_eq!(
            adapter.read_state(&root).unwrap(),
            ProviderTrustEntryState::Untrusted {
                current_value: Some("/somewhere/else".to_string())
            }
        );
        let error = adapter.write_trusted(&root, None).unwrap_err();
        assert_eq!(error.code, "kimi_trust_key_conflict");
        assert_eq!(
            std::fs::read_to_string(&record_path).unwrap(),
            "{\"root\":\"/somewhere/else\",\"trustedAt\":1}"
        );

        // 格式损坏：fail-closed，不猜语义。
        std::fs::write(&record_path, "not-json").unwrap();
        let error = adapter.read_state(&root).unwrap_err();
        assert_eq!(error.code, "kimi_trust_record_invalid");
    }

    #[test]
    fn adapters_expose_provider_identity_and_stable_keys() {
        let home = TempDir::new().unwrap();
        let root = home.path().join("lc");
        let codex = CodexTrustAdapter::for_home(home.path());
        let kimi = KimiTrustAdapter::for_home(home.path());
        assert_eq!(
            codex.trust_key(&root),
            format!("[projects.\"{}\"]", root.display())
        );
        assert!(kimi.trust_key(&root).starts_with("wd_lc_"));
    }

    /// Task 2.2：hint 的路径/key 与 adapter 同源（特殊根的 TOML 转义、
    /// Kimi 实际记录文件与 serde 转义的 JSON 形状），不手抄第二份算法。
    #[test]
    fn adapter_manual_hints_carry_real_paths_keys_and_shapes() {
        let home = TempDir::new().unwrap();
        let special = home.path().join("lc\"root\\path");
        std::fs::create_dir_all(&special).unwrap();
        let codex = CodexTrustAdapter::for_home(home.path());
        let hint = codex.manual_hint(&special);
        assert_eq!(
            hint.artifact_path,
            home.path().join(".codex").join("config.toml")
        );
        assert_eq!(hint.trust_key, codex.trust_key(&special));
        assert!(hint.trust_key.contains("\\\""), "quote must stay escaped");
        assert!(
            hint.trust_key.contains("\\\\"),
            "backslash must stay escaped"
        );
        assert!(hint.manual_shape.contains("trust_level = \"trusted\""));

        let kimi = KimiTrustAdapter::for_home(home.path());
        let root = home.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        let hint = kimi.manual_hint(&root);
        assert_eq!(hint.artifact_path, kimi.record_path(&root));
        assert_eq!(hint.trust_key, kimi.trust_key(&root));
        assert!(
            hint.manual_shape
                .contains(&format!("\"root\":\"{}\"", root.display()))
        );
        assert!(hint.manual_shape.contains("\"trustedAt\""));
    }
}
