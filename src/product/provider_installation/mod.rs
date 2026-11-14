//! provider / codegraph 的安装与更新（REQ-PCI-01 ~ 06，块2 片7A）。
//!
//! 分层：`source_table`（安装源单一来源）→ `registry_version`
//!（`dist-tags.latest` + 24h durable 缓存）→ `job_store`（durable 异步
//! 作业，running → completed/failed）→ `runner`（一个包一次安装、装完
//! 重探 provider health）→ `failure_classify`（四类失败 + 中文提示 +
//! 手动命令，永不自动 sudo）。
//!
//! **安装 ≠ 核验**（REQ-PCI-05）：装完只刷新 provider health，不写
//! capability 记录、不触发核验探针。

pub mod effective_prefix;
pub mod expose;
pub mod failure_classify;
pub mod job_store;
pub mod registry_version;
pub mod runner;
pub mod source_table;

use serde::{Deserialize, Serialize};

use crate::product::models::ProviderName;

/// 安装对象（REQ-PCI-01）：四家 provider + codegraph（同一安装机制）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InstallTarget {
    Provider(ProviderName),
    CodeGraph,
}

impl InstallTarget {
    /// wire 标识（HTTP 路径与 DTO 用，snake_case）。
    pub fn wire_label(&self) -> &'static str {
        match self {
            Self::Provider(ProviderName::ClaudeCode) => "claude_code",
            Self::Provider(ProviderName::Codex) => "codex",
            Self::Provider(ProviderName::Pi) => "pi",
            Self::Provider(ProviderName::KimiCode) => "kimi_code",
            // Fake 仅供测试，不开放安装入口（见 `parse_wire`）。
            Self::Provider(ProviderName::Fake) => "fake",
            Self::CodeGraph => "codegraph",
        }
    }

    /// 解析 wire 标识；`fake` 与未知值一律拒绝。
    pub fn parse_wire(wire: &str) -> Option<Self> {
        match wire {
            "claude_code" => Some(Self::Provider(ProviderName::ClaudeCode)),
            "codex" => Some(Self::Provider(ProviderName::Codex)),
            "pi" => Some(Self::Provider(ProviderName::Pi)),
            "kimi_code" => Some(Self::Provider(ProviderName::KimiCode)),
            "codegraph" => Some(Self::CodeGraph),
            // `fake` 与未知值一律拒绝（Fake 仅供测试，无安装入口）。
            _ => None,
        }
    }

    /// 展示名（与 `/api/providers/status` 的 display_name 同口径）。
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Provider(ProviderName::ClaudeCode) => "Claude Code",
            Self::Provider(ProviderName::Codex) => "Codex",
            Self::Provider(ProviderName::Pi) => "Pi",
            Self::Provider(ProviderName::KimiCode) => "Kimi Code",
            Self::Provider(ProviderName::Fake) => "Fake",
            Self::CodeGraph => "codegraph",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::InstallTarget;
    use crate::product::models::ProviderName;

    #[test]
    fn provider_installation_target_wire_roundtrip() {
        let targets = [
            (
                InstallTarget::Provider(ProviderName::ClaudeCode),
                "claude_code",
            ),
            (InstallTarget::Provider(ProviderName::Codex), "codex"),
            (InstallTarget::Provider(ProviderName::Pi), "pi"),
            (InstallTarget::Provider(ProviderName::KimiCode), "kimi_code"),
            (InstallTarget::CodeGraph, "codegraph"),
        ];
        for (target, wire) in targets {
            assert_eq!(target.wire_label(), wire);
            assert_eq!(InstallTarget::parse_wire(wire), Some(target.clone()));
            assert!(!target.display_name().is_empty());
        }
    }

    #[test]
    fn provider_installation_target_rejects_fake_and_unknown() {
        assert_eq!(InstallTarget::parse_wire("fake"), None);
        assert_eq!(InstallTarget::parse_wire("nope"), None);
        assert_eq!(InstallTarget::parse_wire(""), None);
    }
}
