//! kimi ACP `mcpServers` 受控注入 bundle（tasks.md 6.1）。
//!
//! `session/new`/`session/load` 的 `mcpServers` 必须来自经校验的 Aria-owned
//! 受控 bundle：server 名 allowlist、配置 digest、凭据引用脱敏（env 只允许
//! `env:<VAR>`/`file:<相对路径>` 引用形态，拒绝字面量凭据）、argv 审计记录。
//! 未配置 bundle 时 kimi 会话保持 `mcpServers: []`（见 session.rs）。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;
use sha2::Digest as _;

/// kimi ACP 通道允许注入的 MCP server 名 allowlist（fail-closed：名单外一律拒绝）。
pub const KIMI_MCP_SERVER_ALLOWLIST: [&str; 1] = ["codegraph"];

/// 受控 MCP server 声明（Aria-owned 配置产物反序列化而来，不是模型/用户输入）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

impl McpServerConfig {
    pub fn new(name: &str, command: &str) -> Self {
        Self {
            name: name.to_string(),
            command: command.to_string(),
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: None,
        }
    }
}

/// MCP bundle 校验失败（allowlist 越界、凭据字面量、非法路径等）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpBundleError {
    pub message: String,
}

impl McpBundleError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for McpBundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// 经校验的受控 MCP bundle：allowlist + 配置 digest。
///
/// 没有 public 构造函数能绕过 `validate`；digest 在校验时冻结。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedMcpServerBundle {
    servers: Vec<McpServerConfig>,
    digest: String,
}

/// kimi 会话的 MCP 注入意图：bundle + （resume 时的）冻结 digest。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KimiMcpInjection {
    bundle: ValidatedMcpServerBundle,
    frozen_digest: Option<String>,
}

/// resume 决策结果：session/load 放行，或 digest 漂移时拒绝并转 session/new
/// （旧会话标记 superseded，对齐 session-policy-envelope REQ-ENV-04）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionMethodDecision {
    Load {
        session_id: String,
        mcp_servers: Vec<Value>,
    },
    New {
        superseded: Option<SupersededSession>,
        mcp_servers: Vec<Value>,
    },
}

/// digest 漂移时被拒绝续接的旧会话审计信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersededSession {
    pub session_id: String,
    pub frozen_digest: String,
    pub actual_digest: String,
}

impl ValidatedMcpServerBundle {
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// 派生 ACP `session/new`/`session/load` 的 `mcpServers` JSON（凭据保持引用形态）。
    pub fn mcp_servers_json(&self) -> Vec<Value> {
        self.servers
            .iter()
            .map(|server| {
                let mut value = serde_json::json!({
                    "name": server.name,
                    "command": server.command,
                });
                let object = value.as_object_mut().expect("literal json object");
                if !server.args.is_empty() {
                    object.insert(
                        "args".to_string(),
                        Value::Array(
                            server
                                .args
                                .iter()
                                .map(|arg| Value::String(arg.clone()))
                                .collect(),
                        ),
                    );
                }
                if !server.env.is_empty() {
                    object.insert(
                        "env".to_string(),
                        serde_json::to_value(&server.env).expect("env map serializes"),
                    );
                }
                if let Some(cwd) = &server.cwd {
                    object.insert("cwd".to_string(), Value::String(cwd.clone()));
                }
                value
            })
            .collect()
    }

    /// argv 审计行（name + 完整 argv + digest），写入 run 审计/tracing。
    pub fn argv_audit_lines(&self) -> Vec<String> {
        self.servers
            .iter()
            .map(|server| {
                format!(
                    "mcp_server name={} argv=[{}] digest={}",
                    server.name,
                    std::iter::once(server.command.as_str())
                        .chain(server.args.iter().map(String::as_str))
                        .collect::<Vec<_>>()
                        .join(" "),
                    self.digest
                )
            })
            .collect()
    }
}

impl KimiMcpInjection {
    /// 新会话注入：无冻结 digest，resume 校验不适用。
    pub fn for_new_session(bundle: ValidatedMcpServerBundle) -> Self {
        Self {
            bundle,
            frozen_digest: None,
        }
    }

