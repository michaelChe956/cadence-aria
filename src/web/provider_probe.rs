//! 启动时探测外部 provider CLI（codex / claude）是否可用。
//! 缺失仅提示、不阻断 `aria web` 启动——工作台 UI 与 FakeProvider 演示不依赖外部 CLI。

use crate::cross_cutting::adapter_compatibility::default_compatibility_matrix;
use crate::protocol::contracts::ProviderType;

/// 一个待探测的 provider：展示名 + 可执行程序名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProbe {
    pub display: String,
    pub program: String,
}

/// 从兼容性矩阵取出 codex / claude 的程序名（避免硬编码漂移）。
pub fn provider_probes() -> Vec<ProviderProbe> {
    let matrix = default_compatibility_matrix();
    let mut probes = Vec::new();
    for (display, ty) in [
        ("Claude Code", ProviderType::ClaudeCode),
        ("Codex", ProviderType::Codex),
    ] {
        if let Some(entry) = matrix.entry_for(ty) {
            probes.push(ProviderProbe {
                display: display.to_string(),
                program: entry.provider_command.to_string_lossy().to_string(),
            });
        }
    }
    probes
}

/// 纯函数：给定待探测项与「程序是否在 PATH」的判定闭包，返回提示文案。
/// 返回 None 表示全部就绪（无需提示）；Some(text) 为面向用户的中文提示。
pub fn probe_message<F>(probes: &[ProviderProbe], is_on_path: F) -> Option<String>
where
    F: Fn(&str) -> bool,
{
    let missing: Vec<&ProviderProbe> = probes.iter().filter(|p| !is_on_path(&p.program)).collect();
    if missing.is_empty() {
        return None;
    }
    let mut lines = vec![
        "提示：以下 provider CLI 未在 PATH 中找到，相关真实执行功能将不可用（工作台界面与 Fake 演示不受影响）："
            .to_string(),
    ];
    for p in &missing {
        lines.push(format!(
            "  - {} (`{}`)：安装后即可使用其真实 provider。",
            p.display, p.program
        ));
    }
    lines.push("如需启用真实执行，请安装对应 CLI 并确保其在 PATH 中。".to_string());
    Some(lines.join("\n"))
}

/// 真实 PATH 查找：在 `PATH` 各目录下查找可执行文件。
pub fn is_program_on_path(program: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        let candidate = dir.join(program);
        candidate.is_file()
    })
}

/// 启动时调用：探测并把提示打印到 stderr（不阻断）。
pub fn emit_provider_probe_notice() {
    let probes = provider_probes();
    if let Some(msg) = probe_message(&probes, is_program_on_path) {
        eprintln!("{msg}");
    }
}
