//! 安装失败四分类 + 中文提示 + 手动命令兜底（REQ-PCI-06 / tasks 11.7）。
//!
//! 平台命令**永不带 sudo**（提权与否由用户自行决定）；手动命令 npm 类
//! 带 `--registry`，Kimi 为官方脚本命令。

use serde::{Deserialize, Serialize};

use super::source_table::{InstallSourceRow, manual_install_command};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallFailureKind {
    /// 权限不足（EACCES / permission denied）。
    Permission,
    /// 网络 / 镜像不可达（超时 / 连接失败 / 域名解析失败 / registry 404）。
    Network,
    /// 包名不存在（registry 404 且响应体表明 Not found）。
    PackageNotFound,
    /// 其它非零退出（stderr 摘要）。
    Other,
}

/// 失败载荷（wire 契约：kind / message / manual_command）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallJobFailure {
    pub kind: InstallFailureKind,
    pub message: String,
    pub manual_command: String,
}

/// 分类判定信号。
#[derive(Debug, Clone, Default)]
pub struct InstallFailureSignals {
    pub stderr: String,
    pub stdout: String,
    /// 命令超时（归网络类）。
    pub timed_out: bool,
    /// 安装器本身（npm/bash）缺失的说明（归其它类）。
    pub command_missing: Option<String>,
}

const STDERR_SUMMARY_LIMIT: usize = 200;

/// 分类失败并生成中文提示与手动命令（设计决策 9 的失败表）。
pub fn classify_install_failure(
    row: &InstallSourceRow,
    signals: &InstallFailureSignals,
    registry: &str,
) -> InstallJobFailure {
    let manual_command = manual_install_command(row, registry);
    let haystack = format!("{}\n{}", signals.stdout, signals.stderr).to_lowercase();
    let kind = if signals.timed_out {
        InstallFailureKind::Network
    } else if signals.command_missing.is_some() {
        InstallFailureKind::Other
    } else if haystack.contains("e404") && haystack.contains("not found") {
        // registry 404 且响应体表明包未找到（npm 输出两者都带）。
        InstallFailureKind::PackageNotFound
    } else if haystack.contains("eacces") || haystack.contains("permission denied") {
        InstallFailureKind::Permission
    } else if [
        "etimedout",
        "econnrefused",
        "enotfound",
        "econnreset",
        "getaddrinfo",
        "e404",
        "network",
    ]
    .iter()
    .any(|signal| haystack.contains(signal))
    {
        InstallFailureKind::Network
    } else {
        InstallFailureKind::Other
    };
    let message = match kind {
        InstallFailureKind::Permission => format!(
            "装不上（权限不足）。请以管理员权限手动运行（是否加 sudo 由你决定）：{manual_command}"
        ),
        InstallFailureKind::Network => format!(
            "装不上（网络问题）。请检查网络或更换镜像源（可用 NPM_REGISTRY 覆盖默认 {registry}）后手动运行：{manual_command}"
        ),
        InstallFailureKind::PackageNotFound => {
            format!("装不上（找不到这个包）。请确认包名后手动安装：{manual_command}")
        }
        InstallFailureKind::Other => match &signals.command_missing {
            Some(executable) => format!(
                "安装失败：未找到 {executable} 命令，请先补齐运行环境后重试：{manual_command}"
            ),
            None => format!("安装失败：{}", stderr_summary(signals)),
        },
    };
    InstallJobFailure {
        kind,
        message,
        manual_command,
    }
}

fn stderr_summary(signals: &InstallFailureSignals) -> String {
    let first_line = signals
        .stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .or_else(|| {
            signals
                .stdout
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
        })
        .unwrap_or("安装命令以非零退出码结束");
    first_line.chars().take(STDERR_SUMMARY_LIMIT).collect()
}

#[cfg(test)]
mod tests {
    use super::super::source_table::row_for_wire;
    use super::*;

    fn signals(stderr: &str) -> InstallFailureSignals {
        InstallFailureSignals {
            stderr: stderr.to_string(),
            ..InstallFailureSignals::default()
        }
    }

