//! T3 归档装配测试：解压核对 runtime 原值、两类聚合树层次、凭据遮盖、
//! 轨迹投影（无 canary）、缺项/部分失败与限额可见性。
//!
//! 全部通过可注入 `HostPaths` 的私有构造完成：不设置全局环境、不调用
//! `from_host`（它会读真实 HOME），临时产物都落在 tempfile 的 /tmp 下。

use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use flate2::read::GzDecoder;
use serde_json::{Value, json};
use tar::Archive;
use tempfile::{TempDir, tempdir};

use super::archive::{
    self, ARIA_INPUT_CLASS_BYTES, ARIA_OUTPUT_CLASS_BYTES, Budget, HostPaths,
    MANIFEST_OUTPUT_BYTES, ProviderRoot, RUNTIME_INFO_OUTPUT_BYTES, SOURCE_ENV, TOTAL_BUDGET_BYTES,
    TRACE_INPUT_PER_PROVIDER_BYTES, TRACE_OUTPUT_PER_PROVIDER_BYTES,
};
use super::sessions::Session;

struct Member {
    path: String,
    bytes: Vec<u8>,
    mode: u32,
    uid: u64,
    gid: u64,
    regular: bool,
}

fn unpack_all(file: &mut fs::File) -> Vec<Member> {
    file.seek(SeekFrom::Start(0)).expect("seek bundle");
    let mut archive = Archive::new(GzDecoder::new(file));
    let mut members = Vec::new();
    for entry in archive.entries().expect("tar entries") {
        let mut entry = entry.expect("tar entry");
        let (path, mode, uid, gid, regular) = {
            let header = entry.header();
            (
                header.path().expect("path").to_string_lossy().into_owned(),
                header.mode().expect("mode"),
                header.uid().expect("uid"),
                header.gid().expect("gid"),
                header.entry_type() == tar::EntryType::Regular,
            )
        };
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("entry bytes");
        members.push(Member {
            path,
            bytes,
            mode,
            uid,
            gid,
            regular,
        });
    }
    members
}

fn by_path<'a>(members: &'a [Member], path: &str) -> &'a Member {
    members
        .iter()
        .find(|m| m.path == path)
        .unwrap_or_else(|| panic!("missing member {path}"))
}

fn try_path<'a>(members: &'a [Member], path: &str) -> Option<&'a Member> {
    members.iter().find(|m| m.path == path)
}

fn manifest_of(members: &[Member]) -> Value {
    serde_json::from_slice(&by_path(members, "manifest.json").bytes).expect("manifest json")
}

fn bundle_bytes(members: &[Member]) -> Vec<u8> {
    let mut all = Vec::new();
    for m in members {
        all.extend_from_slice(&m.bytes);
    }
    all
}

fn write_file(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
    fs::write(&path, bytes).expect("write file");
}

fn session_at(members: &[&str], age_secs: u64) -> Session {
    Session {
        modified: SystemTime::now()
            .checked_sub(Duration::from_secs(age_secs))
            .expect("time"),
        members: members.iter().map(PathBuf::from).collect(),
    }
}

fn env_root(path: &Path) -> ProviderRoot {
    ProviderRoot {
        path: Some(path.to_path_buf()),
        source: SOURCE_ENV,
        invalid_var: None,
    }
}

fn empty_home() -> TempDir {
    let home = tempdir().expect("home");
    fs::create_dir_all(home.path().join(".aria").join("projects")).expect("aria projects");
    home
}

#[test]
fn runtime_info_is_verbatim_and_layout_is_declared() {
    let home = empty_home();
    let host = HostPaths::from_home_default(home.path());
    let runtime = json!({
        "workspace_root": "/tmp/aria-e2e-workspace",
        "version": "9.9.9-test",
        "nested": {"ports": [4317, 5173], "flag": true},
    });
    let mut file = archive::build(runtime.clone(), &host).expect("bundle");
    let members = unpack_all(&mut file);

    let runtime_member = by_path(&members, "runtime-info.json");
    assert_eq!(
        runtime_member.bytes,
        serde_json::to_vec(&runtime).expect("serialize"),
        "runtime-info.json 必须是原值"
    );
    let parsed: Value = serde_json::from_slice(&runtime_member.bytes).expect("parse runtime");
    assert_eq!(parsed["workspace_root"], "/tmp/aria-e2e-workspace");

    let manifest = manifest_of(&members);
    assert_eq!(manifest["version"], 1);
    assert_eq!(manifest["trace_only"], true);
    let utc = manifest["generated_at_utc"].as_str().expect("utc");
    assert!(utc.ends_with('Z'), "UTC 时间戳应以 Z 结尾: {utc}");
    assert_eq!(manifest["runtime_info_member"], "runtime-info.json");
    for slug in ["claude_code", "codex", "pi", "kimi_code"] {
        assert_eq!(
            manifest["providers"][slug]["status"], "no_records",
            "{slug}"
        );
    }
    assert_eq!(manifest["aria"]["status"], "ok");
    assert_eq!(
        manifest["limits"]["total_budget_bytes"],
        json!(TOTAL_BUDGET_BYTES)
    );
    assert_eq!(
        manifest["limits"]["per_log_tail_window_bytes"],
        8 * 1024 * 1024
    );
}

