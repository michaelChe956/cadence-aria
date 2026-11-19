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

#[test]
fn pi_collects_root_level_and_cwd_layouts_together_by_mtime() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    // 两种布局混布：根下直接 2 个、一层 cwd 子目录 2 个；合并按 mtime 取最新三个。
    let flat_a = dir.path().join("flat-a.jsonl");
    let flat_b = dir.path().join("flat-b.jsonl");
    let nested_c = dir.path().join("cwd-a").join("nested-c.jsonl");
    let nested_d = dir.path().join("cwd-b").join("nested-d.jsonl");
    for (path, seconds) in [
        (&flat_a, 100u64),
        (&flat_b, 200),
        (&nested_c, 300),
        (&nested_d, 400),
    ] {
        write_file(path, &pi_header(&Uuid::new_v4().to_string()));
        set_mtime(path, seconds);
    }

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("pi", &root, &mut warnings);

    assert_eq!(sessions.len(), 3);
    assert_eq!(
        sessions[0].members,
        vec![PathBuf::from("cwd-b/nested-d.jsonl")]
    );
    assert_eq!(
        sessions[1].members,
        vec![PathBuf::from("cwd-a/nested-c.jsonl")]
    );
    assert_eq!(sessions[2].members, vec![PathBuf::from("flat-b.jsonl")]);
    assert_eq!(sessions[0].modified, epoch(400));
    assert_eq!(sessions[2].modified, epoch(200));
}

#[test]
fn pi_deduplicates_native_session_id_across_root_and_cwd_layouts() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    let session_id = Uuid::new_v4().to_string();
    let flat = dir.path().join("dup.jsonl");
    let nested = dir.path().join("cwd-x").join("dup.jsonl");
    write_file(&flat, &pi_header(&session_id));
    write_file(&nested, &pi_header(&session_id));
    set_mtime(&flat, 100);
    set_mtime(&nested, 700);

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("pi", &root, &mut warnings);

    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].members,
        vec![PathBuf::from("cwd-x").join("dup.jsonl")]
    );
    assert_eq!(sessions[0].modified, epoch(700));
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("同一会话 ID")),
        "重复 ID 应有静态 warning：{warnings:?}"
    );
}

#[test]
fn pi_unrecognized_candidates_with_zero_valid_groups_emit_marker_warning() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    // 四类读到了内容却非原生会话头的候选：非 session 类型、非 JSON、空 id、空文件。
    write_file(&dir.path().join("a.jsonl"), "{\"type\":\"other\"}\n");
    write_file(&dir.path().join("b.jsonl"), "not-json\n");
    write_file(
        &dir.path().join("cwd-a").join("c.jsonl"),
        "{\"type\":\"session\",\"id\":\"\"}\n",
    );
    write_file(&dir.path().join("d.jsonl"), "");
    set_mtime(&dir.path().join("a.jsonl"), 400);
    set_mtime(&dir.path().join("b.jsonl"), 300);
    set_mtime(&dir.path().join("cwd-a").join("c.jsonl"), 200);
    set_mtime(&dir.path().join("d.jsonl"), 100);

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("pi", &root, &mut warnings);

    assert!(sessions.is_empty());
    assert!(
        warnings.contains(&PI_UNRECOGNIZED_WARNING),
        "有候选但零有效会话应有标记 warning：{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("缺少 session 头")),
        "逐文件跳过应有静态 warning：{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("未发现可用会话")),
        "零会话应保留 no_session：{warnings:?}"
    );
    assert!(
        !warnings
            .iter()
            .any(|warning| warning.contains("扫描预算耗尽")),
        "非预算问题不得误报截断：{warnings:?}"
    );
}

#[test]
fn pi_unrecognized_marker_not_emitted_when_valid_group_exists() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    let session_id = Uuid::new_v4().to_string();
    let valid = dir.path().join("valid.jsonl");
    let broken = dir.path().join("cwd-a").join("broken.jsonl");
    write_file(&valid, &pi_header(&session_id));
    write_file(&broken, "{\"type\":\"other\"}\n");
    set_mtime(&valid, 100);
    set_mtime(&broken, 900);

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("pi", &root, &mut warnings);

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].members, vec![PathBuf::from("valid.jsonl")]);
    assert!(
        !warnings.contains(&PI_UNRECOGNIZED_WARNING),
        "仍有有效会话时不得发标记 warning：{warnings:?}"
    );
}

#[test]
fn pi_without_candidates_keeps_plain_no_session_and_no_marker() {
    // 无候选：空根与只有非候选内容的根都不得发“未识别出原生会话”标记。
    for setup in 0..2 {
        let dir = TempDir::new_in("/tmp").expect("临时目录");
        if setup == 1 {
            write_file(&dir.path().join("notes.txt"), "text");
            fs::create_dir_all(dir.path().join("cwd-a")).expect("创建目录");
            write_file(&dir.path().join("cwd-a").join("data.csv"), "x,y");
        }

        let root = open_root(&dir);
        let mut warnings: Vec<&'static str> = Vec::new();
        let sessions = discover("pi", &root, &mut warnings);

        assert!(sessions.is_empty());
        assert!(
            !warnings.contains(&PI_UNRECOGNIZED_WARNING),
            "无候选不得发标记 warning：{warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("未发现可用会话")),
            "零会话应有 no_session：{warnings:?}"
        );
        assert!(
            !warnings.iter().any(|warning| warning.contains("不可读")),
            "无候选不得误报不可读：{warnings:?}"
        );
    }
}

#[test]
fn pi_unreadable_candidate_reports_unreadable_not_marker() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    // 硬链接候选：nlink > 1 被安全层拒绝，属于读失败而非“读了但未识别”。
    let outside = TempDir::new_in("/tmp").expect("临时目录");
    let source = outside.path().join("source.jsonl");
    write_file(&source, &pi_header(&Uuid::new_v4().to_string()));
    let candidate = dir.path().join("hardlinked.jsonl");
    fs::hard_link(&source, &candidate).expect("创建硬链接");
    set_mtime(&source, 100);

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("pi", &root, &mut warnings);

    assert!(sessions.is_empty());
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("会话文件不可读")),
        "读失败应有静态 warning：{warnings:?}"
    );
    assert!(
        !warnings.contains(&PI_UNRECOGNIZED_WARNING),
        "未读到的候选不算未识别：{warnings:?}"
    );
}

#[test]
fn pi_listing_truncation_reports_scan_truncated_honestly() {
    let dir = TempDir::new_in("/tmp").expect("临时目录");
    // 超过单次列举上限 512；readdir 的可见子集不保证按创建顺序。
    for index in 0..600u32 {
        let name = format!("sess-{index:03}.jsonl");
        let path = dir.path().join(&name);
        write_file(&path, &pi_header(&Uuid::new_v4().to_string()));
        set_mtime(&path, 1_000 + u64::from(index));
    }

    let root = open_root(&dir);
    let mut warnings: Vec<&'static str> = Vec::new();
    let sessions = discover("pi", &root, &mut warnings);

    assert_eq!(sessions.len(), 3);
    assert!(
        sessions
            .windows(2)
            .all(|pair| pair[0].modified > pair[1].modified)
    );
    for session in sessions {
        let name = session.members[0].file_stem().unwrap().to_str().unwrap();
        let ordinal: u64 = name.strip_prefix("sess-").unwrap().parse().unwrap();
        assert_eq!(session.modified, epoch(1_000 + ordinal));
    }
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("扫描预算耗尽")),
        "列举截断应如实上报：{warnings:?}"
    );
}