    /// 测试点 #6：EACCES → 权限类 + 管理员命令（不带 sudo 前缀）。
    #[test]
    fn failure_classify_eacces_is_permission_with_admin_command() {
        let row = row_for_wire("codex").expect("codex row");
        let failure = classify_install_failure(
            row,
            &signals("npm ERR! code EACCES\nnpm ERR! permission denied /usr/lib/node_modules"),
            "https://registry.npmmirror.com",
        );
        assert_eq!(failure.kind, InstallFailureKind::Permission);
        assert!(failure.message.contains("权限不足"), "{}", failure.message);
        assert!(failure.manual_command.contains("--registry="));
        assert!(!failure.manual_command.starts_with("sudo"));

        let denied = classify_install_failure(row, &signals("permission denied"), "r");
        assert_eq!(denied.kind, InstallFailureKind::Permission);
    }

    /// 测试点 #6：超时 / 连接失败 / 域名解析失败 → 网络类。
    #[test]
    fn failure_classify_network_signals() {
        let row = row_for_wire("claude_code").expect("claude row");
        for stderr in [
            "npm ERR! network ETIMEDOUT",
            "npm ERR! network ECONNREFUSED 127.0.0.1:443",
            "npm ERR! network ENOTFOUND registry.npmmirror.com",
        ] {
            let failure =
                classify_install_failure(row, &signals(stderr), "https://registry.npmmirror.com");
            assert_eq!(failure.kind, InstallFailureKind::Network, "{stderr}");
            assert!(failure.message.contains("网络问题"), "{}", failure.message);
            assert!(failure.message.contains("NPM_REGISTRY"));
        }
        // 命令超时同样归网络类。
        let timed_out = classify_install_failure(
            row,
            &InstallFailureSignals {
                timed_out: true,
                ..InstallFailureSignals::default()
            },
            "r",
        );
        assert_eq!(timed_out.kind, InstallFailureKind::Network);
    }

    /// 测试点 #6：registry 404 + Not found → 包不存在类。
    #[test]
    fn failure_classify_registry_404_not_found_is_package_missing() {
        let row = row_for_wire("pi").expect("pi row");
        let failure = classify_install_failure(
            row,
            &signals("npm ERR! code E404\nnpm ERR! 404 Not Found: @earendil-works/pi-coding-agent"),
            "https://registry.npmmirror.com",
        );
        assert_eq!(failure.kind, InstallFailureKind::PackageNotFound);
        assert!(
            failure.message.contains("找不到这个包"),
            "{}",
            failure.message
        );
        assert!(failure.manual_command.contains("--registry="));
    }

    /// 测试点 #6：其它非零退出 → stderr 摘要。
    #[test]
    fn failure_classify_other_returns_stderr_summary() {
        let row = row_for_wire("codegraph").expect("codegraph row");
        let failure = classify_install_failure(
            row,
            &signals("some unexpected engine failure\nsecond line"),
            "https://registry.npmmirror.com",
        );
        assert_eq!(failure.kind, InstallFailureKind::Other);
        assert!(
            failure.message.contains("some unexpected engine failure"),
            "{}",
            failure.message
        );
    }

    /// 测试点 #6：任何分支的命令都不以 sudo 开头；Kimi 给官方脚本命令。
    #[test]
    fn failure_classify_manual_commands_never_sudo_on_any_branch() {
        for wire in ["claude_code", "codex", "pi", "kimi_code", "codegraph"] {
            let row = row_for_wire(wire).expect("row");
            let triggers = [
                signals("EACCES permission denied"),
                signals("ETIMEDOUT"),
                signals("E404\n404 Not Found: pkg"),
                signals("whatever else"),
            ];
            for trigger in triggers {
                let failure =
                    classify_install_failure(row, &trigger, "https://registry.npmmirror.com");
                assert!(
                    !failure.manual_command.starts_with("sudo"),
                    "{wire}: {}",
                    failure.manual_command
                );
                assert!(!failure.message.is_empty(), "{wire} 必须给中文提示");
            }
        }
        let kimi = classify_install_failure(
            row_for_wire("kimi_code").expect("kimi row"),
            &signals("curl: (7) couldn't connect"),
            "https://registry.npmmirror.com",
        );
        assert_eq!(
            kimi.manual_command,
            "curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash"
        );
    }

    /// 安装器本身缺失（npm/bash 不存在）→ 其它类 + 明确说明。
    #[test]
    fn failure_classify_missing_installer_is_other_with_note() {
        let row = row_for_wire("codex").expect("codex row");
        let failure = classify_install_failure(
            row,
            &InstallFailureSignals {
                command_missing: Some("npm".to_string()),
                stderr: "not found".to_string(),
                ..InstallFailureSignals::default()
            },
            "r",
        );
        assert_eq!(failure.kind, InstallFailureKind::Other);
        assert!(failure.message.contains("npm"), "{}", failure.message);
    }
}
