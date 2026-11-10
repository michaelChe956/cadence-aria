// 夹具种子助手：经 `include!` 进入 `knowledge_base_http.rs` 的同一模块
//（REQ-KB-13 前置门改造后自主文件拆出，守住单文件 1200 行门禁）。

/// 与 `manifest.rs` 测试同款的合法 complete manifest（api/pages 不适用）。
const COMPLETE_MANIFEST: &str = "\
schema_version: \"4.0\"
generated_at: \"2026-11-09T00:00:00Z\"
coverage:
  initialization:
    status: complete
    global_validation: passed
    completed_stages:
      - base-info
      - overview
      - global-validation
    skipped_stages:
      - stage: api
        reason: 接口领域不适用
      - stage: pages
        reason: 页面领域不适用
    completed_at: \"2026-11-09T00:05:00Z\"
scope:
  configurations:
    status: 全量
  api:
    status: 不适用
  pages:
    status: 不适用
update:
  last_change_package: {}
  processed_packages: []
";

impl KnowledgeBaseFixture {
    /// 预检① / REQ-KB-13 前置门：种齐冷启动投影 `rules_policy` 步 Completed
    /// 的全部 durable 事实——真实（非自举桩）政策 artifact、五步全 Completed
    /// 的根配方 operation、AGENTS.md 根规则与四命令审计全 Allowed 的最终
    /// receipt（与 lib 侧 `preflight.rs` 单测夹具同款三源种子；旧的「两文件」
    /// 种子在新口径下不再构成已完成）。
    fn seed_aggregate_initialization(&self) {
        let policy_text = "\
# Aggregate policy (root recipe)

Root-recipe aggregated policy for planning read-only and coding target-write sessions.
";
        // 政策 artifact 的身份字段必须与注册流写下的 LC manifest 一致：
        // it_web 走 HTTP 注册，manifest 的 logical_codebase_id 是 UUID 形态
        //（路由 lc_id 是 logical_codebase_* 形态）；resolver 会对
        // artifact ↔ manifest 做一致性校验，不一致 ⇒ 投影 fail-closed。
        let manifest = LogicalCodebaseStore::for_lc(self.paths.clone(), self.lc_id.clone())
            .load_lc_manifest(PROJECT_ID, &self.lc_id)
            .expect("load lc manifest")
            .expect("manifest present");
        let manifest_lc_id = manifest.logical_codebase_id.to_string();
        // sha256(policy_text) 的十六进制（与 preflight.rs 单测夹具同文本同摘要）。
        let policy = AggregatePolicyArtifact {
            policy_id: format!("policy/{PROJECT_ID}/{manifest_lc_id}/1"),
            project_id: PROJECT_ID.to_string(),
            logical_codebase_id: manifest_lc_id,
            revision: 1,
            digest: "sha256:e8f94ab7c62c203488cf1b8521ecd3b8f144250517a184698b16ed7c7817bdc1"
                .to_string(),
            policy_text: policy_text.to_string(),
            created_at: "2026-11-10T00:00:00Z".to_string(),
        };
        AggregatePolicyArtifactStore::for_lc(self.paths.clone(), self.lc_id.clone())
            .save(PROJECT_ID, &policy)
            .expect("save policy artifact");

        let operation_id = "aggregate_initialization_itweb_0001".to_string();
        let init_store = AggregateInitializationOperationStore::for_lc(
            self.paths.clone(),
            self.lc_id.clone(),
        );
        init_store
            .create_idempotent(AggregateInitializationOperation::new(
                operation_id.clone(),
                PROJECT_ID.to_string(),
                AggregateInitializationOperationInput {
                    idempotency_key: format!("itweb-{operation_id}"),
                    manifest_revision: 1,
                    policy_digest: policy.digest.clone(),
                    profile_evidence_digest: Some("sha256:profile".to_string()),
                    provider_context_root: self.aggregate_root.clone(),
                    provider: "claude_code".to_string(),
                },
                "2026-11-10T00:01:00Z".to_string(),
            ))
            .expect("create init operation");
        init_store
            .mark_running(PROJECT_ID, &operation_id, "2026-11-10T00:02:00Z".to_string())
            .expect("mark operation running");
        for step in AggregateInitializationStepKind::V1 {
            init_store
                .mark_step_running(
                    PROJECT_ID,
                    &operation_id,
                    step,
                    format!("itweb:{operation_id}:{}", step.as_str()),
                    "2026-11-10T00:03:00Z".to_string(),
                )
                .expect("mark step running");
            init_store
                .checkpoint_step_output(
                    PROJECT_ID,
                    &operation_id,
                    step,
                    format!("aggregate-initializations/op/{}.json", step.as_str()),
                    "2026-11-10T00:04:00Z".to_string(),
                )
                .expect("checkpoint step output");
            init_store
                .mark_step_completed(
                    PROJECT_ID,
                    &operation_id,
                    step,
                    "2026-11-10T00:05:00Z".to_string(),
                )
                .expect("mark step completed");
        }
        init_store
            .finish_completed(
                PROJECT_ID,
                &operation_id,
                "2026-11-10T00:06:00Z".to_string(),
            )
            .expect("finish operation completed");

        std::fs::write(
            self.aggregate_root.join(ROOT_RULE_ENTRY_FILE),
            "# aggregate root rules\n",
        )
        .expect("write root rule entry");
        let canonical_root =
            std::fs::canonicalize(&self.aggregate_root).expect("canonicalize aggregate root");
        let rule_digest = root_rule_digest(&canonical_root)
            .expect("read root rule digest")
            .expect("root rule entry present");
        let receipt_store = RootRecipeReceiptStore::for_lc(self.paths.clone(), self.lc_id.clone());
        let auditor = RootRecipeFilesystemAuditor::new();
        for (index, (step, command_index, command)) in
            root_recipe_command_index().into_iter().enumerate()
        {
            let watch = auditor
                .before_command(&operation_id, &canonical_root, step, command_index, command)
                .expect("freeze command watch");
            let receipt = auditor
                .after_command(watch, format!("2026-11-10T00:10:{index:02}Z"))
                .expect("audit command");
            assert_eq!(receipt.verdict, RootRecipeCommandVerdict::Allowed);
            receipt_store
                .append_command(PROJECT_ID, receipt)
                .expect("append command receipt");
        }
        receipt_store
            .finalize(
                PROJECT_ID,
                &operation_id,
                &policy.digest,
                &rule_digest,
                "2026-11-10T00:20:00Z".to_string(),
            )
            .expect("finalize root receipt");
    }