    /// resume 注入：携带上次 run 审计冻结的 bundle digest。
    pub fn for_resume(bundle: ValidatedMcpServerBundle, frozen_digest: String) -> Self {
        Self {
            bundle,
            frozen_digest: Some(frozen_digest),
        }
    }

    pub fn bundle(&self) -> &ValidatedMcpServerBundle {
        &self.bundle
    }
}

/// 纯决策函数：resume_id + 注入意图 → session/load 或（digest 漂移时）session/new
/// 并携带 superseded 审计信息。供 session.rs 消费并被单测直接覆盖。
pub fn resolve_session_method(
    resume_id: Option<&str>,
    injection: Option<&KimiMcpInjection>,
) -> SessionMethodDecision {
    let resume_id = resume_id.map(str::trim).filter(|id| !id.is_empty());
    let mcp_servers = injection
        .map(|injection| injection.bundle.mcp_servers_json())
        .unwrap_or_default();
    let Some(session_id) = resume_id else {
        return SessionMethodDecision::New {
            superseded: None,
            mcp_servers,
        };
    };
    let Some(injection) = injection else {
        return SessionMethodDecision::Load {
            session_id: session_id.to_string(),
            mcp_servers,
        };
    };
    match injection.frozen_digest.as_deref() {
        None => SessionMethodDecision::Load {
            session_id: session_id.to_string(),
            mcp_servers,
        },
        Some(frozen) if frozen == injection.bundle.digest => SessionMethodDecision::Load {
            session_id: session_id.to_string(),
            mcp_servers,
        },
        Some(frozen) => SessionMethodDecision::New {
            superseded: Some(SupersededSession {
                session_id: session_id.to_string(),
                frozen_digest: frozen.to_string(),
                actual_digest: injection.bundle.digest.clone(),
            }),
            mcp_servers,
        },
    }
}

pub fn validate_bundle(
    servers: Vec<McpServerConfig>,
) -> Result<ValidatedMcpServerBundle, McpBundleError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut canonical = servers;
    for server in &canonical {
        if server.name.trim().is_empty() {
            return Err(McpBundleError::new(
                "MCP bundle server name must not be empty",
            ));
        }
        if !seen.insert(server.name.clone()) {
            return Err(McpBundleError::new(format!(
                "MCP bundle server name {} is declared more than once",
                server.name
            )));
        }
        if !KIMI_MCP_SERVER_ALLOWLIST.contains(&server.name.as_str()) {
            return Err(McpBundleError::new(format!(
                "MCP server {} is outside the kimi allowlist {KIMI_MCP_SERVER_ALLOWLIST:?}",
                server.name
            )));
        }
        validate_command(&server.command)?;
        for arg in &server.args {
            if arg.is_empty() || arg.chars().any(|c| c.is_control() || c == '\u{0}') {
                return Err(McpBundleError::new(format!(
                    "MCP server {} argv contains an empty or control-character argument",
                    server.name
                )));
            }
        }
        if let Some(cwd) = &server.cwd {
            validate_directory(cwd).map_err(|error| {
                McpBundleError::new(format!("MCP server {} cwd: {error}", server.name))
            })?;
        }
        for (key, value) in &server.env {
            if key.is_empty()
                || !key
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            {
                return Err(McpBundleError::new(format!(
                    "MCP server {} env key {key} is not an UPPER_SNAKE identifier",
                    server.name
                )));
            }
            validate_credential_reference(value).map_err(|error| {
                McpBundleError::new(format!(
                    "MCP server {} env {key} must be a credential reference (env:<VAR> or file:<relative>): {error}",
                    server.name
                ))
            })?;
        }
    }
    // canonical：按 name 排序后序列化，保证 digest 与声明顺序无关。
    canonical.sort_by(|left, right| left.name.cmp(&right.name));
    let canonical_json = serde_json::to_string(&canonical).expect("bundle serializes");
    let digest = format!("{:x}", sha2::Sha256::digest(canonical_json.as_bytes()));
    Ok(ValidatedMcpServerBundle {
        servers: canonical,
        digest,
    })
}

