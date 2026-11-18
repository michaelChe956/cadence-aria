//! sessions.rs 的外置测试模块：经 sessions.rs 内 `#[path = "sessions_tests.rs"]` 引入。
//!
//! 全部夹具只写入 `/tmp` 下的临时目录，不读取真实 HOME、不接触真实 provider 日志。

use super::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tempfile::TempDir;

use super::super::fs::Root;

fn epoch(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}

fn write_file(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().expect("父目录")).expect("创建目录");
    fs::write(path, content).expect("写入夹具");
}

fn set_mtime(path: &Path, seconds: u64) {
    let file = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("打开夹具");
    file.set_modified(epoch(seconds)).expect("设置修改时间");
}

fn open_root(dir: &TempDir) -> Root {
    let path = fs::canonicalize(dir.path()).expect("解析临时目录");
    Root::open(&path).expect("打开根目录")
}

fn codex_header(session_id: &str) -> String {
    format!(
        "{{\"timestamp\":\"2025-01-02T00:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{session_id}\"}}}}\n"
    )
}

#[test]
fn claude_ranks_by_newest_member_mtime_and_attaches_subagents() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    let parent_a = dir
        .path()
        .join("proj-a")
        .join("11111111-1111-1111-1111-111111111111.jsonl");
    let subagent_a = dir
        .path()
        .join("proj-a")
        .join("11111111-1111-1111-1111-111111111111")
        .join("subagents")
        .join("agent-1.jsonl");
    let parent_b = dir
        .path()
        .join("proj-b")
        .join("22222222-2222-2222-2222-222222222222.jsonl");
    write_file(&parent_a, "{}\n");
    write_file(&subagent_a, "{}\n");
    write_file(&parent_b, "{}\n");
    set_mtime(&parent_a, 100);
    set_mtime(&subagent_a, 300);
    set_mtime(&parent_b, 200);

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("claude_code", &root, &mut warnings);

    assert_eq!(sessions.len(), 2);
    assert_eq!(
        sessions[0].members,
        vec![
            PathBuf::from("proj-a").join("11111111-1111-1111-1111-111111111111.jsonl"),
            PathBuf::from("proj-a")
                .join("11111111-1111-1111-1111-111111111111")
                .join("subagents")
                .join("agent-1.jsonl"),
        ]
    );
    assert_eq!(sessions[0].modified, epoch(300));
    assert_eq!(
        sessions[1].members,
        vec![PathBuf::from("proj-b").join("22222222-2222-2222-2222-222222222222.jsonl")]
    );
    assert_eq!(sessions[1].modified, epoch(200));
}

#[test]
fn claude_ignores_subagents_without_parent_session() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    let orphan = dir
        .path()
        .join("proj-a")
        .join("33333333-3333-3333-3333-333333333333")
        .join("subagents")
        .join("agent-9.jsonl");
    write_file(&orphan, "{}\n");
    set_mtime(&orphan, 400);

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("claude_code", &root, &mut warnings);

    assert!(sessions.is_empty());
    assert!(
        warnings.iter().any(|warning| warning.contains("子代理")),
        "缺项应有静态 warning：{warnings:?}"
    );
    for session in &sessions {
        for member in &session.members {
            assert!(!member.ends_with("agent-9.jsonl"), "孤儿子代理不得进入成员");
        }
    }
}

#[test]
fn claude_duplicate_ids_do_not_consume_extra_slots() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    let first = dir
        .path()
        .join("proj-a")
        .join("44444444-4444-4444-4444-444444444444.jsonl");
    let second = dir
        .path()
        .join("proj-b")
        .join("44444444-4444-4444-4444-444444444444.jsonl");
    write_file(&first, "{}\n");
    write_file(&second, "{}\n");
    set_mtime(&first, 100);
    set_mtime(&second, 500);

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("claude_code", &root, &mut warnings);

    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].members,
        vec![PathBuf::from("proj-b").join("44444444-4444-4444-4444-444444444444.jsonl")]
    );
    assert_eq!(sessions[0].modified, epoch(500));
}

