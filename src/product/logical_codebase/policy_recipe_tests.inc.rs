#[cfg(test)]
mod recipe_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use super::tests::{
        AT_0800, AT_0830, AT_0900, AT_0930, AT_1000, AT_1100, AT_1200, AT_1300, AT_1400,
        PROJECT, expected_policy_text, publisher, seed_final_step_operation,
        seed_operation_for_manifest, sha256_hex_of, write_root_fixture,
    };

    fn output_path_for(paths: &ProductAppPaths, lc_id: &str, operation_id: &str) -> PathBuf {
        paths
            .logical_codebases_root(PROJECT)
            .join(lc_id)
            .join("aggregate-initializations")
            .join(operation_id)
            .join("policy-publication.json")
    }

    fn tamper_output_json(path: &Path, mutate: impl FnOnce(&mut serde_json::Value)) {
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        mutate(&mut value);
        std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }

    /// 来源字节集完整性:入口第一、嵌套排序、CRLF/front matter 原样;
    /// 干扰材料不入正文;正文不含 revision/policy_id/时间/绝对 root。
    #[test]
    fn recipe_policy_collects_full_sorted_source_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_collect", &canonical);

        let artifact = publisher(&store, &canonical)(&manifest, "op_collect", AT_0800).unwrap();

        let expected = expected_policy_text();
        assert_eq!(artifact.policy_text.as_bytes(), expected.as_bytes());
        assert_eq!(artifact.digest, sha256_hex_of(expected.as_bytes()));
        for absent in [
            "mcp",
            "示例模板",
            "成员仓规则",
            "其他规则目录",
            "非 md 笔记",
        ] {
            assert!(
                !artifact.policy_text.contains(absent),
                "正文不得包含干扰材料:{absent}"
            );
        }
        assert!(!artifact.policy_text.contains(&artifact.policy_id));
        assert!(!artifact.policy_text.contains("2026-10-03"));
        assert!(!artifact.policy_text.contains(temp.path().to_str().unwrap()));
    }

    /// locator 与原字节 digest:根下 policy_id 与正文逐字节一致;digest 等于独立 SHA-256。
    #[test]
    fn recipe_policy_publishes_locator_and_raw_digest() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_locator", &canonical);
        let bootstrap = store.ensure_bootstrap(&manifest).unwrap();

        let artifact = publisher(&store, &canonical)(&manifest, "op_locator", AT_0800).unwrap();

        let locator = canonical.join(&artifact.policy_id);
        assert_eq!(
            std::fs::read(&locator).unwrap(),
            artifact.policy_text.as_bytes()
        );
        assert_eq!(artifact.revision, 2);
        assert_eq!(
            artifact.digest,
            sha256_hex_of(artifact.policy_text.as_bytes())
        );
        assert_eq!(
            artifact.policy_id,
            format!("policy/{PROJECT}/{}/2", manifest.logical_codebase_id)
        );
        assert!(!artifact.policy_id.contains("lc_0001"));

        let output = store
            .get_recipe_policy_publication(PROJECT, "op_locator")
            .unwrap()
            .expect("publication output");
        assert_eq!(output.sources[0].relative_path, "AGENTS.md");
        let entry_digest =
            sha256_hex_of("# 根规则\r\n\r\n规则正文见 .claude/rules/ 目录。\r\n".as_bytes());
        assert_eq!(output.rule_digest, entry_digest);
        assert_eq!(output.sources[0].digest, output.rule_digest);
        assert_ne!(output.rule_digest, artifact.digest);
        assert_eq!(output.base_policy.revision, 1);
        assert_eq!(output.base_policy.policy_id, bootstrap.policy_id);
        assert_eq!(output.artifact, artifact);
        assert_eq!(output.canonical_root, canonical);
    }

    /// 同 operation 重入复用候选与时间:更晚时间重入返回逐字节相等的
    /// artifact;output/locator 字节不变;ensure_bootstrap 返回真正文。
    #[test]
    fn recipe_policy_same_operation_reentry_keeps_candidate_and_time() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_reentry", &canonical);
        store.ensure_bootstrap(&manifest).unwrap();

        let publish = publisher(&store, &canonical);
        let first = publish(&manifest, "op_reentry", AT_0800).unwrap();
        let output_path = output_path_for(&paths, "lc_0001", "op_reentry");
        let locator = canonical.join(&first.policy_id);
        let frozen_output = std::fs::read(&output_path).unwrap();
        let frozen_locator = std::fs::read(&locator).unwrap();

        let second = publish(&manifest, "op_reentry", AT_0930).unwrap();
        assert_eq!(second, first);
        assert_eq!(std::fs::read(&output_path).unwrap(), frozen_output);
        assert_eq!(std::fs::read(&locator).unwrap(), frozen_locator);
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), first);

        let bootstrap_view = store.ensure_bootstrap(&manifest).unwrap();
        assert_eq!(bootstrap_view, first);
        assert!(!bootstrap_view.is_bootstrap_placeholder());
    }

    /// 实际新 operation 即使正文不变也递增 revision、换 locator;旧字节不变。
    #[test]
    fn recipe_policy_new_operation_with_unchanged_text_reuses_revision() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_first", &canonical);
        store.ensure_bootstrap(&manifest).unwrap();
        seed_operation_for_manifest(&paths, "lc_0001", "op_second", &manifest);

        let publish = publisher(&store, &canonical);
        let first = publish(&manifest, "op_first", AT_0800).unwrap();

        let first_locator = canonical.join(&first.policy_id);
        let first_output = output_path_for(&paths, "lc_0001", "op_first");
        let frozen_locator = std::fs::read(&first_locator).unwrap();
        let frozen_output = std::fs::read(&first_output).unwrap();

        let second = publish(&manifest, "op_second", AT_0900).unwrap();
        // R1 幂等：正文未变 → 复用当前 revision，不新增正文文件。
        assert_eq!(second.revision, first.revision);
        assert_eq!(second.policy_id, first.policy_id);
        assert_eq!(second.digest, first.digest);
        assert_eq!(std::fs::read(&first_locator).unwrap(), frozen_locator);
        assert_eq!(std::fs::read(&first_output).unwrap(), frozen_output);
        let output_b = store
            .get_recipe_policy_publication(PROJECT, "op_second")
            .unwrap()
            .expect("second publication");
        assert_eq!(output_b.artifact, second);
    }

    /// 来源缺失/空/纯白/缺 language/不可读/非法 UTF-8 一律失败;current
    /// 不变、无候选正文、不落 locator;不可读走 IO seam,不依赖 chmod。
    #[test]
    fn recipe_policy_rejects_missing_empty_invalid_utf8_and_unreadable_sources() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let good_root = temp.path().join("good-root");
        std::fs::create_dir_all(&good_root).unwrap();
        write_root_fixture(&good_root);
        let good_canonical = std::fs::canonicalize(&good_root).unwrap();
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_baseline", &good_canonical);
        let baseline = store.ensure_bootstrap(&manifest).unwrap();

        fn apply_negative_case(name: &str, root: &Path) {
            write_root_fixture(root);
            match name {
                "missing_agents" => std::fs::remove_file(root.join("AGENTS.md")).unwrap(),
                "empty_entry" => std::fs::write(root.join("AGENTS.md"), "").unwrap(),
                "whitespace_entry" => std::fs::write(root.join("AGENTS.md"), " \r\n\t").unwrap(),
                "missing_language" => {
                    std::fs::remove_file(root.join(".claude/rules/language.md")).unwrap()
                }
                "missing_rules_dir" => std::fs::remove_dir_all(root.join(".claude/rules")).unwrap(),
                "entry_is_directory" => {
                    std::fs::remove_file(root.join("AGENTS.md")).unwrap();
                    std::fs::create_dir_all(root.join("AGENTS.md")).unwrap();
                }
                "rules_dir_is_file" => {
                    std::fs::remove_dir_all(root.join(".claude/rules")).unwrap();
                    std::fs::write(root.join(".claude/rules"), "not a dir").unwrap();
                }
                "invalid_utf8" => {
                    std::fs::write(root.join("AGENTS.md"), b"# \xF0\x9F\x92\xA9\xFF\xFE\n").unwrap()
                }
                _ => unreachable!("unknown negative case {name}"),
            }
        }

        let cases = [
            "missing_agents",
            "empty_entry",
            "whitespace_entry",
            "missing_language",
            "missing_rules_dir",
            "entry_is_directory",
            "rules_dir_is_file",
            "invalid_utf8",
        ];
        for name in cases {
            let root = temp.path().join(name);
            std::fs::create_dir_all(&root).unwrap();
            apply_negative_case(name, &root);
            let canonical = std::fs::canonicalize(&root).unwrap();
            let operation_id = format!("op_{name}");
            let case_manifest =
                seed_final_step_operation(&paths, "lc_0001", &operation_id, &canonical);
            assert!(
                publisher(&store, &canonical)(&case_manifest, &operation_id, AT_0800).is_err(),
                "case {name} must fail"
            );
            let current = store.get(PROJECT);
            assert_eq!(
                current.unwrap().unwrap(),
                baseline,
                "case {name}: current 不变"
            );
            let published = store.get_recipe_policy_publication(PROJECT, &operation_id);
            assert!(published.unwrap().is_none(), "case {name}: 无候选成功正文");
            assert!(
                !canonical.join("policy").exists(),
                "case {name}: 不落 locator"
            );
        }
        // 不可读来源:确定性 IO seam 注入(不依赖 chmod 对 root 失效)。
        let seam_manifest =
            seed_final_step_operation(&paths, "lc_0001", "op_unreadable", &good_canonical);
        publish_faults::arm(temp.path(), publish_faults::FaultPhase::SourceRead);
        assert!(
            publisher(&store, &good_canonical)(&seam_manifest, "op_unreadable", AT_0800).is_err()
        );
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), baseline);
        let seam_output = store.get_recipe_policy_publication(PROJECT, "op_unreadable");
        assert!(seam_output.unwrap().is_none());
        assert!(!good_canonical.join("policy").exists());
    }

    /// symlink(来源/父目录/目标)、非普通/不同字节目标均拒绝且用户字节不变;相同字节可复用。
    #[test]
    fn recipe_policy_rejects_symlink_and_existing_different_locator() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_symlink", &canonical);
        store.ensure_bootstrap(&manifest).unwrap();
        let uuid = manifest.logical_codebase_id.to_string();
        let locator = canonical.join(format!("policy/{PROJECT}/{uuid}/2"));
        let candidate = expected_policy_text();
        let publish = publisher(&store, &canonical);
        // 来源文件 symlink:fail-closed,用户 symlink 不被动。
        let rules_a = canonical.join(".claude/rules/a.md");
        let real_target = temp.path().join("real-a.md");
        std::fs::write(&real_target, "规则 A\n").unwrap();
        std::fs::remove_file(&rules_a).unwrap();
        std::os::unix::fs::symlink(&real_target, &rules_a).unwrap();
        assert!(publish(&manifest, "op_symlink", AT_0800).is_err());
        assert!(std::fs::symlink_metadata(&rules_a).unwrap().is_symlink());
        std::fs::remove_file(&rules_a).unwrap();
        std::fs::write(&rules_a, "规则 A\n").unwrap();

        // 来源目录 symlink:fail-closed。
        let sub = canonical.join(".claude/rules/sub");
        let real_dir = temp.path().join("real-sub");
        std::fs::create_dir_all(&real_dir).unwrap();
        std::fs::remove_dir_all(&sub).unwrap();
        std::os::unix::fs::symlink(&real_dir, &sub).unwrap();
        assert!(publish(&manifest, "op_symlink", AT_0800).is_err());
        std::fs::remove_file(&sub).unwrap();
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("nested.md"), "嵌套规则\n").unwrap();

        // 政策父目录 symlink(root/policy 指向外部目录):fail-closed。
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, canonical.join("policy")).unwrap();
        assert!(publish(&manifest, "op_symlink", AT_0800).is_err());
        std::fs::remove_file(canonical.join("policy")).unwrap();

        // 目标为 symlink:fail-closed,链接不被动。
        std::fs::create_dir_all(locator.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(temp.path().join("elsewhere.txt"), &locator).unwrap();
        assert!(publish(&manifest, "op_symlink", AT_0800).is_err());
        assert!(std::fs::symlink_metadata(&locator).unwrap().is_symlink());
        std::fs::remove_file(&locator).unwrap();

        // 目标为目录(非普通文件):fail-closed。
        std::fs::create_dir_all(&locator).unwrap();
        assert!(publish(&manifest, "op_symlink", AT_0800).is_err());
        std::fs::remove_dir_all(&locator).unwrap();

        // 目标为不同字节的普通文件:冲突不覆盖,用户字节不变。
        std::fs::write(&locator, b"user-owned bytes\n").unwrap();
        assert!(publish(&manifest, "op_symlink", AT_0800).is_err());
        assert_eq!(std::fs::read(&locator).unwrap(), b"user-owned bytes\n");

        // 相同字节普通目标:复验后复用,发布成功。
        std::fs::write(&locator, candidate.as_bytes()).unwrap();
        let artifact = publish(&manifest, "op_symlink", AT_0800).unwrap();
        assert_eq!(std::fs::read(&locator).unwrap(), candidate.as_bytes());
        assert_eq!(artifact.policy_text, candidate);
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), artifact);
    }

    /// source/清单/operation/root/scope/UUID/base 漂移与 revision 溢出均冲突;不覆盖 current。
    #[test]
    fn recipe_policy_rejects_source_identity_base_drift_and_overflow() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_drift", &canonical);
        store.ensure_bootstrap(&manifest).unwrap();
        let publish = publisher(&store, &canonical);
        let first = publish(&manifest, "op_drift", AT_0800).unwrap();
        let output_path = output_path_for(&paths, "lc_0001", "op_drift");
        let frozen = std::fs::read(&output_path).unwrap();
        let current = first.clone();
        let original_entry = "# 根规则\r\n\r\n规则正文见 .claude/rules/ 目录。\r\n";

        // source 字节漂移。
        std::fs::write(canonical.join("AGENTS.md"), "# 漂移后的根规则\n").unwrap();
        assert!(publish(&manifest, "op_drift", AT_0830).is_err());
        assert_eq!(std::fs::read(&output_path).unwrap(), frozen);
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), current);
        std::fs::write(canonical.join("AGENTS.md"), original_entry).unwrap();

        // source 清单漂移(新增规则文件)。
        std::fs::write(canonical.join(".claude/rules/zz.md"), "新规则\n").unwrap();
        assert!(publish(&manifest, "op_drift", AT_0830).is_err());
        assert_eq!(std::fs::read(&output_path).unwrap(), frozen);
        std::fs::remove_file(canonical.join(".claude/rules/zz.md")).unwrap();

        // operation 引用漂移(篡改 output 的 operation_id)。
        tamper_output_json(&output_path, |value| {
            value["operation_id"] = serde_json::json!("op_other")
        });
        assert!(publish(&manifest, "op_drift", AT_0830).is_err());
        std::fs::write(&output_path, &frozen).unwrap();

        // root 引用漂移(同 operation 换 canonical root 参数)。
        let other_root = temp.path().join("other-root");
        std::fs::create_dir_all(&other_root).unwrap();
        write_root_fixture(&other_root);
        let other_canonical = std::fs::canonicalize(&other_root).unwrap();
        assert!(publisher(&store, &other_canonical)(&manifest, "op_drift", AT_0830).is_err());

        // UUID 引用漂移(同 root、不同 manifest UUID)。
        let drifted_manifest =
            LogicalCodebaseManifest::new(PROJECT, canonical.to_path_buf(), vec![]);
        assert!(publish(&drifted_manifest, "op_drift", AT_0830).is_err());

        // scope 引用漂移(篡改 output 的 lc_id)。
        tamper_output_json(&output_path, |value| {
            value["lc_id"] = serde_json::json!("lc_other")
        });
        assert!(publish(&manifest, "op_drift", AT_0830).is_err());
        std::fs::write(&output_path, &frozen).unwrap();

        // base 三元引用漂移(候选 revision 不再与 base 自洽)。
        tamper_output_json(&output_path, |value| {
            value["base_policy"]["revision"] = serde_json::json!(7)
        });
        assert!(publish(&manifest, "op_drift", AT_0830).is_err());
        std::fs::write(&output_path, &frozen).unwrap();
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), current);

        // revision 溢出:base revision 为 u64::MAX 时 checked_add 拒绝。
        let overflow_store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_overflow");
        let overflow_manifest =
            seed_final_step_operation(&paths, "lc_overflow", "op_overflow", &canonical);
        let max_artifact = AggregatePolicyArtifact {
            policy_id: format!(
                "policy/{PROJECT}/{}/{}",
                overflow_manifest.logical_codebase_id,
                u64::MAX
            ),
            project_id: PROJECT.to_string(),
            logical_codebase_id: overflow_manifest.logical_codebase_id.to_string(),
            revision: u64::MAX,
            digest: sha256_hex_of(b"max\n"),
            policy_text: "max\n".to_string(),
            created_at: "2026-10-03T00:00:00Z".to_string(),
        };
        overflow_store.save(PROJECT, &max_artifact).unwrap();
        assert!(
            publisher(&overflow_store, &canonical)(&overflow_manifest, "op_overflow", AT_0900)
                .is_err()
        );
        assert_eq!(overflow_store.get(PROJECT).unwrap().unwrap(), max_artifact);
        assert!(
            overflow_store
                .get_recipe_policy_publication(PROJECT, "op_overflow")
                .unwrap()
                .is_none()
        );
    }

    /// output/正文/current 三阶段故障分别保留已完成前缀;失败不返回成功;
    /// 清 staging 后 output 仍在;重入复用同候选与时间。
    #[test]
    fn recipe_policy_durable_write_failure_preserves_resume_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");

        let publish = publisher(&store, &canonical);
        // 阶段一:不可变 output 写入失败 → 无候选落盘。
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_resume", &canonical);
        let bootstrap = store.ensure_bootstrap(&manifest).unwrap();
        publish_faults::arm(temp.path(), publish_faults::FaultPhase::OutputWrite);
        assert!(publish(&manifest, "op_resume", AT_0800).is_err());
        assert!(!output_path_for(&paths, "lc_0001", "op_resume").exists());
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), bootstrap);
        // 重入(output 未冻结):发布成功,时间取本次调用。
        let resumed = publish(&manifest, "op_resume", AT_1000).unwrap();
        assert_eq!(resumed.revision, 2);
        assert_eq!(resumed.created_at, "2026-10-03T10:00:00Z");
        // 阶段二:locator 发布失败 → output 前缀保留,current 不变。
        seed_operation_for_manifest(&paths, "lc_0001", "op_resume2", &manifest);
        // 改一个来源以产生新正文/新 revision，否则幂等复用不会写 locator。
        std::fs::write(canonical.join(".claude/rules/b.md"), "规则 B 变更\n").unwrap();
        publish_faults::arm(temp.path(), publish_faults::FaultPhase::LocatorPublish);
        assert!(publish(&manifest, "op_resume2", AT_1100).is_err());
        let output2 = output_path_for(&paths, "lc_0001", "op_resume2");
        // R1 原子提交：locator 写失败即整体回滚，不得留下回执/正文。
        assert!(!output2.exists(), "失败的发布不得留下回执");
        assert!(
            !canonical
                .join(format!(
                    "policy/{PROJECT}/{}/3",
                    manifest.logical_codebase_id
                ))
                .exists()
        );
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), resumed);

        // 重入：无冻结候选，时间取本次调用。
        let done2 = publish(&manifest, "op_resume2", AT_1200).unwrap();
        assert_eq!(done2.revision, 3);
        assert_eq!(done2.created_at, "2026-10-03T12:00:00Z");
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), done2);

        // 阶段三：current artifact 替换失败 → 正文/回执整体回滚。
        seed_operation_for_manifest(&paths, "lc_0001", "op_resume3", &manifest);
        std::fs::write(canonical.join(".claude/rules/b.md"), "规则 B 再次变更\n").unwrap();
        publish_faults::arm(temp.path(), publish_faults::FaultPhase::ArtifactSave);
        assert!(publish(&manifest, "op_resume3", AT_1300).is_err());
        let output3 = output_path_for(&paths, "lc_0001", "op_resume3");
        let locator4 = canonical.join(format!(
            "policy/{PROJECT}/{}/4",
            manifest.logical_codebase_id
        ));
        assert!(!output3.exists(), "失败的发布不得留下回执");
        assert!(!locator4.exists(), "失败的发布不得留下正文");
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), done2);

        let done3 = publish(&manifest, "op_resume3", AT_1400).unwrap();
        assert_eq!(done3.revision, 4);
        assert_eq!(done3.created_at, "2026-10-03T14:00:00Z");
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), done3);
    }

    /// R3：来源读出后立刻被外部同步改写（合成窗口内）——读窗口前后
    /// stat 失稳且 seam 持久武装，重试同样失稳 → fail-closed 拒绝拼出
    /// 混合正文：无候选、不落 locator、current 不变。
    #[test]
    fn recipe_policy_refuses_mixed_body_when_source_rewritten_after_read() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_r3_rewrite", &canonical);
        let bootstrap = store.ensure_bootstrap(&manifest).unwrap();

        // 模拟外部进程在来源读出后立刻整文件改写（同 inode、新 mtime/长度）。
        publish_faults::arm_source_mutator(
            &canonical,
            ".claude/rules/a.md",
            "外部同步改写后的规则 A\n".as_bytes(),
        );
        let publish = publisher(&store, &canonical);
        let Err(error) = publish(&manifest, "op_r3_rewrite", AT_0800) else {
            panic!("读后改写必须被检测，绝不拼出混合正文");
        };
        match error {
            ProductStoreError::InvalidRecord { reason, .. } => assert!(
                reason.contains("policy_sources_unstable"),
                "错误应指明来源不稳定: {reason}"
            ),
            other => panic!("应报 InvalidRecord，实际: {other}"),
        }
        // seam 确已开火：磁盘上的来源已是改写字节（失败确因改写被检测）。
        assert_eq!(
            std::fs::read(canonical.join(".claude/rules/a.md")).unwrap(),
            "外部同步改写后的规则 A\n".as_bytes().to_vec()
        );
        // fail-closed 证据链：无候选正文、不落 locator、current 保持 bootstrap。
        assert!(
            store
                .get_recipe_policy_publication(PROJECT, "op_r3_rewrite")
                .unwrap()
                .is_none()
        );
        assert!(!canonical.join("policy").exists());
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), bootstrap);
        // seam 持久武装：新一次发布同样检测到改写并 fail-closed。
        assert!(publish(&manifest, "op_r3_rewrite", AT_0900).is_err());
    }

    /// R2：根规则变更后用当前源重新发布——推进 revision、记录 supersede、
    /// 同 command 重放幂等。
    #[test]
    fn republish_after_rule_drift_advances_and_is_replay_safe() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_drift", &canonical);
        store.ensure_bootstrap(&manifest).unwrap();
        let publish = publisher(&store, &canonical);
        let first = publish(&manifest, "op_drift", AT_0800).unwrap();
        assert_eq!(first.revision, 2);

        // 根规则变更 → 重发必须用新源合成新 revision。
        std::fs::write(
            canonical.join("AGENTS.md"),
            "# 根规则\r\n\r\n规则正文见 .claude/rules/ 目录。\r\n新增一行。\r\n",
        )
        .unwrap();
        let (_record, replayed) = store
            .republish_policy(&manifest, &canonical, "cmd_republish", AT_0900.to_string())
            .unwrap();
        assert!(!replayed, "首次重发不是重放");
        let current = store.get(PROJECT).unwrap().unwrap();
        assert_eq!(current.revision, 3, "重发必须推进 revision");
        assert!(current.policy_text.contains("新增一行。"));

        // 同 command 重放：返回既有记录，不再推进。
        let (_same, replayed) = store
            .republish_policy(&manifest, &canonical, "cmd_republish", AT_1000.to_string())
            .unwrap();
        assert!(replayed, "同 command 必须识别为重放");
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), current);

        // 源未再变：新 command 幂等复用当前 revision。
        let (_again, replayed) = store
            .republish_policy(&manifest, &canonical, "cmd_republish_2", AT_1100.to_string())
            .unwrap();
        assert!(!replayed);
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), current);
    }

    /// 发布与 save/ensure_bootstrap 共用 scope 锁:并发后只有合法
    /// successor 或冲突,无丢失更新、无重复候选;不同 scope 不共锁。
    #[test]
    fn recipe_policy_all_writers_share_scope_lock() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_race", &canonical);
        let bootstrap = store.ensure_bootstrap(&manifest).unwrap();

        let (sender, receiver) = std::sync::mpsc::channel::<(
            &'static str,
            Result<Option<AggregatePolicyArtifact>, ProductStoreError>,
        )>();
        let mut handles = Vec::new();
        for index in 0..3 {
            let store = store.clone();
            let bootstrap = bootstrap.clone();
            let sender = sender.clone();
            handles.push(std::thread::spawn(move || {
                let candidate = bootstrap.with_revised_policy(
                    format!("并发修订 {index}\n"),
                    format!("2026-10-03T09:00:0{index}Z"),
                );
                sender
                    .send(("save", store.save(PROJECT, &candidate).map(|()| None)))
                    .unwrap();
            }));
        }
        for _ in 0..2 {
            let store = store.clone();
            let manifest = manifest.clone();
            let canonical = canonical.clone();
            let sender = sender.clone();
            handles.push(std::thread::spawn(move || {
                let published = store
                    .publish_recipe_policy(&manifest, "op_race", &canonical, AT_0930.to_string())
                    .map(Some);
                sender.send(("publish", published)).unwrap();
            }));
        }
        for _ in 0..2 {
            let store = store.clone();
            let manifest = manifest.clone();
            let sender = sender.clone();
            handles.push(std::thread::spawn(move || {
                sender
                    .send(("bootstrap", store.ensure_bootstrap(&manifest).map(Some)))
                    .unwrap();
            }));
        }
        drop(sender);
        for handle in handles {
            handle.join().expect("writer thread must not panic");
        }

        let mut save_wins = 0;
        let mut published: Option<AggregatePolicyArtifact> = None;
        for (kind, result) in receiver.into_iter() {
            match kind {
                "save" => match result {
                    Ok(_) => save_wins += 1,
                    Err(
                        ProductStoreError::InvalidRecord { .. }
                        | ProductStoreError::IdentityMismatch { .. },
                    ) => {}
                    Err(other) => panic!("save 只允许合法 successor 或冲突:{other}"),
                },
                "publish" => {
                    let artifact = result
                        .expect("同 operation 并发发布必须重入复用成功")
                        .expect("publish 结果必须携带 artifact");
                    if let Some(previous) = &published {
                        assert_eq!(previous, &artifact, "不得产生重复候选");
                    } else {
                        published = Some(artifact);
                    }
                }
                "bootstrap" => {
                    result.expect("ensure_bootstrap 并发必须幂等成功");
                }
                _ => unreachable!(),
            }
        }
        assert!(save_wins <= 1, "revision 2 只能被一个 save 赢得");
        let current = store.get(PROJECT).unwrap().unwrap();
        assert!(current.revision == 2 || current.revision == 3);
        let output = store
            .get_recipe_policy_publication(PROJECT, "op_race")
            .unwrap()
            .expect("唯一发布输出");
        assert_eq!(output.artifact, current, "发布候选与 current 一致");

        // 不同 scope 不共锁:持 A scope 锁时 B scope 仍可完成写入。
        let store_b = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_other");
        let root_b = temp.path().join("root-b");
        std::fs::create_dir_all(&root_b).unwrap();
        let manifest_b = LogicalCodebaseManifest::new(PROJECT, root_b, vec![]);
        let lock_a = paths
            .logical_codebases_root(PROJECT)
            .join("lc_0001")
            .join(".aggregate-policy.lock");
        let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let blocker = std::thread::spawn(move || {
            crate::product::coding_attempt_store::locking::with_exact_exclusive_lock(
                &lock_a,
                || {
                    held_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok::<(), ProductStoreError>(())
                },
            )
            .expect("scope A lock holder must succeed");
        });
        held_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("scope A 锁必须被持有");
        let (b_tx, b_rx) = std::sync::mpsc::channel();
        let ensure_b = std::thread::spawn(move || {
            let artifact = store_b.ensure_bootstrap(&manifest_b).unwrap();
            b_tx.send(artifact).unwrap();
        });
        let b_artifact = b_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("不同 scope 不得共享锁");
        ensure_b.join().unwrap();
        release_tx.send(()).unwrap();
        blocker.join().unwrap();
        assert_eq!(b_artifact.revision, 1);
    }

    /// 桩 digest 常量与精确谓词稳定:常量等于桩原字节独立 SHA-256;
    /// bootstrap 命中;关键词/换行变体/真正文 revision 1 不命中;不随 revision 漂移。
    #[test]
    fn recipe_policy_bootstrap_digest_and_exact_predicate_are_stable() {
        assert_eq!(
            BOOTSTRAP_POLICY_DIGEST,
            sha256_hex_of(BOOTSTRAP_POLICY_TEXT.as_bytes())
        );

        let placeholder =
            AggregatePolicyArtifact::bootstrap("p1", "l1", "2026-10-03T00:00:00Z".into());
        assert!(placeholder.is_bootstrap_placeholder());

        let keyword_only = placeholder.with_revised_policy(
            "Allow planning read-only and coding target-write sessions under the logical codebase.\n",
            "t",
        );
        assert!(!keyword_only.is_bootstrap_placeholder());

        let crlf_variant =
            placeholder.with_revised_policy(BOOTSTRAP_POLICY_TEXT.replace('\n', "\r\n"), "t");
        assert!(!crlf_variant.is_bootstrap_placeholder());

        let real_text = "# 聚合政策\n\n## 来源：AGENTS.md\n\n真实根规则\n\n";
        let real = AggregatePolicyArtifact {
            policy_id: "policy/p1/l1/1".to_string(),
            project_id: "p1".to_string(),
            logical_codebase_id: "l1".to_string(),
            revision: 1,
            digest: sha256_hex_of(real_text.as_bytes()),
            policy_text: real_text.to_string(),
            created_at: "t".to_string(),
        };
        assert!(!real.is_bootstrap_placeholder());

        // 桩判定不比较 revision/年龄:同一桩正文在任何 revision 都命中。
        let mut at_five = placeholder;
        at_five.revision = 5;
        at_five.policy_id = "policy/p1/l1/5".to_string();
        assert!(at_five.is_bootstrap_placeholder());
    }
}