#[test]
fn aria_trees_keep_hierarchy_mask_credentials_and_skip_unsafe() {
    let home = tempdir().expect("home");
    let projects = home.path().join(".aria").join("projects");
    let init = json!({
        "workspace_root": "/w",
        "model": "keep-me",
        "api_key": "sk-live-CANARY-KEY",
        "env": {"OPENAI_API_KEY": "sk-CANARY-2"},
    });
    write_file(
        &projects,
        "proj-a/aggregate-initializations/init.json",
        &serde_json::to_vec(&init).unwrap(),
    );
    write_file(
        &projects,
        "proj-a/aggregate-initializations/broken.json",
        b"{not json",
    );
    write_file(
        &projects,
        "proj-a/aggregate-recipe-receipts/receipt.json",
        &serde_json::to_vec(&json!({"recipe": "r1", "commands": {"c0": {"exit_code": 0}}}))
            .unwrap(),
    );
    write_file(
        &projects,
        "proj-a/aggregate-recipe-receipts/commands/c1.json",
        &serde_json::to_vec(&json!({"exit_code": 0, "token": "ghp_CANARY3"})).unwrap(),
    );
    write_file(
        &projects,
        "proj-a/aggregate-recipe-receipts/commands/c2.json",
        &serde_json::to_vec(&json!({"ok": true})).unwrap(),
    );
    // 旁文件：两类聚合树之外的内容不收录。
    write_file(&projects, "proj-a/lock.json", b"{}");
    write_file(&projects, "proj-b/state.json", b"{}");
    // 不安全文件名：冒号 / 控制字符 / 反斜杠。
    write_file(
        &projects,
        "proj-a/aggregate-initializations/bad:name.json",
        b"{}",
    );
    write_file(
        &projects,
        "proj-a/aggregate-initializations/ctrl\x01.json",
        b"{}",
    );
    write_file(
        &projects,
        "proj-a/aggregate-initializations/back\\slash.json",
        b"{}",
    );
    #[cfg(unix)]
    {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        use std::os::unix::fs::symlink;
        let non_utf8 = OsString::from_vec(vec![b'x', 0xff, b'.', b'j', b's', b'o', b'n']);
        fs::write(
            projects
                .join("proj-a/aggregate-initializations")
                .join(non_utf8),
            b"{}",
        )
        .expect("non utf8");
        symlink(
            "/etc/hostname",
            projects.join("proj-a/aggregate-initializations/linked.json"),
        )
        .expect("symlink");
    }

    let host = HostPaths::from_home_default(home.path());
    let mut file = archive::build(json!({"v": 1}), &host).expect("bundle");
    let members = unpack_all(&mut file);

    let paths: BTreeSet<String> = members.iter().map(|m| m.path.clone()).collect();
    let expected: BTreeSet<String> = [
        "runtime-info.json",
        "manifest.json",
        "aria/projects/proj-a/aggregate-initializations/init.json",
        "aria/projects/proj-a/aggregate-recipe-receipts/receipt.json",
        "aria/projects/proj-a/aggregate-recipe-receipts/commands/c1.json",
        "aria/projects/proj-a/aggregate-recipe-receipts/commands/c2.json",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(
        paths, expected,
        "只收录两类聚合树内的普通 JSON（含嵌套 commands）"
    );

    let init_back: Value = serde_json::from_slice(
        &by_path(
            &members,
            "aria/projects/proj-a/aggregate-initializations/init.json",
        )
        .bytes,
    )
    .unwrap();
    assert_eq!(init_back["api_key"], "[REDACTED]");
    assert_eq!(init_back["env"]["OPENAI_API_KEY"], "[REDACTED]");
    assert_eq!(init_back["model"], "keep-me");
    assert_eq!(init_back["workspace_root"], "/w");
    let c1: Value = serde_json::from_slice(
        &by_path(
            &members,
            "aria/projects/proj-a/aggregate-recipe-receipts/commands/c1.json",
        )
        .bytes,
    )
    .unwrap();
    assert_eq!(c1["token"], "[REDACTED]");
    assert_eq!(c1["exit_code"], 0);

    let text = String::from_utf8_lossy(&bundle_bytes(&members)).into_owned();
    assert!(!text.contains("sk-live-CANARY-KEY"));
    assert!(!text.contains("sk-CANARY-2"));
    assert!(!text.contains("ghp_CANARY3"));

    let manifest = manifest_of(&members);
    assert_eq!(manifest["aria"]["status"], "ok");
    let skipped = manifest["aria"]["skipped"]
        .as_array()
        .expect("skipped")
        .clone();
    assert!(
        skipped.iter().any(|s| s["path"]
            .as_str()
            .is_some_and(|p| p.ends_with("broken.json"))
            && s["reason"] == "invalid_json"),
        "坏 JSON 记稳定原因: {skipped:?}"
    );
    let unsafe_count = skipped
        .iter()
        .filter(|s| s["reason"] == "unsafe_name")
        .count();
    assert!(unsafe_count >= 3, "不安全文件名逐个记原因: {skipped:?}");
    let included = manifest["aria"]["included"].as_array().expect("included");
    assert_eq!(included.len(), 4);
}

#[test]
fn provider_traces_use_tail_window_and_never_leak_raw_lines() {
    let home = empty_home();
    let codex_root = tempdir().expect("codex root");
    let valid = |ts: &str, body: &str| {
        format!(
            "{{\"timestamp\":\"{ts}\",\"type\":\"message\",\"message\":{{\"content\":\"{body}\"}}}}\n"
        )
    };
    let mut wire_a: Vec<u8> = Vec::new();
    // 窗口外巨行（超过 8 MiB 尾窗，也超过单行上限）：不得以任何形式进入包内。
    wire_a.extend(std::iter::repeat(b'J').take(8 * 1024 * 1024 + 100));
    wire_a.push(b'\n');
    wire_a.extend_from_slice(valid("2026-11-18T01:00:00Z", "CANARY-IN-WINDOW-A").as_bytes());
    wire_a.extend_from_slice(b"{oops\n");
    wire_a.extend(std::iter::repeat(b'B').take(archive::MAX_LINE_BYTES + 64));
    wire_a.push(b'\n');
    wire_a.extend_from_slice(valid("2026-11-18T02:00:00Z", "CANARY-IN-WINDOW-B").as_bytes());
    // 尾行无换行：仍是一条完整记录，应被投影。
    wire_a.extend_from_slice(
        valid("2026-11-18T03:00:00Z", "CANARY-TRAILING")
            .trim_end()
            .as_bytes(),
    );
    write_file(codex_root.path(), "2026/11/18/rollout-a.jsonl", &wire_a);
    write_file(
        codex_root.path(),
        "2026/11/18/rollout-b.jsonl",
        valid("2026-11-18T04:00:00Z", "CANARY-B").as_bytes(),
    );
    write_file(
        codex_root.path(),
        "2026/11/17/rollout-c.jsonl",
        valid("2026-11-17T09:00:00Z", "CANARY-C").as_bytes(),
    );

    let mut host = HostPaths::from_home_default(home.path());
    host.codex = env_root(codex_root.path());
    let injected = [(
        "codex",
        vec![
            session_at(
                &["2026/11/18/rollout-a.jsonl", "2026/11/18/rollout-b.jsonl"],
                0,
            ),
            session_at(&["2026/11/17/rollout-c.jsonl"], 86_400),
        ],
    )];
    let mut file = archive::build_with_sessions(json!({"v": 1}), &host, &injected).expect("bundle");
    let members = unpack_all(&mut file);

    let member_a = by_path(&members, "providers/codex/1/0.jsonl");
    let lines: Vec<&[u8]> = member_a
        .bytes
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(lines.len(), 3, "窗口内 3 条完整可解析记录被投影");
    for line in &lines {
        let projected: Value = serde_json::from_slice(line).expect("每条投影行都是完整 JSON");
        assert!(projected.is_object());
    }
    assert!(
        !by_path(&members, "providers/codex/1/1.jsonl")
            .bytes
            .is_empty()
    );
    assert!(
        !by_path(&members, "providers/codex/2/0.jsonl")
            .bytes
            .is_empty()
    );

    let text = String::from_utf8_lossy(&bundle_bytes(&members)).into_owned();
    for canary in [
        "CANARY-IN-WINDOW-A",
        "CANARY-IN-WINDOW-B",
        "CANARY-TRAILING",
        "CANARY-B",
        "CANARY-C",
        "{oops",
    ] {
        assert!(!text.contains(canary), "包内不得出现原文/canary: {canary}");
    }
    let j_probe: String = std::iter::repeat('J').take(4096).collect();
    assert!(!text.contains(&j_probe), "窗口外巨行不得进入包内");
    let b_probe: String = std::iter::repeat('B').take(4096).collect();
    assert!(!text.contains(&b_probe), "超限巨行不得进入包内");

    let manifest = manifest_of(&members);
    let codex = &manifest["providers"]["codex"];
    assert_eq!(codex["status"], "ok");
    assert_eq!(codex["selected_groups"], 2);
    assert_eq!(codex["included_files"].as_array().map(Vec::len), Some(3));
    let files = codex["files"].as_array().expect("files").clone();
    let a_report = files
        .iter()
        .find(|f| f["group"] == 1 && f["stream"] == 0)
        .expect("a report");
    assert_eq!(a_report["events"], 3);
    assert_eq!(a_report["skipped_lines"]["invalid_json"], 1);
    assert_eq!(a_report["skipped_lines"]["oversized"], 1);
    assert_eq!(a_report["truncated"], true);
    assert!(
        a_report["reasons"]
            .as_array()
            .expect("reasons")
            .iter()
            .any(|r| r == "tail_window")
    );
    assert_eq!(manifest["providers"]["kimi_code"]["status"], "no_records");

    // 清单不泄漏本机源绝对路径。
    let manifest_text =
        String::from_utf8_lossy(&by_path(&members, "manifest.json").bytes).into_owned();
    assert!(!manifest_text.contains(home.path().to_string_lossy().as_ref()));
    assert!(!manifest_text.contains(codex_root.path().to_string_lossy().as_ref()));
}

#[test]
fn missing_or_invalid_roots_are_reported_without_fallback() {
    let home = tempdir().expect("home");
    fs::create_dir_all(home.path().join(".aria").join("projects")).expect("projects");
    // 诱饵：HOME 默认位置的 codex 会话——env 覆盖打不开时绝不能回退采用。
    let bait = home.path().join(".codex").join("sessions");
    fs::create_dir_all(&bait).expect("bait dirs");
    fs::write(bait.join("legacy.jsonl"), b"{}\n").expect("bait file");

    let unused = tempdir().expect("unused");
    let mut host = HostPaths::from_home_default(home.path());
    host.codex = ProviderRoot {
        path: Some(unused.path().join("missing-sessions")),
        source: SOURCE_ENV,
        invalid_var: None,
    };
    let mut file = archive::build(json!({"v": 1}), &host).expect("bundle");
    let members = unpack_all(&mut file);
    let manifest = manifest_of(&members);
    assert_eq!(manifest["providers"]["codex"]["status"], "root_unavailable");
    assert!(
        members.iter().all(|m| !m.path.starts_with("providers/")),
        "无效覆盖不得回退到 HOME 默认根"
    );
    assert_eq!(manifest["providers"]["kimi_code"]["status"], "no_records");

    // 缺 HOME：aria 明确记缺，其余照常局部交付。
    let mut no_home = HostPaths::from_home_default(home.path());
    no_home.home = None;
    no_home.claude_code.path = None;
    no_home.codex.path = None;
    no_home.pi.path = None;
    no_home.kimi_code.path = None;
    let mut file2 = archive::build(json!({"v": 1}), &no_home).expect("bundle2");
    let members2 = unpack_all(&mut file2);
    let manifest2 = manifest_of(&members2);
    assert_eq!(manifest2["aria"]["status"], "home_unavailable");
    assert_eq!(manifest2["providers"]["codex"]["status"], "no_records");
    assert!(try_path(&members2, "runtime-info.json").is_some());
}

#[test]
fn unreadable_selected_member_is_recorded_and_rest_delivered() {
    let home = empty_home();
    let codex_root = tempdir().expect("codex root");
    let line = "{\"timestamp\":\"2026-11-18T05:00:00Z\",\"type\":\"message\",\"message\":{\"content\":\"CANARY-G\"}}\n";
    write_file(codex_root.path(), "good.jsonl", line.as_bytes());
    write_file(codex_root.path(), "locked.jsonl", line.as_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            codex_root.path().join("locked.jsonl"),
            fs::Permissions::from_mode(0o000),
        )
        .expect("chmod");
    }
    // root 运行时 chmod 不生效：据此退化为收录断言，避免环境性误报。
    let can_read_locked = fs::read(codex_root.path().join("locked.jsonl")).is_ok();

    let mut host = HostPaths::from_home_default(home.path());
    host.codex = env_root(codex_root.path());
    let injected = [(
        "codex",
        vec![session_at(&["good.jsonl", "locked.jsonl"], 0)],
    )];
    let mut file = archive::build_with_sessions(json!({"v": 1}), &host, &injected).expect("bundle");
    let members = unpack_all(&mut file);

    assert!(
        !by_path(&members, "providers/codex/1/0.jsonl")
            .bytes
            .is_empty(),
        "可读流照常收录"
    );
    let manifest = manifest_of(&members);
    let files = manifest["providers"]["codex"]["files"]
        .as_array()
        .expect("files")
        .clone();
    let locked = files
        .iter()
        .find(|f| f["stream"] == 1)
        .expect("locked report");
    if can_read_locked {
        assert_eq!(locked["status"], "included");
    } else {
        assert_eq!(locked["status"], "unreadable");
        assert!(
            try_path(&members, "providers/codex/1/1.jsonl").is_none(),
            "不可读成员不得用空文件冒充"
        );
    }
    assert_eq!(manifest["providers"]["codex"]["status"], "ok");
}

