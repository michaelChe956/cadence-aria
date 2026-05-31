use cadence_aria::web::provider_probe::{ProviderProbe, probe_message, provider_probes};

fn probes() -> Vec<ProviderProbe> {
    vec![
        ProviderProbe {
            display: "Claude Code".into(),
            program: "claude".into(),
        },
        ProviderProbe {
            display: "Codex".into(),
            program: "codex".into(),
        },
    ]
}

#[test]
fn all_present_yields_no_message() {
    let msg = probe_message(&probes(), |_| true);
    assert!(msg.is_none(), "全部就绪时不应提示");
}

#[test]
fn missing_one_lists_it_without_blocking() {
    // claude 缺失、codex 存在
    let msg = probe_message(&probes(), |program| program != "claude").expect("应有提示");
    assert!(msg.contains("Claude Code"), "应列出缺失的 Claude Code");
    assert!(msg.contains("`claude`"), "应含程序名 claude");
}

#[test]
fn missing_all_lists_all() {
    let msg = probe_message(&probes(), |_| false).expect("应有提示");
    assert!(
        msg.contains("Claude Code") && msg.contains("Codex"),
        "应列出全部缺失项"
    );
}

#[test]
fn provider_probes_resolved_from_matrix() {
    // 真实矩阵应解析出 claude 与 codex 两个程序名
    let resolved = provider_probes();
    let programs: Vec<String> = resolved.iter().map(|p| p.program.clone()).collect();
    assert!(programs.iter().any(|p| p == "claude"), "矩阵应含 claude");
    assert!(programs.iter().any(|p| p == "codex"), "矩阵应含 codex");
}
