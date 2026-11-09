// `files.rs` 的内联测试（按仓内惯例拆 `*_tests.inc.rs` + `include!`）。
// 夹具一律落 /tmp（tempfile::TempDir）；符号链接用例仅在 unix 下成立。

#[cfg(test)]
mod knowledge_base_files_tests {
    use super::{
        KnowledgeBaseFileError, KnowledgeBaseRoot, create_exclusive_at, kb_root,
        list_knowledge_base_files, read_knowledge_base_file, write_knowledge_base_file,
    };
    use std::path::PathBuf;
    use std::sync::{Arc, Barrier};

    /// 夹具只落 /tmp：独占一个聚合根，并预建知识库根。
    fn kb_fixture() -> (tempfile::TempDir, PathBuf) {
        let fixture = tempfile::TempDir::new().expect("创建 /tmp 夹具");
        let aggregate_root = fixture.path().to_path_buf();
        std::fs::create_dir_all(kb_root(&aggregate_root)).expect("创建知识库根");
        (fixture, aggregate_root)
    }

    #[test]
    fn read_rejects_parent_directory_escape() {
        let (_fixture, aggregate_root) = kb_fixture();
        let error = read_knowledge_base_file(&aggregate_root, "../../etc/passwd")
            .expect_err("父目录逃逸必须被拒绝");
        assert_eq!(error, KnowledgeBaseFileError::PathEscapes);
    }

    #[test]
    fn read_rejects_absolute_path() {
        let (_fixture, aggregate_root) = kb_fixture();
        let error = read_knowledge_base_file(&aggregate_root, "/etc/passwd")
            .expect_err("绝对路径必须被拒绝");
        assert_eq!(error, KnowledgeBaseFileError::PathEscapes);
    }

    #[cfg(unix)]
    #[test]
    fn read_rejects_symlink_escaping_root() {
        let (fixture, aggregate_root) = kb_fixture();
        let outside = fixture.path().join("outside.md");
        std::fs::write(&outside, "外部内容").expect("写入根外文件");
        let link = kb_root(&aggregate_root).join("link.md");
        std::os::unix::fs::symlink(&outside, &link).expect("创建越界符号链接");

        let error = read_knowledge_base_file(&aggregate_root, "link.md")
            .expect_err("越界符号链接必须被拒绝");
        assert_eq!(error, KnowledgeBaseFileError::PathEscapes);
    }

    #[test]
    fn write_rejects_manifest_yaml() {
        let (_fixture, aggregate_root) = kb_fixture();
        let error = write_knowledge_base_file(&aggregate_root, "manifest.yaml", "x", None, false)
            .expect_err("manifest.yaml 不可写");
        assert_eq!(error, KnowledgeBaseFileError::NotEditable);
    }

    #[test]
    fn write_rejects_non_md_outside_user_input() {
        let (_fixture, aggregate_root) = kb_fixture();
        let error =
            write_knowledge_base_file(&aggregate_root, "evidence/foo.txt", "x", None, false)
                .expect_err("user-input 之外的非 md 不可写");
        assert_eq!(error, KnowledgeBaseFileError::NotEditable);
    }

