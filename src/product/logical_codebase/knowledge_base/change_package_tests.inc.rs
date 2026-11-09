// 变更包门禁的 tests 模块（由 change_package.rs 末尾 `include!` 进来，
// 与主文件同模块命名空间；独立成文件以守住 large_file_guard 的 1200 行
// 上限，参照 aggregate_initialization_store_tests.inc.rs 先例）。

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    use super::super::manifest::KnowledgeBaseManifestSnapshot;

    /// 拼出完整稳定码。
    fn code(suffix: &str) -> String {
        format!("{CODE_PREFIX}{suffix}")
    }

    fn has_code(lint: &ChangePackageLint, code: &str) -> bool {
        lint.violations
            .iter()
            .any(|violation| violation.code == code)
    }

    fn first_with_code<'a>(lint: &'a ChangePackageLint, code: &str) -> &'a ChangePackageViolation {
        lint.violations
            .iter()
            .find(|violation| violation.code == code)
            .expect("预期存在该稳定码的违规")
    }

    /// 在临时目录里建一个带初始提交的成员仓，返回（仓库根, HEAD 提交）。
    fn member_repo_with_commit(parent: &Path, name: &str) -> (PathBuf, String) {
        let repo = parent.join(name);
        std::fs::create_dir_all(&repo).expect("创建成员仓目录");
        let git = |args: &[&str]| -> String {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .expect("git 可执行");
            assert!(
                output.status.success(),
                "git {args:?} 失败：{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "kb@example.test"]);
        git(&["config", "user.name", "kb-test"]);
        git(&["commit", "--allow-empty", "-q", "-m", "init"]);
        let sha = git(&["rev-parse", "HEAD"]);
        (repo, sha)
    }

    /// change-summary.md 合法样例（模板结构逐字来自插件模板，值已填齐）。
    fn valid_change_summary() -> String {
        String::from(
            "# 变更摘要\n\
             \n\
             > 所有字段必须填写且不得留空，非适用字段填写 `不适用（具体原因）`。领域变更矩阵中的状态只允许填写 `有变更` 或 `无变更`。不得填写真实配置值、密码、Token、AccessKey、Secret、密钥、私钥、完整连接串、未脱敏内部地址或敏感值哈希；发现这些内容时整个变更包在计算幂等标识前停止。\n\
             \n\
             ## 基本信息\n\
             \n\
             - 变更标识：CHANGE-20261109-0001\n\
             - 变更目的：为订单导出增加流式分页\n\
             - 目标环境：生产\n\
             - 涉及服务：order-api\n\
             - 业务影响：导出接口新增分页参数\n\
             - 风险：低\n\
             \n\
             ## 领域变更矩阵\n\
             \n\
             | 领域 | 变更状态 | 摘要或无变更判断依据 |\n\
             |------|----------|----------------------|\n\
             | 代码 | 有变更 | 订单导出分页改造 |\n\
             | 数据模型 | 无变更 | 未新增表与字段，见数据库文档 |\n\
             | 配置 | 无变更 | 本次未调整配置，见配置文档 |\n\
             | 中间件 | 无变更 | 未引入新中间件 |\n\
             | 接口 | 有变更 | 导出接口新增分页参数 |\n\
             | 页面 | 无变更 | 无前端改动 |\n\
             | 业务知识 | 有变更 | 补充分页导出说明 |\n\
             \n\
             ## 关联说明\n\
             \n\
             - 需求、缺陷或发布单：REQ-1001\n\
             - 负责人或确认人：张三\n\
             - 补充说明：无\n",
        )
    }

    /// code-change.md 合法样例（起止提交由用例注入真实可解析提交）。
    fn valid_code_change(start: &str, end: &str) -> String {
        format!(
            "# 代码变更\n\
             \n\
             > 所有字段必须填写且不得留空。非适用字段填写 `不适用（具体原因）`；无代码变更时必须提供可验证的判断依据。不得粘贴真实配置值、凭证、完整连接串、未脱敏内部地址或敏感值哈希；发现这些内容时整个变更包在计算幂等标识前停止。\n\
             \n\
             - 变更状态：有变更\n\
             - Merge Request 地址或编号：!1234\n\
             - 源分支：feat/export-pagination\n\
             - 目标分支：main\n\
             - 起始提交：{start}\n\
             - 结束提交：{end}\n\
             - 修改工程：order-api\n\
             - 修改文件与符号：export.rs 的 stream_export\n\
             - 代码变更说明：分页游标改造\n\
             - 本地可验证范围：单测覆盖游标回退\n\
             - 无变更判断依据：不适用（本次存在代码变更）\n"
        )
    }

    /// database-change.md 合法样例（无数据模型变更分支）。
    fn valid_database_change() -> String {
        String::from(
            "# 数据库变更\n\
             \n\
             > 本文档始终强制提供，所有字段必须填写且不得留空。非适用字段填写 `不适用（具体原因）`；无数据模型变更时必须填写无变更判断依据。DDL 或迁移证据必须先脱敏，不得包含真实配置值、凭证、完整连接串、未脱敏内部地址、真实个人数据或敏感值哈希；发现这些内容时整个变更包在计算幂等标识前停止。\n\
             \n\
             - 变更状态：无变更\n\
             - 数据库 / Schema：不适用（本次无数据模型变更）\n\
             - 逻辑表：不适用（本次无数据模型变更）\n\
             - 字段变化：不适用（本次无数据模型变更）\n\
             - 索引变化：不适用（本次无数据模型变更）\n\
             - 约束变化：不适用（本次无数据模型变更）\n\
             - DDL 或迁移路径：不适用（本次无数据模型变更）\n\
             - 上线状态：不适用（本次无数据模型变更）\n\
             - 兼容性：不适用（本次无数据模型变更）\n\
             - 回滚方式：不适用（本次无数据模型变更）\n\
             - 无变更判断依据：未新增或修改任何表、字段、索引与约束\n",
        )
    }

    /// configuration-change.md 合法样例（Manifest 配置领域为「全量」分支）。
    fn valid_configuration_change() -> String {
        String::from(
            "# 配置变更\n\
             \n\
             > 本文档始终强制提供，所有字段必须填写且不得留空。Manifest 的 `scope.configurations.status` 为 `全量` 或 `指定` 时，基线和目标必须是同一环境的外部不可变快照，目录必须可读；无配置变更时仍需提供两份快照及判断依据。状态为 `不适用` 时，变更状态必须为 `无变更`，快照字段填写 `不适用（具体原因）`，并跳过快照比较。\n\
             >\n\
             > 配置差异明细只允许记录配置键、变更类型、用途和固定脱敏值 `<redacted>`。不得填写真实配置值、密码、Token、AccessKey、Secret、密钥、私钥、完整连接串、内部域名/IP/URL、原始配置片段或敏感值哈希；发现这些内容时整个变更包在计算幂等标识前停止。\n\
             \n\
             - Manifest 配置领域状态：全量\n\
             - 配置领域不适用原因：不适用（配置领域为全量纳管）\n\
             - 变更状态：无变更\n\
             - 环境（基线与目标必须相同）：生产\n\
             - 基线快照标识：snap-base-001\n\
             - 基线快照发布批次：batch-2026-11-08\n\
             - 基线快照生成或获取时间：2026-11-08T10:00:00Z\n\
             - 基线快照来源类型：外部发布系统\n\
             - 基线快照外部目录：/mnt/config-snapshots/base-001\n\
             - 基线快照指纹：digest-9f2c1f（脱敏占位）\n\
             - 基线范围摘要：全部业务配置\n\
             - 基线纳入文件数量或文件清单摘要：共 42 个配置文件\n\
             - 基线服务摘要：order-api 与调度服务\n\
             - 基线文件规则摘要：纳管 application 与 datasource 两类\n\
             - 目标快照标识：snap-target-001\n\
             - 目标快照发布批次：batch-2026-11-09\n\
             - 目标快照生成或获取时间：2026-11-09T10:00:00Z\n\
             - 目标快照来源类型：外部发布系统\n\
             - 目标快照外部目录：/mnt/config-snapshots/target-001\n\
             - 目标快照指纹：digest-1a7d22（脱敏占位）\n\
             - 目标范围摘要：全部业务配置\n\
             - 目标纳入文件数量或文件清单摘要：共 42 个配置文件\n\
             - 目标服务摘要：order-api 与调度服务\n\
             - 目标文件规则摘要：纳管 application 与 datasource 两类\n\
             - 纳入文件范围：全部纳管配置\n\
             - 涉及服务：order-api\n\
             - 配置组：默认配置组\n\
             - 已知差异：导出分页阈值待发布后核对\n\
             - 无变更判断依据：两份快照一致，本次发布未调整配置\n\
             \n\
             ## 配置差异明细\n\
             \n\
             | 配置键 | 变更类型 | 用途 | 脱敏值 |\n\
             |--------|----------|------|--------|\n\
             |  | 新增 / 删除 / 修改 / 移动 / 无变更 |  | `<redacted>` |\n",
        )
    }

    /// verification.md 合法样例。
    fn valid_verification() -> String {
        String::from(
            "# 验证记录\n\
             \n\
             > 所有字段必须填写且不得留空。未执行的验证必须明确填写 `未执行`，并在未验证项目和风险中说明影响。验证证据不得包含真实配置值、凭证、完整连接串、未脱敏内部地址、原始敏感配置或敏感值哈希；发现这些内容时整个变更包在计算幂等标识前停止。\n\
             \n\
             - 已执行测试：单测与集成测试全部通过\n\
             - 发布验证：预发环境已验证导出分页\n\
             - 数据兼容验证：不适用（本次无数据模型变更）\n\
             - 配置生效验证：不适用（本次未调整配置）\n\
             - 回滚方式：回滚发布批次并还原代码\n\
             - 未验证项目：生产灰度观察未完成\n\
             - 风险：灰度期间导出可能短暂限流\n",
        )
    }

    /// 把五份文档写进指定目录；overrides 以整份文档内容覆盖默认值。
    fn write_docs(dir: &Path, start: &str, end: &str, overrides: &[(&'static str, String)]) {
        let defaults: Vec<(&'static str, String)> = vec![
            ("change-summary.md", valid_change_summary()),
            ("code-change.md", valid_code_change(start, end)),
            ("database-change.md", valid_database_change()),
            ("configuration-change.md", valid_configuration_change()),
            ("verification.md", valid_verification()),
        ];
        for (document, default) in defaults {
            let content = overrides
                .iter()
                .find(|(name, _)| *name == document)
                .map(|(_, content)| content.clone())
                .unwrap_or(default);
            std::fs::write(dir.join(document), content).expect("写入变更包文档");
        }
    }

    /// 在 root 下写一份合法变更包（位于 user-input/updates/CHANGE- 目录内）。
    fn write_package(
        root: &Path,
        start: &str,
        end: &str,
        overrides: &[(&'static str, String)],
    ) -> PathBuf {
        let dir = root.join("user-input/updates/CHANGE-20261109-0001");
        std::fs::create_dir_all(&dir).expect("创建变更包目录");
        write_docs(&dir, start, end, overrides);
        dir
    }

    /// 只声明配置领域 scope 的最小 Manifest 快照。
    fn manifest_with_config_scope(status: &str) -> KnowledgeBaseManifestSnapshot {
        KnowledgeBaseManifestSnapshot {
            exists: true,
            schema_version: Some("4.0".to_string()),
            configurations_scope: Some(status.to_string()),
            ..KnowledgeBaseManifestSnapshot::default()
        }
    }

    fn members(repo: PathBuf) -> Vec<MemberRepoPaths> {
        vec![MemberRepoPaths { repo_root: repo }]
    }

    #[test]
    fn lint_accepts_fully_filled_change_package() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let dir = write_package(temp.path(), &sha, &sha, &[]);
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(
            lint.violations.is_empty(),
            "字段齐全的合法变更包不应有违规：{:?}",
            lint.violations
        );
    }

    #[test]
    fn lint_reports_field_status_for_editor() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let dir = write_package(temp.path(), &sha, &sha, &[]);
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        // 74 = 基本信息 9 + 领域矩阵 7 + 代码 11 + 数据库 11 + 配置 29 + 验证 7。
        assert_eq!(lint.fields.len(), 74, "字段状态应覆盖全部声明字段");
        assert!(
            lint.fields.iter().all(|status| status.filled),
            "合法包所有字段应为已填：{:?}",
            lint.fields
                .iter()
                .filter(|status| !status.filled)
                .collect::<Vec<_>>()
        );
        for document in [
            "change-summary.md",
            "code-change.md",
            "database-change.md",
            "configuration-change.md",
            "verification.md",
        ] {
            assert!(
                lint.fields.iter().any(|status| status.document == document),
                "缺少 {document} 的字段状态"
            );
        }
        assert!(
            lint.fields
                .iter()
                .any(|status| status.document == "code-change.md"
                    && status.field == "Merge Request 地址或编号"
                    && status.filled)
        );
        assert!(
            lint.fields
                .iter()
                .any(|status| status.field == "领域变更矩阵/业务知识" && status.filled)
        );
    }

    #[test]
    fn lint_rejects_missing_document() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let dir = write_package(temp.path(), &sha, &sha, &[]);
        std::fs::remove_file(dir.join("verification.md")).expect("删除验证文档");
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        let violation = first_with_code(&lint, &code("missing_document"));
        assert_eq!(violation.document.as_deref(), Some("verification.md"));
    }

    #[test]
    fn lint_rejects_empty_document() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let dir = write_package(temp.path(), &sha, &sha, &[]);
        std::fs::write(dir.join("verification.md"), "").expect("写入空文档");
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        let violation = first_with_code(&lint, &code("missing_document"));
        assert_eq!(violation.document.as_deref(), Some("verification.md"));
        assert!(
            violation.detail.contains("为空"),
            "空文档 detail 应说明为空"
        );
    }

    #[test]
    fn lint_rejects_absent_field_line() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let code_doc = valid_code_change(&sha, &sha)
            .lines()
            .filter(|line| !line.starts_with("- 源分支："))
            .collect::<Vec<_>>()
            .join("\n");
        let dir = write_package(temp.path(), &sha, &sha, &[("code-change.md", code_doc)]);
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        let violation = first_with_code(&lint, &code("field_empty"));
        assert_eq!(violation.document.as_deref(), Some("code-change.md"));
        assert_eq!(violation.field.as_deref(), Some("源分支"));
    }

    #[test]
    fn lint_tolerates_extra_whitespace_before_field_name() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let code_doc = valid_code_change(&sha, &sha).replace(
            "- 源分支：feat/export-pagination",
            "-   源分支：feat/export-pagination",
        );
        let dir = write_package(temp.path(), &sha, &sha, &[("code-change.md", code_doc)]);
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(
            !lint.violations.iter().any(|violation| violation.code == code("field_empty")
                && violation.field.as_deref() == Some("源分支")),
            "字段名前的多余空白不应把合法行判成模板行不存在：{:?}",
            lint.violations
        );
        assert!(
            lint.fields.iter().any(|status| status.document == "code-change.md"
                && status.field == "源分支"
                && status.filled),
            "双空格写法的源分支应被正常解析并标记为已填"
        );
    }

    #[test]
    fn lint_rejects_empty_required_field() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let code_doc = valid_code_change(&sha, &sha).replace(
            "- Merge Request 地址或编号：!1234",
            "- Merge Request 地址或编号：",
        );
        let dir = write_package(temp.path(), &sha, &sha, &[("code-change.md", code_doc)]);
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(
            lint.violations
                .iter()
                .any(|violation| violation.code == code("field_empty")
                    && violation.field.as_deref() == Some("Merge Request 地址或编号"))
        );
    }

    #[test]
    fn lint_rejects_unresolved_enum_placeholder() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let code_doc = valid_code_change(&sha, &sha)
            .replace("- 变更状态：有变更\n", "- 变更状态：有变更 / 无变更\n");
        let dir = write_package(temp.path(), &sha, &sha, &[("code-change.md", code_doc)]);
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(
            lint.violations
                .iter()
                .any(|violation| violation.code == code("field_empty")
                    && violation.document.as_deref() == Some("code-change.md")
                    && violation.field.as_deref() == Some("变更状态"))
        );
    }

    #[test]
    fn lint_rejects_unresolved_configuration_scope_placeholder() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let config_doc = valid_configuration_change().replace(
            "- Manifest 配置领域状态：全量",
            "- Manifest 配置领域状态：全量 / 指定 / 不适用",
        );
        let dir = write_package(
            temp.path(),
            &sha,
            &sha,
            &[("configuration-change.md", config_doc)],
        );
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(
            lint.violations
                .iter()
                .any(|violation| violation.code == code("field_empty")
                    && violation.field.as_deref() == Some("Manifest 配置领域状态"))
        );
    }

    #[test]
    fn lint_rejects_bare_not_applicable() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let db_doc = valid_database_change()
            .replace("- 逻辑表：不适用（本次无数据模型变更）", "- 逻辑表：不适用");
        let dir = write_package(temp.path(), &sha, &sha, &[("database-change.md", db_doc)]);
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(
            lint.violations
                .iter()
                .any(|violation| violation.code == code("reason_missing")
                    && violation.document.as_deref() == Some("database-change.md")
                    && violation.field.as_deref() == Some("逻辑表"))
        );
    }

    #[test]
    fn lint_rejects_placeholder_not_applicable_reason() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let db_doc = valid_database_change().replace(
            "- 逻辑表：不适用（本次无数据模型变更）",
            "- 逻辑表：不适用（具体原因）",
        );
        let dir = write_package(temp.path(), &sha, &sha, &[("database-change.md", db_doc)]);
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(
            lint.violations
                .iter()
                .any(|violation| violation.code == code("reason_missing")
                    && violation.field.as_deref() == Some("逻辑表"))
        );
    }

    #[test]
    fn lint_accepts_natural_language_not_applicable() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let db_doc = valid_database_change().replace(
            "- 逻辑表：不适用（本次无数据模型变更）",
            "- 逻辑表：不适用于存量数据",
        );
        let config_doc = valid_configuration_change().replace(
            "- 基线快照标识：snap-base-001",
            "- 基线快照标识：不适用：旧环境无配置快照",
        );
        let dir = write_package(
            temp.path(),
            &sha,
            &sha,
            &[
                ("database-change.md", db_doc),
                ("configuration-change.md", config_doc),
            ],
        );
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(
            !has_code(&lint, &code("reason_missing")),
            "「不适用于…」是自然语言写法，不应误报缺原因：{:?}",
            lint.violations
        );
        assert!(
            !lint
                .violations
                .iter()
                .any(|violation| violation.code == code("scope_mismatch")
                    && violation.field.as_deref() == Some("基线快照标识")),
            "「不适用：…」非模板形态，全量分支不得追加 scope_mismatch：{:?}",
            lint.violations
        );
    }

    #[test]
    fn lint_rejects_incomplete_domain_matrix() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let summary = valid_change_summary()
            .lines()
            .filter(|line| !line.starts_with("| 接口 |"))
            .collect::<Vec<_>>()
            .join("\n");
        let dir = write_package(temp.path(), &sha, &sha, &[("change-summary.md", summary)]);
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(
            lint.violations
                .iter()
                .any(|violation| violation.code == code("domain_matrix_invalid")
                    && violation.field.as_deref() == Some("领域变更矩阵/接口"))
        );
    }

    #[test]
    fn lint_rejects_domain_matrix_placeholder_status() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let summary = valid_change_summary().replace(
            "| 代码 | 有变更 | 订单导出分页改造 |",
            "| 代码 | 有变更 / 无变更 | 订单导出分页改造 |",
        );
        let dir = write_package(temp.path(), &sha, &sha, &[("change-summary.md", summary)]);
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(
            lint.violations
                .iter()
                .any(|violation| violation.code == code("domain_matrix_invalid")
                    && violation.field.as_deref() == Some("领域变更矩阵/代码"))
        );
    }

    #[test]
    fn lint_rejects_domain_matrix_without_basis() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let summary = valid_change_summary().replace(
            "| 数据模型 | 无变更 | 未新增表与字段，见数据库文档 |",
            "| 数据模型 | 无变更 |  |",
        );
        let dir = write_package(temp.path(), &sha, &sha, &[("change-summary.md", summary)]);
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(
            lint.violations
                .iter()
                .any(|violation| violation.code == code("domain_matrix_invalid")
                    && violation.field.as_deref() == Some("领域变更矩阵/数据模型"))
        );
    }

    #[test]
    fn lint_rejects_sensitive_content_without_echoing_value() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let code_doc = format!(
            "{}\npassword = \"s3cr3t-value\"\n",
            valid_code_change(&sha, &sha)
        );
        let dir = write_package(temp.path(), &sha, &sha, &[("code-change.md", code_doc)]);
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        let violation = first_with_code(&lint, &code("sensitive_content"));
        assert_eq!(violation.document.as_deref(), Some("code-change.md"));
        assert!(
            !violation.detail.contains("s3cr3t-value"),
            "detail 不得回显命中值：{}",
            violation.detail
        );
        assert!(violation.detail.contains("行"), "detail 应给出行号位置");
    }

    #[test]
    fn lint_rejects_sensitive_connection_string() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let db_doc = valid_database_change().replace(
            "- DDL 或迁移路径：不适用（本次无数据模型变更）",
            "- DDL 或迁移路径：jdbc:mysql://10.2.3.4:3306/order",
        );
        let dir = write_package(temp.path(), &sha, &sha, &[("database-change.md", db_doc)]);
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        let violation = first_with_code(&lint, &code("sensitive_content"));
        assert_eq!(violation.document.as_deref(), Some("database-change.md"));
        assert!(
            !violation.detail.contains("jdbc:mysql"),
            "detail 不回显原文"
        );
        assert!(
            !violation.detail.contains("10.2.3.4"),
            "detail 不回显内网地址"
        );
    }

    #[test]
    fn lint_rejects_sensitive_internal_url() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let config_doc = valid_configuration_change().replace(
            "- 已知差异：导出分页阈值待发布后核对",
            "- 已知差异：内网入口 http://192.168.1.10/api 已迁移",
        );
        let dir = write_package(
            temp.path(),
            &sha,
            &sha,
            &[("configuration-change.md", config_doc)],
        );
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        let violation = first_with_code(&lint, &code("sensitive_content"));
        assert_eq!(
            violation.document.as_deref(),
            Some("configuration-change.md")
        );
        assert!(
            !violation.detail.contains("192.168.1.10"),
            "detail 不回显内网地址"
        );
    }

    #[test]
    fn lint_scans_attachment_sensitive_content_without_echoing_value() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let dir = write_package(temp.path(), &sha, &sha, &[]);
        let attachments = dir.join("attachments");
        std::fs::create_dir_all(&attachments).expect("创建附件目录");
        std::fs::write(
            attachments.join("ddl.sql"),
            "-- 数据库差异导出\npassword = \"s3cr3t-value\"\n",
        )
        .expect("写附件");
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        let violation = first_with_code(&lint, &code("sensitive_content"));
        assert_eq!(
            violation.document.as_deref(),
            Some("attachments/ddl.sql"),
            "附件命中的 document 必须是 attachments/<相对路径>"
        );
        assert!(
            !violation.detail.contains("s3cr3t-value"),
            "detail 不得回显命中值：{}",
            violation.detail
        );
        assert!(violation.detail.contains("行"), "detail 应给出行号位置");
    }

    #[test]
    fn lint_accepts_clean_attachments_without_false_positives() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let dir = write_package(temp.path(), &sha, &sha, &[]);
        let attachments = dir.join("attachments");
        std::fs::create_dir_all(&attachments).expect("创建附件目录");
        std::fs::write(
            attachments.join("ddl.sql"),
            "-- 数据库差异导出\nALTER TABLE orders ADD COLUMN cursor_id BIGINT;\n",
        )
        .expect("写干净附件");
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(
            !has_code(&lint, &code("sensitive_content")),
            "干净附件不应产生敏感误报：{:?}",
            lint.violations
        );
    }

    #[test]
    fn lint_reports_unscannable_attachments_instead_of_skipping_silently() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let dir = write_package(temp.path(), &sha, &sha, &[]);
        let attachments = dir.join("attachments");
        std::fs::create_dir_all(&attachments).expect("创建附件目录");
        // 非 UTF-8 的常规文件：无法逐行扫描 ⇒ 记说明性违规（不静默跳过）。
        std::fs::write(attachments.join("dump.bin"), [0xFF, 0xFE, 0x00, 0x81])
            .expect("写二进制附件");
        // 超过 1 MiB 扫描上限的文本文件：同样记说明性违规。
        std::fs::write(attachments.join("huge.sql"), vec![b'x'; 1024 * 1024 + 1])
            .expect("写超限附件");
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        let unscannable: Vec<_> = lint
            .violations
            .iter()
            .filter(|violation| {
                violation.code == code("sensitive_content")
                    && violation
                        .document
                        .as_deref()
                        .is_some_and(|document| document.starts_with("attachments/"))
                    && violation.detail.contains("无法")
            })
            .collect();
        let documents: Vec<_> = unscannable
            .iter()
            .map(|violation| violation.document.as_deref())
            .collect();
        assert_eq!(
            documents,
            vec![Some("attachments/dump.bin"), Some("attachments/huge.sql")],
            "不可扫描的附件要各记一条说明性违规：{:?}",
            lint.violations
        );
        for violation in unscannable {
            assert!(
                !violation.detail.contains("xxx"),
                "detail 不得回显附件内容：{}",
                violation.detail
            );
        }
    }

    #[test]
    fn lint_rejects_unresolvable_commit() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, _sha) = member_repo_with_commit(temp.path(), "order-api");
        let dir = write_package(temp.path(), "deadbeefdeadbeef", "deadbeefdeadbeef", &[]);
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        let violation = first_with_code(&lint, &code("commit_unresolvable"));
        assert_eq!(violation.document.as_deref(), Some("code-change.md"));
        assert_eq!(violation.field.as_deref(), Some("起始提交"));
    }

    #[test]
    fn lint_skips_commit_resolution_when_no_code_change() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, _sha) = member_repo_with_commit(temp.path(), "order-api");
        let code_doc = valid_code_change("deadbeefdeadbeef", "deadbeefdeadbeef")
            .replace("- 变更状态：有变更", "- 变更状态：无变更")
            .replace(
                "- 无变更判断依据：不适用（本次存在代码变更）",
                "- 无变更判断依据：发布分支未包含代码提交",
            );
        let dir = write_package(
            temp.path(),
            "deadbeefdeadbeef",
            "deadbeefdeadbeef",
            &[("code-change.md", code_doc)],
        );
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(
            !has_code(&lint, &code("commit_unresolvable")),
            "无变更分支不应校验提交可解析：{:?}",
            lint.violations
        );
    }

    #[test]
    fn lint_accepts_commit_resolvable_in_any_member_repo() {
        let temp = tempfile::tempdir().unwrap();
        let (first, _first_sha) = member_repo_with_commit(temp.path(), "alpha");
        let (second, second_sha) = member_repo_with_commit(temp.path(), "beta");
        let dir = write_package(temp.path(), &second_sha, &second_sha, &[]);
        let members = vec![
            MemberRepoPaths { repo_root: first },
            MemberRepoPaths { repo_root: second },
        ];
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members);
        assert!(
            !has_code(&lint, &code("commit_unresolvable")),
            "任一成员仓可解析即应通过：{:?}",
            lint.violations
        );
    }

    #[test]
    fn lint_rejects_configuration_scope_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let config_doc = valid_configuration_change()
            .replace(
                "- Manifest 配置领域状态：全量",
                "- Manifest 配置领域状态：不适用",
            )
            .replace(
                "- 配置领域不适用原因：不适用（配置领域为全量纳管）",
                "- 配置领域不适用原因：配置中心未纳管",
            )
            .replace("- 变更状态：无变更\n", "- 变更状态：有变更\n");
        let dir = write_package(
            temp.path(),
            &sha,
            &sha,
            &[("configuration-change.md", config_doc)],
        );
        let lint = lint_change_package(&dir, &manifest_with_config_scope("不适用"), &members(repo));
        assert!(
            lint.violations
                .iter()
                .any(|violation| violation.code == code("scope_mismatch")
                    && violation.field.as_deref() == Some("变更状态"))
        );
    }

    #[test]
    fn lint_rejects_off_enum_status_under_not_applicable_scope() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let config_doc = valid_configuration_change()
            .replace(
                "- Manifest 配置领域状态：全量",
                "- Manifest 配置领域状态：不适用",
            )
            .replace("- 变更状态：无变更\n", "- 变更状态：不适用\n");
        let dir = write_package(
            temp.path(),
            &sha,
            &sha,
            &[("configuration-change.md", config_doc)],
        );
        let lint = lint_change_package(&dir, &manifest_with_config_scope("不适用"), &members(repo));
        assert!(
            lint.violations
                .iter()
                .any(|violation| violation.code == code("scope_mismatch")
                    && violation.field.as_deref() == Some("变更状态")),
            "不适用分支的变更状态填表外取值也应报 scope_mismatch：{:?}",
            lint.violations
        );
    }

    #[test]
    fn lint_rejects_declared_scope_differs_from_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let config_doc = valid_configuration_change().replace(
            "- Manifest 配置领域状态：全量",
            "- Manifest 配置领域状态：指定",
        );
        let dir = write_package(
            temp.path(),
            &sha,
            &sha,
            &[("configuration-change.md", config_doc)],
        );
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(
            lint.violations
                .iter()
                .any(|violation| violation.code == code("scope_mismatch")
                    && violation.field.as_deref() == Some("Manifest 配置领域状态"))
        );
    }

    #[test]
    fn lint_rejects_snapshot_not_applicable_under_full_scope() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let config_doc = valid_configuration_change().replace(
            "- 基线快照标识：snap-base-001",
            "- 基线快照标识：不适用（快照尚未生成）",
        );
        let dir = write_package(
            temp.path(),
            &sha,
            &sha,
            &[("configuration-change.md", config_doc)],
        );
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(
            lint.violations
                .iter()
                .any(|violation| violation.code == code("scope_mismatch")
                    && violation.field.as_deref() == Some("基线快照标识"))
        );
    }

    #[test]
    fn lint_rejects_invalid_package_path() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let dir = temp.path().join("user-input/updates/pkg-0001");
        std::fs::create_dir_all(&dir).expect("创建非法命名的包目录");
        write_docs(&dir, &sha, &sha, &[]);
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(has_code(&lint, &code("path_invalid")));
    }

    #[test]
    fn lint_rejects_package_path_outside_updates() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let dir = temp.path().join("downloads/CHANGE-20261109-0001");
        std::fs::create_dir_all(&dir).expect("创建 updates 之外的包目录");
        write_docs(&dir, &sha, &sha, &[]);
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(has_code(&lint, &code("path_invalid")));
    }

    #[test]
    fn lint_rejects_change_id_with_path_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let (repo, sha) = member_repo_with_commit(temp.path(), "order-api");
        let summary = valid_change_summary().replace(
            "- 变更标识：CHANGE-20261109-0001",
            "- 变更标识：CHANGE-../../etc",
        );
        let dir = write_package(temp.path(), &sha, &sha, &[("change-summary.md", summary)]);
        let lint = lint_change_package(&dir, &manifest_with_config_scope("全量"), &members(repo));
        assert!(
            lint.violations
                .iter()
                .any(|violation| violation.code == code("path_invalid")
                    && violation.field.as_deref() == Some("变更标识"))
        );
    }
}