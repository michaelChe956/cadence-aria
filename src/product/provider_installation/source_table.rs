//! 五类对象安装源的单一来源（REQ-PCI-01 / tasks 11.1、11.2）。
//!
//! npm 包名 / 命令名 / Kimi 官方脚本地址**只在此处**定义：UI 展示、平台
//! 安装命令、手动兜底命令全部取自本表，禁止在别处复制包名。默认阿里
//! 镜像 + `NPM_REGISTRY`/`NPM_PREFIX` 环境覆盖；`--prefix` 前缀避开
//! sudo（REQ-PCI-03）。

use std::path::{Path, PathBuf};

/// 默认 npm 镜像：阿里（REQ-PCI-03）。
pub const DEFAULT_NPM_REGISTRY: &str = "https://registry.npmmirror.com";
/// 覆盖镜像的环境变量（供海外用户切换官方 registry）。
pub const NPM_REGISTRY_ENV: &str = "NPM_REGISTRY";
/// 覆盖 npm 全局前缀的环境变量。
pub const NPM_PREFIX_ENV: &str = "NPM_PREFIX";
/// 缺省可写全局前缀（用户 HOME 下，避开需要提权的系统全局目录）。
pub const DEFAULT_NPM_PREFIX_DIR: &str = ".npm-global";
/// Kimi Code 官方安装脚本（非 npm 包，REQ-PCI-01）。
pub const KIMI_INSTALL_SCRIPT_URL: &str = "https://code.kimi.com/kimi-code/install.sh";
/// Kimi Code 安装目录名（**假设**：来自官方脚本的 `KIMI_INSTALL_DIR`
/// 约定；容器实测脚本输出 `Installed to /root/.kimi-code/bin/kimi`，离线无法
/// 二次核实 ⇒ 登记条件以「脚本实际输出 / 实际存在的文件」为准，不依赖本
/// 常量）。
pub const KIMI_INSTALL_DIR_NAME: &str = ".kimi-code";

/// Kimi 安装产物目录（`<home>/.kimi-code/bin`）。
pub fn kimi_install_bin_dir(home_dir: &Path) -> PathBuf {
    home_dir.join(KIMI_INSTALL_DIR_NAME).join("bin")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallSourceRow {
    /// wire 标识（与 `InstallTarget::wire_label` 一致）。
    pub target_wire: &'static str,
    pub display_name: &'static str,
    /// CLI 命令名（`claude` / `codex` / `pi` / `kimi` / `codegraph`）。
    pub command: &'static str,
    /// **安装源**：npm 包名；Kimi Code 为 `None`（官方脚本，MUST NOT 走 npm 路径）。
    pub npm_package: Option<&'static str>,
    /// **版本来源**：用于「有新版」判定的包名（REQ-PCI-07）。
    /// 与安装源相互独立 —— Kimi Code 的安装源是官方脚本，但版本来源是 npm 包
    /// `@moonshot-ai/kimi-code`（官方以该包名发布，`dist-tags.latest` 与官方
    /// `/latest` 端点同版本号）。
    pub version_package: Option<&'static str>,
    /// 官方安装脚本地址；npm 对象为 `None`。
    pub install_script: Option<&'static str>,
}

/// 安装源表（spec REQ-PCI-01 Scenario「五类对象的安装源各自明确」，
/// 数据核实自 Cadence-devbox/devbox/Dockerfile + versions.env）。
pub const INSTALL_SOURCE_TABLE: [InstallSourceRow; 5] = [
    InstallSourceRow {
        target_wire: "claude_code",
        display_name: "Claude Code",
        command: "claude",
        npm_package: Some("@anthropic-ai/claude-code"),
        version_package: Some("@anthropic-ai/claude-code"),
        install_script: None,
    },
    InstallSourceRow {
        target_wire: "codex",
        display_name: "Codex",
        command: "codex",
        npm_package: Some("@openai/codex"),
        version_package: Some("@openai/codex"),
        install_script: None,
    },
    InstallSourceRow {
        target_wire: "pi",
        display_name: "Pi",
        command: "pi",
        npm_package: Some("@earendil-works/pi-coding-agent"),
        version_package: Some("@earendil-works/pi-coding-agent"),
        install_script: None,
    },
    InstallSourceRow {
        target_wire: "kimi_code",
        display_name: "Kimi Code",
        command: "kimi",
        npm_package: None,
        version_package: Some("@moonshot-ai/kimi-code"),
        install_script: Some(KIMI_INSTALL_SCRIPT_URL),
    },
    InstallSourceRow {
        target_wire: "codegraph",
        display_name: "codegraph",
        command: "codegraph",
        npm_package: Some("@colbymchenry/codegraph"),
        version_package: Some("@colbymchenry/codegraph"),
        install_script: None,
    },
];

pub fn row_for_wire(wire: &str) -> Option<&'static InstallSourceRow> {
    INSTALL_SOURCE_TABLE
        .iter()
        .find(|row| row.target_wire == wire)
}