    #[test]
    fn write_allows_sql_under_user_input() {
        let (_fixture, aggregate_root) = kb_fixture();
        std::fs::create_dir_all(kb_root(&aggregate_root).join("user-input"))
            .expect("创建 user-input 目录");
        let digest = write_knowledge_base_file(
            &aggregate_root,
            "user-input/database-ddl.sql",
            "CREATE TABLE demo (id INT);",
            None,
            false,
        )
        .expect("user-input 下的 sql 可写");
        assert_eq!(digest.len(), 64);
        assert!(digest.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_eq!(
            std::fs::read_to_string(kb_root(&aggregate_root).join("user-input/database-ddl.sql"))
                .expect("读取写入结果"),
            "CREATE TABLE demo (id INT);"
        );
    }

    #[test]
    fn write_conflicts_when_digest_mismatches() {
        let (_fixture, aggregate_root) = kb_fixture();
        write_knowledge_base_file(&aggregate_root, "notes.md", "A", None, false)
            .expect("首次写入成功");
        let stale = crate::cross_cutting::document_ops::compute_sha256(b"stale");

        let error = write_knowledge_base_file(
            &aggregate_root,
            "notes.md",
            "B",
            Some(stale.as_str()),
            false,
        )
        .expect_err("摘要不符必须冲突");
        assert_eq!(error, KnowledgeBaseFileError::Conflict);
        assert_eq!(
            std::fs::read_to_string(kb_root(&aggregate_root).join("notes.md")).expect("读取文件"),
            "A"
        );
    }

    #[test]
    fn write_rejected_while_operation_running() {
        let (_fixture, aggregate_root) = kb_fixture();
        let error = write_knowledge_base_file(&aggregate_root, "notes.md", "A", None, true)
            .expect_err("运行期禁写");
        assert_eq!(error, KnowledgeBaseFileError::OperationRunning);
    }

    #[test]
    fn write_is_atomic_and_leaves_no_temp_files() {
        let (_fixture, aggregate_root) = kb_fixture();
        write_knowledge_base_file(&aggregate_root, "notes.md", "A", None, false).expect("写入成功");

        let leftovers: Vec<String> = std::fs::read_dir(kb_root(&aggregate_root))
            .expect("读取知识库根")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp") || name.contains(".write-"))
            .collect();
        assert!(leftovers.is_empty(), "留下临时文件：{leftovers:?}");
        assert_eq!(
            std::fs::read_to_string(kb_root(&aggregate_root).join("notes.md")).expect("读取文件"),
            "A"
        );
    }

    #[test]
    fn write_rejects_when_base_digest_targets_missing_file() {
        let (_fixture, aggregate_root) = kb_fixture();
        let error = write_knowledge_base_file(
            &aggregate_root,
            "notes.md",
            "A",
            Some("0000000000000000000000000000000000000000000000000000000000000000"),
            false,
        )
        .expect_err("给了摘要但文件不存在必须报未找到");
        assert_eq!(error, KnowledgeBaseFileError::NotFound);
    }

    #[test]
    fn write_accepts_matching_base_digest() {
        let (_fixture, aggregate_root) = kb_fixture();
        let first = write_knowledge_base_file(&aggregate_root, "notes.md", "A", None, false)
            .expect("首次写入成功");
        let second = write_knowledge_base_file(
            &aggregate_root,
            "notes.md",
            "B",
            Some(first.as_str()),
            false,
        )
        .expect("摘要一致应允许写入");
        assert_eq!(
            std::fs::read_to_string(kb_root(&aggregate_root).join("notes.md")).expect("读取文件"),
            "B"
        );
        assert_eq!(second.len(), 64);
    }

    #[test]
    fn read_rejects_file_over_size_limit() {
        let (_fixture, aggregate_root) = kb_fixture();
        let target = kb_root(&aggregate_root).join("big.md");
        // 1 MiB 上限之上一个字节 ⇒ TooLarge。
        std::fs::write(&target, vec![b'a'; 1024 * 1024 + 1]).expect("写超限文件");

        let error =
            read_knowledge_base_file(&aggregate_root, "big.md").expect_err("超限文件必须被拒绝");
        assert_eq!(error, KnowledgeBaseFileError::TooLarge);
    }

    #[test]
    fn kb_root_points_at_canonical_aggregate_child() {
        assert_eq!(
            kb_root(std::path::Path::new("/tmp/aggregate")),
            PathBuf::from("/tmp/aggregate/cadence/knowledge-base")
        );
    }

    #[test]
    fn list_reports_editable_flags_for_tree() {
        let (_fixture, aggregate_root) = kb_fixture();
        std::fs::create_dir_all(kb_root(&aggregate_root).join("user-input")).expect("建目录");
        std::fs::write(kb_root(&aggregate_root).join("manifest.yaml"), "schema").expect("写文件");
        std::fs::write(kb_root(&aggregate_root).join("notes.md"), "笔记").expect("写文件");
        std::fs::write(
            kb_root(&aggregate_root).join("user-input/database-ddl.sql"),
            "CREATE TABLE demo (id INT);",
        )
        .expect("写文件");

        let nodes = list_knowledge_base_files(&aggregate_root).expect("列举知识库文件树");
        let editable = |path: &str| {
            nodes
                .iter()
                .find(|node| node.path == path)
                .map(|node| node.editable)
        };
        assert_eq!(editable("notes.md"), Some(true));
        assert_eq!(editable("manifest.yaml"), Some(false));
        assert_eq!(editable("user-input/database-ddl.sql"), Some(true));
        assert!(
            nodes
                .iter()
                .any(|node| node.path == "user-input" && node.is_dir)
        );
    }

    // -----------------------------------------------------------------------
    // 修复轮 1 回归用例（Critical 1/2/3 + Important 5/8）
    // -----------------------------------------------------------------------

    /// Critical 1：根内别名指向不可写文件时，写入必须被拒绝且原文件不被覆盖。
    #[cfg(unix)]
    #[test]
    fn write_rejects_in_root_alias_to_non_editable_file() {
        let (_fixture, aggregate_root) = kb_fixture();
        let root = kb_root(&aggregate_root);
        std::fs::write(root.join("manifest.yaml"), "schema_version: \"4.0\"").expect("写文件");
        std::os::unix::fs::symlink(root.join("manifest.yaml"), root.join("alias.md"))
            .expect("创建根内别名");

        let error = write_knowledge_base_file(&aggregate_root, "alias.md", "恶意内容", None, false)
            .expect_err("根内别名指向不可写文件必须被拒绝");
        assert_eq!(error, KnowledgeBaseFileError::NotEditable);
        assert_eq!(
            std::fs::read_to_string(root.join("manifest.yaml")).expect("读取 manifest"),
            "schema_version: \"4.0\"",
            "机器管理文件不得被别名覆盖"
        );
    }

    /// Critical 1：根内别名指向 `relation-graph.yaml` 时同样必须被拒绝。
    #[cfg(unix)]
    #[test]
    fn write_rejects_in_root_alias_to_relation_graph() {
        let (_fixture, aggregate_root) = kb_fixture();
        let root = kb_root(&aggregate_root);
        std::fs::write(root.join("relation-graph.yaml"), "nodes: []").expect("写文件");
        std::os::unix::fs::symlink(root.join("relation-graph.yaml"), root.join("graph.md"))
            .expect("创建根内别名");

        let error = write_knowledge_base_file(&aggregate_root, "graph.md", "恶意内容", None, false)
            .expect_err("根内别名指向派生文件必须被拒绝");
        assert_eq!(error, KnowledgeBaseFileError::NotEditable);
        assert_eq!(
            std::fs::read_to_string(root.join("relation-graph.yaml")).expect("读取派生文件"),
            "nodes: []"
        );
    }

    /// Critical 2：父目录是指向根外的符号链接时，写入必须被拒绝且根外目录不得
    /// 收到文件（操作绑定在根目录 fd 上，不按路径二次解析）。
    #[cfg(unix)]
    #[test]
    fn write_rejects_symlinked_parent_directory() {
        let (fixture, aggregate_root) = kb_fixture();
        let root = kb_root(&aggregate_root);
        let outside = fixture.path().join("outside-dir");
        std::fs::create_dir_all(&outside).expect("建根外目录");
        std::os::unix::fs::symlink(&outside, root.join("user-input")).expect("创建越界目录链接");

        let error = write_knowledge_base_file(
            &aggregate_root,
            "user-input/database-ddl.sql",
            "CREATE TABLE demo (id INT);",
            None,
            false,
        )
        .expect_err("父目录是越界符号链接时必须被拒绝");
        assert_eq!(error, KnowledgeBaseFileError::PathEscapes);
        assert!(
            !outside.join("database-ddl.sql").exists(),
            "根外目录不得收到写入内容"
        );
    }

    /// Critical 2：读取同样绑定在根目录 fd 上 ⇒ 父目录换成越界链接后读不到根外。
    #[cfg(unix)]
    #[test]
    fn read_rejects_symlinked_parent_directory() {
        let (fixture, aggregate_root) = kb_fixture();
        let root = kb_root(&aggregate_root);
        let outside = fixture.path().join("outside-dir");
        std::fs::create_dir_all(&outside).expect("建根外目录");
        std::fs::write(outside.join("secret.md"), "根外机密").expect("写根外文件");
        std::os::unix::fs::symlink(&outside, root.join("user-input")).expect("创建越界目录链接");

        let error = read_knowledge_base_file(&aggregate_root, "user-input/secret.md")
            .expect_err("父目录是越界符号链接时读取必须被拒绝");
        assert_eq!(error, KnowledgeBaseFileError::PathEscapes);
    }

    /// Critical 3 + Minor 1：预置的临时文件符号链接不得被跟随、其目标不得被截断。
    ///
    /// 预置的是**当前实现实际使用的临时名**（`.notes.md.write-<pid>` 与
    /// `.notes.md.write-<pid>-N`）⇒ 覆盖换名路径（旧版预置 `.tmp-<pid>-N`
    /// 已不再被实现使用，覆盖不到调用链）。
    #[cfg(unix)]
    #[test]
    fn write_does_not_follow_preplaced_temporary_symlink() {
        let (fixture, aggregate_root) = kb_fixture();
        let root = kb_root(&aggregate_root);
        let victim = fixture.path().join("victim.md");
        std::fs::write(&victim, "重要内容").expect("写根外文件");
        // 覆盖实现的前四个候选名（无序号 + 序号 1..=3）⇒ 实际写入必须换名到
        // 下一个候选名（`-4`）才成功。
        let pid = std::process::id();
        let preplaced: Vec<String> = (0..4)
            .map(|attempt| {
                if attempt == 0 {
                    format!(".notes.md.write-{pid}")
                } else {
                    format!(".notes.md.write-{pid}-{attempt}")
                }
            })
            .collect();
        for name in &preplaced {
            std::os::unix::fs::symlink(&victim, root.join(name)).expect("预置临时符号链接");
        }

        write_knowledge_base_file(&aggregate_root, "notes.md", "正常内容", None, false)
            .expect("正常写入必须成功");
        assert_eq!(
            std::fs::read_to_string(&victim).expect("读取根外文件"),
            "重要内容",
            "预置的临时符号链接目标不得被截断或改写"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("notes.md")).expect("读取写入结果"),
            "正常内容"
        );
        for name in &preplaced {
            assert!(
                root.join(name)
                    .symlink_metadata()
                    .expect("读取预置链接元数据")
                    .file_type()
                    .is_symlink(),
                "预置的临时符号链接不得被替换：{name}"
            );
        }
        // 除预置链接外不得留下任何临时文件 ⇒ 实现确实换名到了新候选名并清理。
        let leftovers: Vec<String> = std::fs::read_dir(&root)
            .expect("读取知识库根")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".write-") && !preplaced.contains(name))
            .collect();
        assert!(leftovers.is_empty(), "留下临时文件：{leftovers:?}");
    }

    /// Important 1：知识库内存在**没有写端的 FIFO** 时，读取必须立即返回
    /// `NotFound`，不得永久阻塞在打开动作上。
    #[cfg(unix)]
    #[test]
    fn read_rejects_fifo_without_blocking() {
        let (_fixture, aggregate_root) = kb_fixture();
        let root = kb_root(&aggregate_root);
        let fifo = root.join("notes.md");
        let fifo_path =
            std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).expect("路径不含 NUL");
        assert_eq!(
            unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o644) },
            0,
            "创建无写端 FIFO"
        );

        let (sender, receiver) = std::sync::mpsc::channel();
        let reader_root = aggregate_root.clone();
        std::thread::spawn(move || {
            let outcome = read_knowledge_base_file(&reader_root, "notes.md");
            let _ = sender.send(outcome);
        });
        let outcome = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("读取无写端 FIFO 必须在超时前返回（不得阻塞在 openat 上）");

        assert_eq!(
            outcome.expect_err("FIFO 不是常规文件，必须被拒绝"),
            KnowledgeBaseFileError::NotFound
        );
    }

    /// Important 1：写入侧打开目标（乐观并发摘要比对）同样不得阻塞在 FIFO 上。
    #[cfg(unix)]
    #[test]
    fn write_does_not_block_on_fifo_target() {
        let (_fixture, aggregate_root) = kb_fixture();
        let root = kb_root(&aggregate_root);
        let fifo = root.join("notes.md");
        let fifo_path =
            std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).expect("路径不含 NUL");
        assert_eq!(
            unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o644) },
            0,
            "创建无写端 FIFO"
        );

        let (sender, receiver) = std::sync::mpsc::channel();
        let writer_root = aggregate_root.clone();
        std::thread::spawn(move || {
            let outcome = write_knowledge_base_file(
                &writer_root,
                "notes.md",
                "内容",
                Some("0000000000000000000000000000000000000000000000000000000000000000"),
                false,
            );
            let _ = sender.send(outcome);
        });
        let outcome = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("写入侧不得阻塞在无写端 FIFO 上");

        assert!(
            outcome.is_err(),
            "无写端 FIFO 的摘要比对必须失败而不是挂住：{outcome:?}"
        );
    }

    /// Critical 3：排他创建原语本身遇到同名符号链接必须失败且不改写其目标。
    #[cfg(unix)]
    #[test]
    fn exclusive_creation_refuses_preplaced_symlink() {
        let (fixture, aggregate_root) = kb_fixture();
        let root = kb_root(&aggregate_root);
        let victim = fixture.path().join("victim.md");
        std::fs::write(&victim, "重要内容").expect("写根外文件");
        std::os::unix::fs::symlink(&victim, root.join("claimed.md")).expect("预置符号链接");

        let anchor = KnowledgeBaseRoot::open_existing(&aggregate_root).expect("锚定知识库根");
        let error = create_exclusive_at(&anchor.fd, std::ffi::OsStr::new("claimed.md"))
            .expect_err("排他创建必须因同名符号链接失败");
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(&victim).expect("读取根外文件"),
            "重要内容",
            "排他创建不得改写符号链接目标"
        );
    }

    /// Important 5：两个并发写者携带同一有效摘要 ⇒ 只有一个成功，另一个冲突。
    #[test]
    fn write_serializes_digest_check_and_commit_for_same_target() {
        let (_fixture, aggregate_root) = kb_fixture();
        write_knowledge_base_file(&aggregate_root, "notes.md", "A", None, false)
            .expect("首次写入成功");
        let base = crate::cross_cutting::document_ops::compute_sha256(b"A");
        let barrier = Arc::new(Barrier::new(2));

        let mut handles = Vec::new();
        for content in ["B", "C"] {
            let aggregate_root = aggregate_root.clone();
            let base = base.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                write_knowledge_base_file(
                    &aggregate_root,
                    "notes.md",
                    content,
                    Some(base.as_str()),
                    false,
                )
            }));
        }
        let outcomes: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().expect("写线程结束"))
            .collect();

        let succeeded: Vec<&String> = outcomes
            .iter()
            .filter_map(|outcome| outcome.as_ref().ok())
            .collect();
        let conflicted: Vec<&KnowledgeBaseFileError> = outcomes
            .iter()
            .filter_map(|outcome| outcome.as_ref().err())
            .collect();
        assert_eq!(succeeded.len(), 1, "并发写只能有一个成功：{outcomes:?}");
        assert_eq!(conflicted.len(), 1, "另一个必须冲突：{outcomes:?}");
        assert_eq!(*conflicted[0], KnowledgeBaseFileError::Conflict);
        let final_content =
            std::fs::read_to_string(kb_root(&aggregate_root).join("notes.md")).expect("读取文件");
        assert!(
            final_content == "B" || final_content == "C",
            "文件内容必须来自成功的那次写入：{final_content}"
        );
    }

    /// Important 8：知识库根的元数据查询失败（此处为「父路径不是目录」）必须
    /// 保留为 IO 错误，不得伪装成空文件树。
    #[test]
    fn list_reports_io_error_when_root_metadata_fails() {
        let fixture = tempfile::TempDir::new().expect("创建 /tmp 夹具");
        let aggregate_root = fixture.path().join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).expect("创建聚合根");
        // `cadence` 是普通文件 ⇒ `cadence/knowledge-base` 的元数据查询报
        // ENOTDIR（不是 NotFound）⇒ 不得折叠成空列表。
        std::fs::write(aggregate_root.join("cadence"), "not a directory").expect("写占位文件");

        let error =
            list_knowledge_base_files(&aggregate_root).expect_err("根目录查询失败必须报 IO 错误");
        assert!(
            matches!(error, crate::product::json_store::ProductStoreError::Io(_)),
            "{error:?}"
        );
    }
}