fn validate_command(command: &str) -> Result<(), McpBundleError> {
    if command.is_empty() {
        return Err(McpBundleError::new("MCP server command must not be empty"));
    }
    let path = PathBuf::from(command);
    if path.is_absolute() {
        if command.chars().any(|c| c.is_control())
            || path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(McpBundleError::new(format!(
                "MCP server command {command} is not a normalized absolute path"
            )));
        }
        return Ok(());
    }
    // 裸程序名只允许 allowlist 内的可信命令（相对路径一律拒绝）。
    if !command.contains('/')
        && !command.contains('\\')
        && KIMI_MCP_SERVER_ALLOWLIST.contains(&command)
    {
        return Ok(());
    }
    Err(McpBundleError::new(format!(
        "MCP server command {command} must be an absolute path or an allowlisted bare command"
    )))
}

fn validate_directory(path: &str) -> Result<(), String> {
    let path = PathBuf::from(path);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!("{path:?} must be an absolute path without '..'"));
    }
    Ok(())
}

fn validate_credential_reference(value: &str) -> Result<(), String> {
    if let Some(var) = value.strip_prefix("env:") {
        if var.is_empty()
            || !var
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(format!(
                "env reference {value} must target an UPPER_SNAKE variable"
            ));
        }
        return Ok(());
    }
    if let Some(relative) = value.strip_prefix("file:") {
        let path = PathBuf::from(relative);
        if relative.is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(format!(
                "file reference {value} must be a relative path without '..'"
            ));
        }
        return Ok(());
    }
    Err(format!(
        "value is not a credential reference (literal credentials are rejected)"
    ))
}