#[test]
fn codex_deduplicates_by_native_session_id_and_skips_missing_header() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    let first = dir
        .path()
        .join("2025")
        .join("01")
        .join("02")
        .join("rollout-2025-01-02T00-00-00-a.jsonl");
    let second = dir
        .path()
        .join("2025")
        .join("01")
        .join("03")
        .join("rollout-2025-01-03T00-00-00-b.jsonl");
    let broken = dir
        .path()
        .join("2025")
        .join("01")
        .join("04")
        .join("rollout-2025-01-04T00-00-00-c.jsonl");
    write_file(&first, &codex_header("codex-session-1"));
    write_file(&second, &codex_header("codex-session-1"));
    write_file(&broken, "not-json\n");
    set_mtime(&first, 100);
    set_mtime(&second, 600);
    set_mtime(&broken, 900);

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("codex", &root, &mut warnings);

    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].members,
        vec![PathBuf::from("2025/01/03").join("rollout-2025-01-03T00-00-00-b.jsonl")]
    );
    assert_eq!(sessions[0].modified, epoch(600));
    assert!(
        warnings.iter().any(|warning| warning.contains("session_meta")),
        "缺头应有静态 warning：{warnings:?}"
    );
}

#[test]
fn pi_reads_first_session_header_and_keeps_at_most_three() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    let mut expected: Vec<PathBuf> = Vec::new();
    for index in 0..5u32 {
        let path = dir
            .path()
            .join("cwd-a")
            .join(format!("2025-01-0{index}-session.jsonl"));
        write_file(&path, &format!("{{\"type\":\"session\",\"id\":\"pi-{index}\"}}\n"));
        set_mtime(&path, 100 + u64::from(index));
        expected.push(PathBuf::from("cwd-a").join(format!("2025-01-0{index}-session.jsonl")));
    }
    let broken = dir.path().join("cwd-a").join("broken.jsonl");
    write_file(&broken, "{\"type\":\"other\"}\n");
    set_mtime(&broken, 999);
    expected.reverse();

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("pi", &root, &mut warnings);

    assert_eq!(sessions.len(), 3);
    assert_eq!(sessions[0].members, vec![expected[0].clone()]);
    assert_eq!(sessions[1].members, vec![expected[1].clone()]);
    assert_eq!(sessions[2].members, vec![expected[2].clone()]);
    assert_eq!(sessions[0].modified, epoch(104));
    assert!(
        warnings.iter().any(|warning| warning.contains("session")),
        "缺头应有静态 warning：{warnings:?}"
    );
}

#[test]
fn pi_deduplicates_native_session_id_across_working_directories() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    let first = dir.path().join("cwd-a").join("a.jsonl");
    let second = dir.path().join("cwd-b").join("b.jsonl");
    write_file(&first, "{\"type\":\"session\",\"id\":\"pi-same\"}\n");
    write_file(&second, "{\"type\":\"session\",\"id\":\"pi-same\"}\n");
    set_mtime(&first, 100);
    set_mtime(&second, 700);

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("pi", &root, &mut warnings);

    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].members,
        vec![PathBuf::from("cwd-b").join("b.jsonl")]
    );
    assert_eq!(sessions[0].modified, epoch(700));
}