/// 解析生效镜像：`NPM_REGISTRY` 覆盖（非空）→ 默认阿里镜像。
pub fn resolve_npm_registry(env_override: Option<&str>) -> String {
    env_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| DEFAULT_NPM_REGISTRY.to_string())
}

/// 生产入口：读取进程环境解析生效镜像。
pub fn npm_registry() -> String {
    resolve_npm_registry(std::env::var(NPM_REGISTRY_ENV).ok().as_deref())
}

/// 解析 npm 全局前缀：`NPM_PREFIX` 覆盖（非空）→ `$HOME/.npm-global`。
pub fn resolve_npm_prefix(env_override: Option<&str>, home: Option<&Path>) -> PathBuf {
    if let Some(value) = env_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return PathBuf::from(value);
    }
    home.unwrap_or_else(|| Path::new("."))
        .join(DEFAULT_NPM_PREFIX_DIR)
}

/// 生产入口：读取进程环境解析生效前缀。
pub fn npm_prefix() -> PathBuf {
    resolve_npm_prefix(
        std::env::var(NPM_PREFIX_ENV).ok().as_deref(),
        std::env::var_os("HOME").map(PathBuf::from).as_deref(),
    )
}

/// UI 展示用安装来源标签。
pub fn install_source_label(row: &InstallSourceRow) -> String {
    match (row.npm_package, row.install_script) {
        (Some(package), _) => format!("npm {package}"),
        (None, Some(script)) => format!("official script {script}"),
        (None, None) => row.command.to_string(),
    }
}

