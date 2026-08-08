//! 基于共享 Provider 健康快照生成启动提示。
//! 健康状态的唯一 truth 来自 `ProviderHealthService`，本模块不执行独立探测。

use crate::cross_cutting::adapter_compatibility::default_compatibility_matrix;
use crate::cross_cutting::provider_health::{ProviderHealthReasonCode, ProviderHealthSnapshot};
use crate::product::models::ProviderName;
use crate::protocol::contracts::ProviderType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProbe {
    pub display: String,
    pub program: String,
}

/// 旧测试与 runtime 的矩阵读取兼容入口；不执行命令，也不用于 Web 健康状态。
pub fn provider_probes() -> Vec<ProviderProbe> {
    let matrix = default_compatibility_matrix();
    [
        ("Claude Code", ProviderType::ClaudeCode),
        ("Codex", ProviderType::Codex),
    ]
    .into_iter()
    .filter_map(|(display, provider_type)| {
        matrix.entry_for(provider_type).map(|entry| ProviderProbe {
            display: display.to_string(),
            program: entry.provider_command.to_string_lossy().into_owned(),
        })
    })
    .collect()
}

/// 旧集成测试兼容入口；产品启动提示必须调用 `snapshot_probe_message`。
pub fn probe_message<F>(probes: &[ProviderProbe], is_on_path: F) -> Option<String>
where
    F: Fn(&str) -> bool,
{
    let missing = probes
        .iter()
        .filter(|probe| !is_on_path(&probe.program))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return None;
    }
    let mut lines = vec!["以下 provider CLI 不可用：".to_string()];
    for probe in missing {
        lines.push(format!("  - {} (`{}`)", probe.display, probe.program));
    }
    Some(lines.join("\n"))
}

pub fn snapshot_probe_message(snapshot: &ProviderHealthSnapshot, degraded: bool) -> Option<String> {
    let unavailable = snapshot
        .providers
        .iter()
        .filter(|entry| !entry.available)
        .collect::<Vec<_>>();
    if unavailable.is_empty() && !degraded {
        return None;
    }

    let mut lines = vec![format!(
        "Provider 健康状态 generation {}{}：",
        snapshot.generation,
        if degraded { " (degraded)" } else { "" }
    )];
    for entry in unavailable {
        let display = match entry.provider {
            ProviderName::ClaudeCode => "Claude Code",
            ProviderName::Codex => "Codex",
            ProviderName::Pi => "Pi",
            ProviderName::Fake => continue,
        };
        let reason_code = entry
            .reason_code
            .map(reason_code_text)
            .unwrap_or("io_error");
        let reason = entry.reason.as_deref().unwrap_or("provider unavailable");
        lines.push(format!(
            "  - {display}: {reason_code}: {reason}（安装或修复 CLI 后可通过状态 API 重检）"
        ));
    }
    if degraded {
        lines.push(
            "  - 共享健康状态存储 degraded；真实工作流保持阻断，Web 服务继续启动。".to_string(),
        );
    }
    Some(lines.join("\n"))
}

fn reason_code_text(code: ProviderHealthReasonCode) -> &'static str {
    match code {
        ProviderHealthReasonCode::CommandMissing => "command_missing",
        ProviderHealthReasonCode::Timeout => "timeout",
        ProviderHealthReasonCode::NonZeroExit => "non_zero_exit",
        ProviderHealthReasonCode::VersionUnparseable => "version_unparseable",
        ProviderHealthReasonCode::IoError => "io_error",
    }
}

/// 旧 runtime 兼容入口；新 Web 状态、API 和启动提示不得依赖此函数。
pub fn is_program_on_path(program: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        let candidate = dir.join(program);
        candidate.is_file()
    })
}

pub fn emit_provider_probe_notice(snapshot: &ProviderHealthSnapshot, degraded: bool) {
    if let Some(msg) = snapshot_probe_message(snapshot, degraded) {
        eprintln!("{msg}");
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::snapshot_probe_message;
    use crate::cross_cutting::provider_health::{
        ProviderHealthEntry, ProviderHealthReasonCode, ProviderHealthSnapshot,
    };
    use crate::product::models::ProviderName;

    fn entry(provider: ProviderName, available: bool) -> ProviderHealthEntry {
        let checked_at = Utc::now();
        ProviderHealthEntry {
            command: match provider {
                ProviderName::ClaudeCode => "claude --version",
                ProviderName::Codex => "codex --version",
                ProviderName::Pi => "pi --version",
                ProviderName::Fake => "fake",
            }
            .to_string(),
            provider,
            available,
            version: available.then(|| "1.0".to_string()),
            reason_code: (!available).then_some(ProviderHealthReasonCode::CommandMissing),
            reason: (!available).then(|| "not found".to_string()),
            checked_at,
        }
    }

    fn snapshot(claude_available: bool, codex_available: bool) -> ProviderHealthSnapshot {
        let checked_at = Utc::now();
        ProviderHealthSnapshot {
            schema_version: 1,
            generation: 7,
            checked_at,
            providers: vec![
                entry(ProviderName::ClaudeCode, claude_available),
                entry(ProviderName::Codex, codex_available),
            ],
        }
    }

    #[test]
    fn provider_probe_uses_snapshot_and_stays_silent_when_all_available() {
        assert_eq!(snapshot_probe_message(&snapshot(true, true), false), None);
    }

    #[test]
    fn provider_probe_reports_unavailable_provider_from_snapshot() {
        let message = snapshot_probe_message(&snapshot(false, true), false).expect("notice");

        assert!(message.contains("Claude Code"));
        assert!(message.contains("command_missing"));
        assert!(message.contains("not found"));
        assert!(!message.contains("Codex (`codex`)"));
    }

    #[test]
    fn provider_probe_reports_degraded_shared_state() {
        let message = snapshot_probe_message(&snapshot(true, true), true).expect("degraded notice");

        assert!(message.contains("degraded"));
        assert!(message.contains("generation 7"));
    }
}