    /// 预检④⑤：合法 complete 的 manifest.yaml。
    fn seed_complete_manifest(&self) {
        std::fs::create_dir_all(self.kb_root()).expect("create kb root");
        std::fs::write(self.kb_root().join("manifest.yaml"), COMPLETE_MANIFEST)
            .expect("write manifest");
    }

    /// 空变更包（缺全部五份文档 ⇒ missing_document 违规）。
    fn seed_empty_change_package(&self, name: &str) {
        let dir = self.kb_root().join("user-input").join("updates").join(name);
        std::fs::create_dir_all(&dir).expect("create change package dir");
    }

    /// 直接经 durable store 造一条 Running operation（文件面运行期禁写的种子）。
    fn seed_running_operation(&self) {
        let now = "2026-11-09T00:00:00Z".to_string();
        // store 校验 operation_id 必须由 (project, lc, idempotency_key) 派生。
        let operation_id =
            cadence_aria::product::logical_codebase::knowledge_base::operation::derive_operation_id(
                PROJECT_ID,
                &self.lc_id,
                "seed",
            );
        let operation = KnowledgeBaseOperation {
            operation_id,
            project_id: PROJECT_ID.to_string(),
            logical_codebase_id: self.lc_id.clone(),
            kind: KnowledgeBaseOperationKind::Bootstrap,
            input: KnowledgeBaseOperationInput {
                idempotency_key: "seed".to_string(),
                provider: "claude_code".to_string(),
                package_path: None,
                package_id: None,
            },
            status: KnowledgeBaseOperationStatus::Running,
            steps: vec![KnowledgeBaseStepRecord {
                step_id: KnowledgeBaseStepKind::Bootstrap,
                status: KnowledgeBaseStepStatus::Running,
                started_at: Some(now.clone()),
                completed_at: None,
                output_summary: None,
            }],
            provider_selection: None,
            error: None,
            created_at: now.clone(),
            started_at: Some(now),
            completed_at: None,
        };
        KnowledgeBaseOperationStore::for_lc(self.paths.clone(), self.lc_id.clone())
            .create(operation)
            .expect("seed running operation");
    }

    /// 9 份输入全部写入与模板逐字不同的已填内容（filled ⇒ 已准备）。
    fn seed_filled_user_inputs(&self) {
        let user_input = self.kb_root().join("user-input");
        std::fs::create_dir_all(&user_input).expect("create user-input");
        for (index, name) in USER_INPUT_FILES.iter().enumerate() {
            std::fs::write(
                user_input.join(name),
                format!("# 已填写 {index}\n这是与模板逐字不同的真实内容。\n"),
            )
            .expect("write filled input");
        }
    }

    /// 平台侧确认记录文件路径（per-LC `.aria`，绝不落 user-input/）。
    fn confirmations_path(&self) -> PathBuf {
        self.paths
            .logical_codebases_root(PROJECT_ID)
            .join(&self.lc_id)
            .join("knowledge-base")
            .join("user-input-confirmations.json")
    }

    /// 经真实客户端流程批量确认全部 9 份：逐份 GET /file 取服务端摘要 →
    /// POST /user-input/confirm { all: true, items }。
    async fn confirm_all_user_inputs(&self) {
        let mut items = Vec::new();
        for name in USER_INPUT_FILES {
            let path = format!("user-input/{name}");
            let (status, body) = request_json(
                &self.app,
                Method::GET,
                &self.kb_uri(&format!("/file?path={path}")),
                json!({}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let digest = body["digest"].as_str().expect("digest").to_string();
            items.push(json!({ "path": path, "digest": digest }));
        }
        let (status, body) = request_json(
            &self.app,
            Method::POST,
            &self.kb_uri("/user-input/confirm"),
            json!({ "all": true, "items": items }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["confirmed"].as_array().expect("confirmed").len(), 9);
    }

    /// 有界等待 spawn 的 execute 收口（operation 进入终态）后返回终态名；
    /// 10 秒内未收口即失败（避免后台任务在测试结束后拖尾持有资源）。
    async fn wait_operation_settled(&self, operation_id: &str) -> String {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let (_, snapshot) = request_json(
                    &self.app,
                    Method::GET,
                    &self.kb_uri(&format!("/initializations/{operation_id}")),
                    json!({}),
                )
                .await;
                match snapshot["status"].as_str() {
                    Some("completed") | Some("failed") | Some("cancelled") => {
                        return snapshot["status"].as_str().expect("status").to_string();
                    }
                    _ => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
                }
            }
        })
        .await
        .expect("knowledge base operation must settle within 10s")
    }
}