/// 手动兜底命令（REQ-PCI-06）：npm 带 `--registry` 参数；Kimi 为官方
/// 脚本命令。**永不带 `sudo` 前缀**（提权与否由用户自行决定）。
pub fn manual_install_command(row: &InstallSourceRow, registry: &str) -> String {
    match (row.npm_package, row.install_script) {
        (Some(package), _) => format!("npm install -g --registry={registry} {package}"),
        (None, Some(script)) => format!("curl -fsSL {script} | bash"),
        (None, None) => row.command.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(wire: &str) -> &'static InstallSourceRow {
        row_for_wire(wire).unwrap_or_else(|| panic!("missing row for {wire}"))
    }

    /// 测试点 #1：五类对象的包名/命令名/来源与 spec 逐字一致。
    #[test]
    fn source_table_matches_spec_verbatim() {
        let claude = row("claude_code");
        assert_eq!(claude.npm_package, Some("@anthropic-ai/claude-code"));
        assert_eq!(claude.command, "claude");
        assert_eq!(claude.display_name, "Claude Code");
        assert_eq!(claude.install_script, None);

        let codex = row("codex");
        assert_eq!(codex.npm_package, Some("@openai/codex"));
        assert_eq!(codex.command, "codex");
        assert_eq!(codex.install_script, None);

        let pi = row("pi");
        assert_eq!(pi.npm_package, Some("@earendil-works/pi-coding-agent"));
        assert_eq!(pi.command, "pi");
        assert_eq!(pi.install_script, None);

        let codegraph = row("codegraph");
        // 包名以 spec 原文为准（openspec/specs/provider-cli-installation/spec.md
        // REQ-PCI-01 Scenario「五类对象的安装源各自明确」）：
        // codegraph → npm 包 `@colbymchenry/codegraph` / 命令 `codegraph`。
        assert_eq!(codegraph.npm_package, Some("@colbymchenry/codegraph"));
        assert_eq!(codegraph.command, "codegraph");
        assert_eq!(codegraph.install_script, None);

        assert_eq!(INSTALL_SOURCE_TABLE.len(), 5);
    }

    /// 测试点 #1：Kimi 不在 npm 表内（官方脚本，非 npm 包）。
    #[test]
    fn source_table_kimi_is_official_script_not_npm() {
        let kimi = row("kimi_code");
        assert_eq!(
            kimi.npm_package, None,
            "Kimi Code MUST NOT 走 npm 包路径（REQ-PCI-01）"
        );
        assert_eq!(kimi.command, "kimi");
        assert_eq!(
            kimi.install_script,
            Some("https://code.kimi.com/kimi-code/install.sh")
        );
    }

    /// 版本来源与安装来源分离（REQ-PCI-07）：Kimi Code 的**版本判定源**是 npm 包
    /// `@moonshot-ai/kimi-code`（官方以该包名发布，dist-tags.latest 与官方
    /// `/latest` 端点同版本号），但**安装源**仍是官方脚本（REQ-PCI-01 不变）。
    #[test]
    fn source_table_separates_version_source_from_install_source() {
        for (wire, package) in [
            ("claude_code", "@anthropic-ai/claude-code"),
            ("codex", "@openai/codex"),
            ("pi", "@earendil-works/pi-coding-agent"),
            ("codegraph", "@colbymchenry/codegraph"),
            ("kimi_code", "@moonshot-ai/kimi-code"),
        ] {
            assert_eq!(
                row(wire).version_package,
                Some(package),
                "{wire}: 版本来源包必须明确"
            );
        }
        // Kimi：安装源仍是官方脚本，MUST NOT 走 npm 安装路径。
        let kimi = row("kimi_code");
        assert_eq!(kimi.npm_package, None);
        assert_eq!(kimi.install_script, Some(KIMI_INSTALL_SCRIPT_URL));
    }

    /// 测试点 #5（镜像解析半边）：默认阿里 + NPM_REGISTRY 覆盖。
    #[test]
    fn source_table_registry_default_aliyun_and_env_override() {
        assert_eq!(resolve_npm_registry(None), DEFAULT_NPM_REGISTRY);
        assert_eq!(resolve_npm_registry(Some("  ")), DEFAULT_NPM_REGISTRY);
        assert_eq!(
            resolve_npm_registry(Some("https://registry.npmjs.org")),
            "https://registry.npmjs.org"
        );
    }

    #[test]
    fn source_table_npm_prefix_prefers_env_then_home() {
        assert_eq!(
            resolve_npm_prefix(Some("/opt/npm-global"), Some(Path::new("/home/dev"))),
            PathBuf::from("/opt/npm-global")
        );
        assert_eq!(
            resolve_npm_prefix(None, Some(Path::new("/home/dev"))),
            PathBuf::from("/home/dev/.npm-global")
        );
        assert_eq!(
            resolve_npm_prefix(Some(" "), Some(Path::new("/home/dev"))),
            PathBuf::from("/home/dev/.npm-global")
        );
    }

    /// 手动命令：npm 带 registry 参数；Kimi 为官方脚本命令；永不 sudo。
    #[test]
    fn source_table_manual_commands_carry_registry_and_never_sudo() {
        for wire in ["claude_code", "codex", "pi", "codegraph"] {
            let source_row = row(wire);
            let command = manual_install_command(source_row, DEFAULT_NPM_REGISTRY);
            assert!(
                command.contains("--registry=https://registry.npmmirror.com"),
                "{wire}: {command}"
            );
            assert!(
                command.contains(source_row.npm_package.unwrap()),
                "{wire}: {command}"
            );
            assert!(!command.starts_with("sudo"), "{wire}: {command}");
        }
        let kimi = manual_install_command(row("kimi_code"), DEFAULT_NPM_REGISTRY);
        assert_eq!(
            kimi,
            "curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash"
        );
        assert!(!kimi.starts_with("sudo"));
    }

    /// install_source_label：npm 对象带包名，Kimi 标官方脚本。
    #[test]
    fn source_table_labels_npm_and_official_script() {
        assert_eq!(install_source_label(row("codex")), "npm @openai/codex");
        assert_eq!(
            install_source_label(row("kimi_code")),
            "official script https://code.kimi.com/kimi-code/install.sh"
        );
    }
}