#[test]
fn kimi_groups_all_agent_streams_under_one_parent_session() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    let main_wire = dir
        .path()
        .join("wd-1")
        .join("session_kimi-1")
        .join("agents")
        .join("main")
        .join("wire.jsonl");
    let aux_wire = dir
        .path()
        .join("wd-1")
        .join("session_kimi-1")
        .join("agents")
        .join("aux")
        .join("wire.jsonl");
    let other = dir
        .path()
        .join("wd-2")
        .join("session_kimi-2")
        .join("agents")
        .join("main")
        .join("wire.jsonl");
    write_file(&main_wire, "{}\n");
    write_file(&aux_wire, "{}\n");
    write_file(&other, "{}\n");
    set_mtime(&main_wire, 300);
    set_mtime(&aux_wire, 800);
    set_mtime(&other, 200);

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("kimi_code", &root, &mut warnings);

    assert_eq!(sessions.len(), 2);
    assert_eq!(
        sessions[0].members,
        vec![
            PathBuf::from("wd-1/session_kimi-1/agents/main/wire.jsonl"),
            PathBuf::from("wd-1/session_kimi-1/agents/aux/wire.jsonl"),
        ]
    );
    assert_eq!(sessions[0].modified, epoch(800));
    assert_eq!(
        sessions[1].members,
        vec![PathBuf::from("wd-2/session_kimi-2/agents/main/wire.jsonl")]
    );
    assert_eq!(sessions[1].modified, epoch(200));
}

#[test]
fn each_provider_is_discovered_by_its_own_wire_name() {
    let claude_dir = TempDir::new_in("/tmp").expect("临时目录");
    write_file(
        &claude_dir
            .path()
            .join("proj")
            .join("55555555-5555-5555-5555-555555555555.jsonl"),
        "{}\n",
    );
    let codex_dir = TempDir::new_in("/tmp").expect("临时目录");
    write_file(
        &codex_dir
            .path()
            .join("2025/07/08")
            .join("rollout-2025-07-08T00-00-00-a.jsonl"),
        &codex_header("codex-only"),
    );
    let pi_dir = TempDir::new_in("/tmp").expect("临时目录");
    write_file(
        &pi_dir.path().join("cwd").join("s.jsonl"),
        "{\"type\":\"session\",\"id\":\"pi-only\"}\n",
    );
    let kimi_dir = TempDir::new_in("/tmp").expect("临时目录");
    write_file(
        &kimi_dir
            .path()
            .join("wd/session_kimi-only/agents/main/wire.jsonl"),
        "{}\n",
    );

    let mut warnings: Vec<&'static str> = Vec::new();
    assert_eq!(
        discover("claude_code", &open_root(&claude_dir), &mut warnings).len(),
        1
    );
    assert_eq!(
        discover("codex", &open_root(&codex_dir), &mut warnings).len(),
        1
    );
    assert_eq!(
        discover("pi", &open_root(&pi_dir), &mut warnings).len(),
        1
    );
    assert_eq!(
        discover("kimi_code", &open_root(&kimi_dir), &mut warnings).len(),
        1
    );
    // 形态不匹配时不得把别的 provider 的目录当成会话。
    assert!(discover("codex", &open_root(&claude_dir), &mut warnings).is_empty());
    assert!(discover("pi", &open_root(&kimi_dir), &mut warnings).is_empty());
}

#[test]
fn fewer_than_three_sessions_are_reported_as_missing() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    write_file(
        &dir.path().join("cwd").join("a.jsonl"),
        "{\"type\":\"session\",\"id\":\"pi-1\"}\n",
    );
    write_file(
        &dir.path().join("cwd").join("b.jsonl"),
        "{\"type\":\"session\",\"id\":\"pi-2\"}\n",
    );

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("pi", &root, &mut warnings);

    assert_eq!(sessions.len(), 2);
    assert!(
        warnings.iter().any(|warning| warning.contains("少于")),
        "缺项应有静态 warning：{warnings:?}"
    );
}

#[test]
fn unknown_provider_yields_no_sessions_and_a_warning() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    write_file(
        &dir.path().join("cwd").join("a.jsonl"),
        "{\"type\":\"session\",\"id\":\"pi-1\"}\n",
    );

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("unknown_provider", &root, &mut warnings);

    assert!(sessions.is_empty());
    assert!(!warnings.is_empty(), "未知 provider 应有静态 warning");
}
