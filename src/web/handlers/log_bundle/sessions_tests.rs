//! sessions.rs 的外置测试模块：经 sessions.rs 内 `#[path = "sessions_tests.rs"]` 引入。
//!
//! 全部夹具只写入 `/tmp` 下的临时目录，会话 ID 用真实 UUID，不读取真实 HOME、
//! 不接触真实 provider 日志。

use super::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tempfile::TempDir;
use uuid::Uuid;

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

fn pi_header(session_id: &str) -> String {
    format!("{{\"type\":\"session\",\"id\":\"{session_id}\"}}\n")
}

#[test]
fn claude_ranks_by_newest_member_mtime_and_attaches_subagents() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    let session_id = Uuid::new_v4().to_string();
    let parent_a = dir
        .path()
        .join("proj-a")
        .join(format!("{session_id}.jsonl"));
    let subagent_a = dir
        .path()
        .join("proj-a")
        .join(&session_id)
        .join("subagents")
        .join("agent-1.jsonl");
    let other_id = Uuid::new_v4().to_string();
    let parent_b = dir.path().join("proj-b").join(format!("{other_id}.jsonl"));
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
            PathBuf::from("proj-a").join(format!("{session_id}.jsonl")),
            PathBuf::from("proj-a")
                .join(&session_id)
                .join("subagents")
                .join("agent-1.jsonl"),
        ]
    );
    assert_eq!(sessions[0].modified, epoch(300));
    assert_eq!(
        sessions[1].members,
        vec![PathBuf::from("proj-b").join(format!("{other_id}.jsonl"))]
    );
    assert_eq!(sessions[1].modified, epoch(200));
}

#[test]
fn claude_ignores_subagents_without_parent_session() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    let session_id = Uuid::new_v4().to_string();
    let orphan = dir
        .path()
        .join("proj-a")
        .join(&session_id)
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
    let session_id = Uuid::new_v4().to_string();
    let first = dir
        .path()
        .join("proj-a")
        .join(format!("{session_id}.jsonl"));
    let second = dir
        .path()
        .join("proj-b")
        .join(format!("{session_id}.jsonl"));
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
        vec![PathBuf::from("proj-b").join(format!("{session_id}.jsonl"))]
    );
    assert_eq!(sessions[0].modified, epoch(500));
}

#[test]
fn codex_deduplicates_by_native_session_id_and_skips_missing_header() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    let session_id = Uuid::new_v4().to_string();
    let first = dir
        .path()
        .join("2025")
        .join("01")
        .join("02")
        .join(format!("rollout-2025-01-02T00-00-00-{session_id}.jsonl"));
    let second = dir
        .path()
        .join("2025")
        .join("01")
        .join("03")
        .join(format!("rollout-2025-01-03T00-00-00-{session_id}.jsonl"));
    let broken = dir
        .path()
        .join("2025")
        .join("01")
        .join("04")
        .join("rollout-2025-01-04T00-00-00-broken.jsonl");
    write_file(&first, &codex_header(&session_id));
    write_file(&second, &codex_header(&session_id));
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
        vec![
            PathBuf::from("2025/01/03")
                .join(format!("rollout-2025-01-03T00-00-00-{session_id}.jsonl"))
        ]
    );
    assert_eq!(sessions[0].modified, epoch(600));
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("session_meta")),
        "缺头应有静态 warning：{warnings:?}"
    );
}

#[test]
fn pi_reads_first_session_header_and_keeps_at_most_three() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    let mut expected: Vec<PathBuf> = Vec::new();
    for index in 0..5u32 {
        let session_id = Uuid::new_v4().to_string();
        let name = format!("2025-01-0{index}-{session_id}.jsonl");
        let path = dir.path().join("cwd-a").join(&name);
        write_file(&path, &pi_header(&session_id));
        set_mtime(&path, 100 + u64::from(index));
        expected.push(PathBuf::from("cwd-a").join(name));
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
    let session_id = Uuid::new_v4().to_string();
    let first = dir.path().join("cwd-a").join("a.jsonl");
    let second = dir.path().join("cwd-b").join("b.jsonl");
    write_file(&first, &pi_header(&session_id));
    write_file(&second, &pi_header(&session_id));
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
    let session_id = Uuid::new_v4().to_string();
    let other_id = Uuid::new_v4().to_string();
    let main_wire = dir
        .path()
        .join("wd-1")
        .join(format!("session_{session_id}"))
        .join("agents")
        .join("main")
        .join("wire.jsonl");
    let aux_wire = dir
        .path()
        .join("wd-1")
        .join(format!("session_{session_id}"))
        .join("agents")
        .join("aux")
        .join("wire.jsonl");
    let other = dir
        .path()
        .join("wd-2")
        .join(format!("session_{other_id}"))
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
            PathBuf::from("wd-1")
                .join(format!("session_{session_id}"))
                .join("agents")
                .join("main")
                .join("wire.jsonl"),
            PathBuf::from("wd-1")
                .join(format!("session_{session_id}"))
                .join("agents")
                .join("aux")
                .join("wire.jsonl"),
        ]
    );
    assert_eq!(sessions[0].modified, epoch(800));
    assert_eq!(
        sessions[1].members,
        vec![
            PathBuf::from("wd-2")
                .join(format!("session_{other_id}"))
                .join("agents")
                .join("main")
                .join("wire.jsonl")
        ]
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
            .join(format!("{}.jsonl", Uuid::new_v4())),
        "{}\n",
    );
    let codex_dir = TempDir::new_in("/tmp").expect("临时目录");
    let codex_id = Uuid::new_v4().to_string();
    write_file(
        &codex_dir
            .path()
            .join("2025/07/08")
            .join(format!("rollout-2025-07-08T00-00-00-{codex_id}.jsonl")),
        &codex_header(&codex_id),
    );
    let pi_dir = TempDir::new_in("/tmp").expect("临时目录");
    write_file(
        &pi_dir.path().join("cwd").join("s.jsonl"),
        &pi_header(&Uuid::new_v4().to_string()),
    );
    let kimi_dir = TempDir::new_in("/tmp").expect("临时目录");
    write_file(
        &kimi_dir.path().join(format!(
            "wd/session_{}/agents/main/wire.jsonl",
            Uuid::new_v4()
        )),
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
    assert_eq!(discover("pi", &open_root(&pi_dir), &mut warnings).len(), 1);
    assert_eq!(
        discover("kimi_code", &open_root(&kimi_dir), &mut warnings).len(),
        1
    );
    // 形态不匹配时不得把别的 provider 的目录当成会话。
    assert!(discover("codex", &open_root(&claude_dir), &mut warnings).is_empty());
    assert!(discover("pi", &open_root(&kimi_dir), &mut warnings).is_empty());
    // wire 名（下划线）与 provider 家族名（连字符）指向同一 provider。
    assert_eq!(
        discover("claude-code", &open_root(&claude_dir), &mut warnings).len(),
        1
    );
    assert_eq!(
        discover("kimi-code", &open_root(&kimi_dir), &mut warnings).len(),
        1
    );
}