#[test]
fn tar_members_are_relative_regular_files_with_fixed_metadata() {
    let home = empty_home();
    let codex_root = tempdir().expect("codex root");
    write_file(codex_root.path(), "s.jsonl", b"{\"type\":\"message\"}\n");
    let mut host = HostPaths::from_home_default(home.path());
    host.codex = env_root(codex_root.path());
    let injected = [("codex", vec![session_at(&["s.jsonl"], 0)])];
    let mut file = archive::build_with_sessions(json!({"v": 1}), &host, &injected).expect("bundle");
    let members = unpack_all(&mut file);
    assert!(members.len() >= 3);
    for m in &members {
        let p = m.path.as_str();
        assert!(!p.starts_with('/'), "相对路径: {p}");
        assert!(!p.contains(".."), "无父目录分量: {p}");
        assert!(!p.contains('\\') && !p.contains(':'), "无反斜杠/冒号: {p}");
        assert!(!p.chars().any(char::is_control), "无控制字符: {p}");
        assert!(m.regular, "普通文件: {p}");
        assert_eq!(m.mode, 0o600, "解包后仅当前用户可读写: {p}");
        assert_eq!(m.uid, 0, "无属主: {p}");
        assert_eq!(m.gid, 0, "无属组: {p}");
    }
}