/// 从 Aria 控制的 CodeGraph 安装位置与聚合根构造受控 server 声明（argv 走 mcp 子命令）。
pub fn codegraph_server_config(command: PathBuf, root: &std::path::Path) -> McpServerConfig {
    McpServerConfig {
        name: "codegraph".to_string(),
        command: command.to_string_lossy().into_owned(),
        args: vec!["mcp".to_string(), "serve".to_string()],
        env: BTreeMap::new(),
        cwd: Some(root.to_string_lossy().into_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn codegraph() -> McpServerConfig {
        McpServerConfig {
            name: "codegraph".to_string(),
            command: "/usr/local/bin/codegraph".to_string(),
            args: vec!["mcp".to_string()],
            env: BTreeMap::new(),
            cwd: Some("/repo".to_string()),
        }
    }

    #[test]
    fn validate_accepts_allowlisted_server_and_freezes_stable_digest() {
        let bundle = validate_bundle(vec![codegraph()]).expect("valid bundle");
        assert_eq!(bundle.digest().len(), 64);
        let again = validate_bundle(vec![codegraph()]).expect("valid bundle");
        assert_eq!(bundle.digest(), again.digest());

        let mut drifted = codegraph();
        drifted.args.push("--extra".to_string());
        let drifted_bundle = validate_bundle(vec![drifted]).expect("valid bundle");
        assert_ne!(bundle.digest(), drifted_bundle.digest());
    }

    #[test]
    fn validate_rejects_servers_outside_the_allowlist() {
        let mut server = codegraph();
        server.name = "exfiltrate".to_string();
        let error = validate_bundle(vec![server]).expect_err("allowlist must be enforced");
        assert!(error.message.contains("allowlist"), "{error}");
    }

    #[test]
    fn validate_rejects_relative_or_empty_commands() {
        let mut server = codegraph();
        server.command = "relative/path/codegraph".to_string();
        assert!(validate_bundle(vec![server.clone()]).is_err());

        server.command = String::new();
        assert!(validate_bundle(vec![server]).is_err());
    }

    #[test]
    fn validate_rejects_control_characters_in_args() {
        let mut server = codegraph();
        server.args.push("bad\u{0}arg".to_string());
        assert!(validate_bundle(vec![server]).is_err());
    }

    #[test]
    fn validate_accepts_credential_references_and_rejects_literal_secrets() {
        let mut server = codegraph();
        server.env.insert(
            "CODEGRAPH_TOKEN".to_string(),
            "env:CODEGRAPH_TOKEN".to_string(),
        );
        server.env.insert(
            "CODEGRAPH_KEY_FILE".to_string(),
            "file:secrets/codegraph.key".to_string(),
        );
        let bundle =
            validate_bundle(vec![server.clone()]).expect("reference env values are allowed");
        assert!(!bundle.mcp_servers_json().is_empty());

        // 字面量凭据 fail-closed。
        let mut literal = codegraph();
        literal
            .env
            .insert("CODEGRAPH_TOKEN".to_string(), "sk-live-abcdef".to_string());
        let error = validate_bundle(vec![literal]).expect_err("literal secrets rejected");
        assert!(error.message.contains("credential"), "{error}");

        // file: 引用不得逃逸配置目录。
        let mut escaping = codegraph();
        escaping
            .env
            .insert("KEY".to_string(), "file:../../etc/shadow".to_string());
        assert!(validate_bundle(vec![escaping]).is_err());
    }

    #[test]
    fn mcp_servers_json_matches_acp_shape_and_sanitized_env() {
        let mut server = codegraph();
        server.env.insert(
            "CODEGRAPH_TOKEN".to_string(),
            "env:CODEGRAPH_TOKEN".to_string(),
        );
        let bundle = validate_bundle(vec![server]).expect("valid bundle");
        let servers = bundle.mcp_servers_json();
        assert_eq!(
            servers,
            vec![json!({
                "name": "codegraph",
                "command": "/usr/local/bin/codegraph",
                "args": ["mcp"],
                "env": {"CODEGRAPH_TOKEN": "env:CODEGRAPH_TOKEN"},
                "cwd": "/repo",
            })]
        );
    }

    #[test]
    fn argv_audit_lines_record_name_argv_and_digest() {
        let bundle = validate_bundle(vec![codegraph()]).expect("valid bundle");
        let lines = bundle.argv_audit_lines();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("codegraph"), "{}", lines[0]);
        assert!(
            lines[0].contains("/usr/local/bin/codegraph mcp"),
            "{}",
            lines[0]
        );
        assert!(lines[0].contains(bundle.digest()), "{}", lines[0]);
    }

    #[test]
    fn resolve_session_method_loads_when_digest_matches_frozen_value() {
        let bundle = validate_bundle(vec![codegraph()]).expect("valid bundle");
        let injection = KimiMcpInjection::for_resume(bundle.clone(), bundle.digest().to_string());
        let decision = resolve_session_method(Some("old-session"), Some(&injection));
        assert!(matches!(
            decision,
            SessionMethodDecision::Load { ref session_id, .. } if session_id == "old-session"
        ));
        if let SessionMethodDecision::Load { mcp_servers, .. } = decision {
            assert!(!mcp_servers.is_empty());
        } else {
            panic!("expected Load decision");
        }
    }

    #[test]
    fn resolve_session_method_rejects_drifted_digest_with_superseded_audit() {
        let bundle = validate_bundle(vec![codegraph()]).expect("valid bundle");
        let injection = KimiMcpInjection::for_resume(bundle.clone(), "frozen-digest".to_string());
        let decision = resolve_session_method(Some("old-session"), Some(&injection));
        match decision {
            SessionMethodDecision::New {
                superseded,
                mcp_servers,
            } => {
                assert!(!mcp_servers.is_empty());
                let superseded = superseded.expect("superseded audit info");
                assert_eq!(superseded.session_id, "old-session");
                assert_eq!(superseded.frozen_digest, "frozen-digest");
                assert_eq!(superseded.actual_digest, bundle.digest());
            }
            other => panic!("expected New decision, got {other:?}"),
        }
    }

    #[test]
    fn resolve_session_method_keeps_empty_mcp_servers_without_injection() {
        assert_eq!(
            resolve_session_method(None, None),
            SessionMethodDecision::New {
                superseded: None,
                mcp_servers: Vec::new(),
            }
        );
        assert_eq!(
            resolve_session_method(Some("old"), None),
            SessionMethodDecision::Load {
                session_id: "old".to_string(),
                mcp_servers: Vec::new(),
            }
        );
    }

    #[test]
    fn codegraph_server_config_targets_index_root_with_reference_env() {
        let config = codegraph_server_config(
            PathBuf::from("/usr/local/bin/codegraph"),
            std::path::Path::new("/repo"),
        );
        assert_eq!(config.name, "codegraph");
        assert_eq!(config.command, "/usr/local/bin/codegraph");
        assert_eq!(config.cwd.as_deref(), Some("/repo"));
        assert!(validate_bundle(vec![config]).is_ok());
    }
}