#[test]
fn fewer_than_three_sessions_are_reported_as_missing() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    write_file(
        &dir.path().join("cwd").join("a.jsonl"),
        &pi_header(&Uuid::new_v4().to_string()),
    );
    write_file(
        &dir.path().join("cwd").join("b.jsonl"),
        &pi_header(&Uuid::new_v4().to_string()),
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
        &pi_header(&Uuid::new_v4().to_string()),
    );

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("unknown_provider", &root, &mut warnings);

    assert!(sessions.is_empty());
    assert!(!warnings.is_empty(), "未知 provider 应有静态 warning");
}

#[test]
fn pi_selects_three_newest_sessions_among_many_large_files() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    // 70 个文件：文件名字典序 = 由旧到新，头部很短但正文都超过 64 KiB。
    // 逐个读满 64 KiB 的旧做法会先耗尽探头预算，结果选中最早的三个。
    let mut expected: Vec<PathBuf> = Vec::new();
    for index in 0..70u32 {
        let session_id = Uuid::new_v4().to_string();
        let name = format!("sess-{index:03}-{session_id}.jsonl");
        let path = dir.path().join("cwd-a").join(&name);
        let mut content = pi_header(&session_id);
        content.push_str(&"x".repeat(70 * 1024));
        write_file(&path, &content);
        set_mtime(&path, 1_000 + u64::from(index));
        expected.push(PathBuf::from("cwd-a").join(name));
    }
    expected.reverse();

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("pi", &root, &mut warnings);

    assert_eq!(sessions.len(), 3);
    assert_eq!(sessions[0].members, vec![expected[0].clone()]);
    assert_eq!(sessions[1].members, vec![expected[1].clone()]);
    assert_eq!(sessions[2].members, vec![expected[2].clone()]);
    assert_eq!(sessions[0].modified, epoch(1_069));
    assert_eq!(sessions[2].modified, epoch(1_067));
}

#[test]
fn codex_selects_three_newest_sessions_among_many_large_files() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    let mut expected: Vec<PathBuf> = Vec::new();
    for index in 0..70u32 {
        let session_id = Uuid::new_v4().to_string();
        let name = format!("rollout-2025-01-01T00-00-00-{session_id}.jsonl");
        let path = dir.path().join("2025/01/01").join(&name);
        let mut content = codex_header(&session_id);
        content.push_str(&"x".repeat(70 * 1024));
        write_file(&path, &content);
        set_mtime(&path, 1_000 + u64::from(index));
        expected.push(PathBuf::from("2025/01/01").join(name));
    }
    expected.reverse();

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("codex", &root, &mut warnings);

    assert_eq!(sessions.len(), 3);
    assert_eq!(sessions[0].members, vec![expected[0].clone()]);
    assert_eq!(sessions[1].members, vec![expected[1].clone()]);
    assert_eq!(sessions[2].members, vec![expected[2].clone()]);
    assert_eq!(sessions[0].modified, epoch(1_069));
}

#[test]
fn kimi_reports_member_truncation_when_agent_streams_exceed_limit() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    let session_id = Uuid::new_v4().to_string();
    for index in 0..70u32 {
        write_file(
            &dir.path()
                .join("wd-1")
                .join(format!("session_{session_id}"))
                .join("agents")
                .join(format!("agent-{index:03}"))
                .join("wire.jsonl"),
            "{}\n",
        );
    }

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("kimi_code", &root, &mut warnings);

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].members.len(), MAX_GROUP_MEMBERS);
    assert!(
        warnings.iter().any(|warning| warning.contains("成员过多")),
        "裁剪应有静态 warning：{warnings:?}"
    );
}