#[test]
fn budget_grants_respect_class_and_total_caps() {
    let mut b = Budget::default();
    assert_eq!(
        b.grant_aria_input(ARIA_INPUT_CLASS_BYTES + 5),
        ARIA_INPUT_CLASS_BYTES
    );
    assert_eq!(b.exhausted_reason(), Some("budget_exhausted"));
    assert_eq!(b.grant_aria_input(1), 0);
    assert_eq!(b.grant_trace_input(0, 1024), 1024, "其它类别仍有基本份额");
    for i in 0..4 {
        assert_eq!(
            b.grant_trace_input(i, TRACE_INPUT_PER_PROVIDER_BYTES * 2),
            TRACE_INPUT_PER_PROVIDER_BYTES - if i == 0 { 1024 } else { 0 }
        );
    }
    assert_eq!(b.grant_trace_input(1, 1), 0, "类内份额用尽");
    assert_eq!(b.grant_trace_output(0, 1), 1, "输出侧独立计量");

    let mut o = Budget::default();
    assert_eq!(
        o.grant_runtime_output(RUNTIME_INFO_OUTPUT_BYTES),
        RUNTIME_INFO_OUTPUT_BYTES
    );
    assert_eq!(
        o.grant_manifest_output(MANIFEST_OUTPUT_BYTES),
        MANIFEST_OUTPUT_BYTES
    );
    assert_eq!(
        o.grant_aria_output(ARIA_OUTPUT_CLASS_BYTES),
        ARIA_OUTPUT_CLASS_BYTES
    );
    for i in 0..4 {
        assert_eq!(
            o.grant_trace_output(i, TRACE_OUTPUT_PER_PROVIDER_BYTES * 2),
            TRACE_OUTPUT_PER_PROVIDER_BYTES
        );
    }
    assert_eq!(o.grant_aria_output(1), 0, "类内份额用尽");
    assert_eq!(o.grant_trace_output(2, 1), 0);
    assert_eq!(o.exhausted_reason(), Some("budget_exhausted"));
    assert_eq!(o.grant_trace_input(3, 1024), 1024, "输入侧独立计量");
}
