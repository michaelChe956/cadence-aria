//! b45(T22) plan 段失败可诊断化。
//!
//! 真机现场(T22 r20/t25)plan 候选两次 `SINGLE_CANDIDATE_APPROVAL_COMPILE_FAILED`
//! (`cause=single candidate publish freshness failed: MechanicalValidationFailed`)
//! 后,机械报告随测试 tempdir 一起销毁,只余 `mechanical_report_ref`——违规项
//! 不可知。本模块在 plan 阶段收口后把 `.aria` 内本 plan 的 durable 产物
//! (机械报告/候选 IR/源修订/编译事务/draft 记录)整棵复制进证据根:
//!
//! - `<evidence_root>/plan-diagnostics/<issue 根下相对路径>`:原样副本;
//! - `<evidence_root>/plan_diagnostics.json`:摘要(findings 逐条 code/message/
//!   work_item_ids、编译事务 status/failure_reason/validator_findings、源修订
//!   长度、复制失败错误)。
//!
//! 另:`b45_record_http_failure` 把 coding-attempt 创建失败的 HTTP 响应体原样
//! 落盘(400 的 `work_item_plan_not_confirmed` 等分类码不再只埋在错误文案里)。
//!
//! 判据边界:本模块只读 `.aria` 副本、只写证据根;任何 IO/解析失败如实进摘要
//! `errors`,绝不 panic、绝不改变测试判据。产品代码零改动。

use serde_json::{Value, json};
use std::path::Path;

use super::harness::B45_MAVEN_IMAGE;
use super::harness::issue_scope_guards::ISSUE_AUTHOR_READONLY_CLAUSE;

/// 本 plan 在 issue 根下的 durable 目录(各自再挂 `<plan_id>`)。
const B45_PLAN_DIAGNOSTIC_ROOTS: [&str; 4] = [
    "work-item-plan-sources",
    "work_item_plan_compiles",
    "work_item_plan_drafts",
    "work_item_plan_outlines",
];

/// plan 阶段后:复制本 plan 的 durable 产物到证据根并返回摘要。
///
/// `aria_root` 即 `ProductAppPaths::root()`(`.aria` 目录);目标目录
/// `<evidence_root>/plan-diagnostics/` 与摘要 `<evidence_root>/plan_diagnostics.json`
/// 由本函数自建,不依赖调用侧先建目录。
pub(crate) fn b45_capture_plan_diagnostics(
    aria_root: &Path,
    scope: (&str, &str, &str),
    evidence_root: &Path,
) -> Value {
    let (project_id, issue_id, plan_id) = scope;
    let issue_root = aria_root
        .join("projects")
        .join(project_id)
        .join("issues")
        .join(issue_id);
    let diagnostics_dir = evidence_root.join("plan-diagnostics");
    let mut summary = json!({
        "aria_root": aria_root.display().to_string(),
        "issue_root": issue_root.display().to_string(),
        "plan_id": plan_id,
        "diagnostics_dir": diagnostics_dir.display().to_string(),
        "summary_path": evidence_root.join("plan_diagnostics.json").display().to_string(),
    });
    let mut errors: Vec<String> = Vec::new();
    let mut missing_roots: Vec<String> = Vec::new();
    let mut files: Vec<Value> = Vec::new();
    let mut mechanical_reports: Vec<Value> = Vec::new();
    let mut compile_transactions: Vec<Value> = Vec::new();
    let mut source_revisions: Vec<Value> = Vec::new();
    let mut plan_candidate_irs: Vec<Value> = Vec::new();
    let mut drafts: Vec<Value> = Vec::new();

    if plan_id.is_empty() {
        errors.push("plan_id 为空(prepare 未返回 durable plan id),无 plan 产物可复制".to_string());
    } else if !issue_root.is_dir() {
        errors.push(format!(
            "issue 根不存在:{}(.aria 副本已被销毁或 project/issue 口径漂移)",
            issue_root.display()
        ));
    } else {
        for root in B45_PLAN_DIAGNOSTIC_ROOTS {
            let source_root = issue_root.join(root).join(plan_id);
            if !source_root.is_dir() {
                missing_roots.push(root.to_string());
                continue;
            }
            let mut stack = vec![source_root];
            while let Some(directory) = stack.pop() {
                let entries = match std::fs::read_dir(&directory) {
                    Ok(entries) => entries,
                    Err(error) => {
                        errors.push(format!("读目录 {} 失败:{error}", directory.display()));
                        continue;
                    }
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                        continue;
                    }
                    if !path.is_file() {
                        continue;
                    }
                    // 产品文件锁(`.<id>.json.lock`)是空占位,不属证据。
                    if path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with('.'))
                    {
                        continue;
                    }
                    let Ok(relative) = path.strip_prefix(&issue_root) else {
                        errors.push(format!("路径不在 issue 根下:{}", path.display()));
                        continue;
                    };
                    let relative = relative.display().to_string();
                    let target = diagnostics_dir.join(&relative);
                    let copied = match target.parent() {
                        Some(parent) => std::fs::create_dir_all(parent)
                            .and_then(|()| std::fs::copy(&path, &target)),
                        None => std::fs::copy(&path, &target),
                    };
                    match copied {
                        Ok(bytes) => {
                            files.push(json!({"source": relative, "bytes": bytes}));
                            b45_summarize_plan_record(
                                &relative,
                                &target,
                                &mut mechanical_reports,
                                &mut compile_transactions,
                                &mut source_revisions,
                                &mut plan_candidate_irs,
                                &mut drafts,
                                &mut errors,
                            );
                        }
                        Err(error) => errors.push(format!("复制 {} 失败:{error}", path.display())),
                    }
                }
            }
        }
    }

    summary["missing_roots"] = json!(missing_roots);
    summary["files"] = json!(files);
    summary["mechanical_reports"] = json!(mechanical_reports);
    summary["compile_transactions"] = json!(compile_transactions);
    summary["source_revisions"] = json!(source_revisions);
    summary["plan_candidate_irs"] = json!(plan_candidate_irs);
    summary["drafts"] = json!(drafts);
    if let Err(error) = b45_write_json(&evidence_root.join("plan_diagnostics.json"), &summary) {
        errors.push(error);
    }
    summary["errors"] = json!(errors);
    summary
}

/// 按目录族把 durable 记录压成摘要(解析失败只记 error,不中断复制)。
#[allow(clippy::too_many_arguments)]
fn b45_summarize_plan_record(
    relative: &str,
    target: &Path,
    mechanical_reports: &mut Vec<Value>,
    compile_transactions: &mut Vec<Value>,
    source_revisions: &mut Vec<Value>,
    plan_candidate_irs: &mut Vec<Value>,
    drafts: &mut Vec<Value>,
    errors: &mut Vec<String>,
) {
    if !relative.ends_with(".json") {
        return;
    }
    let record = match b45_read_json(target) {
        Ok(record) => record,
        Err(error) => {
            errors.push(format!("解析 {relative} 失败:{error}"));
            return;
        }
    };
    if relative.contains("/mechanical_report/") {
        let findings = record
            .pointer("/report/findings")
            .cloned()
            .unwrap_or_else(|| json!([]));
        mechanical_reports.push(json!({
            "file": relative,
            "id": record.get("id").cloned().unwrap_or(Value::Null),
            "ir_id": record.get("ir_id").cloned().unwrap_or(Value::Null),
            "source_revision_id": record.get("source_revision_id").cloned().unwrap_or(Value::Null),
            "findings": findings,
        }));
    } else if relative.contains("work_item_plan_compiles/") {
        compile_transactions.push(json!({
            "file": relative,
            "compile_id": record.get("compile_id").cloned().unwrap_or(Value::Null),
            "status": record.get("status").cloned().unwrap_or(Value::Null),
            "failure_reason": record.get("failure_reason").cloned().unwrap_or(Value::Null),
            "mechanical_report_ref": record.get("mechanical_report_ref").cloned().unwrap_or(Value::Null),
            "plan_candidate_ir_ref": record.get("plan_candidate_ir_ref").cloned().unwrap_or(Value::Null),
            "source_revision_ref": record.get("source_revision_ref").cloned().unwrap_or(Value::Null),
            "validator_findings": record.get("validator_findings").cloned().unwrap_or_else(|| json!([])),
        }));
    } else if relative.contains("work_item_plan_drafts/") {
        drafts.push(json!({
            "file": relative,
            "draft_id": record.get("draft_id").cloned().unwrap_or(Value::Null),
            "generation_round_id": record.get("generation_round_id").cloned().unwrap_or(Value::Null),
            "status": record.get("status").cloned().unwrap_or(Value::Null),
            "active": record.get("active").cloned().unwrap_or(Value::Null),
            "supersede_reason": record.get("supersede_reason").cloned().unwrap_or(Value::Null),
        }));
    } else if relative.contains("/source_revision/") {
        source_revisions.push(json!({
            "file": relative,
            "id": record.get("id").cloned().unwrap_or(Value::Null),
            "source_chars": record
                .get("source")
                .and_then(Value::as_str)
                .map(|source| source.chars().count())
                .unwrap_or_default(),
        }));
    } else if relative.contains("/plan_candidate_ir/") {
        plan_candidate_irs.push(json!({
            "file": relative,
            "id": record.get("id").cloned().unwrap_or(Value::Null),
            "compiler_version": record.pointer("/ir/compiler_version").cloned().unwrap_or(Value::Null),
            "item_count": record
                .pointer("/ir/items")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or_default(),
        }));
    }
}

/// coding-attempt 创建失败的 HTTP 响应体原样落盘(状态码 + body)。
///
/// `label` 区分调用点(主 coding 格 / 取消格),文件名
/// `coding-attempt-<label>.json`;写失败静默忽略(证据面不改变测试判据)。
pub(crate) fn b45_record_http_failure(
    evidence_root: &Path,
    label: &str,
    status: u16,
    body: &Value,
) {
    let payload = json!({
        "label": label,
        "status": status,
        "body": body,
    });
    let _ = b45_write_json(
        &evidence_root.join(format!("coding-attempt-{label}.json")),
        &payload,
    );
}

/// 成员仓 git 快照(HEAD+porcelain;前后对照证明只清登记资源/工作保留)。
pub(crate) fn b45_member_git_snapshot(member_worktree: &Path) -> Value {
    let git = |args: &[&str]| -> String {
        std::process::Command::new("git")
            .args(["-C", &member_worktree.display().to_string()])
            .args(args)
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .unwrap_or_default()
    };
    json!({
        "head": git(&["rev-parse", "HEAD"]),
        "porcelain": git(&["status", "--porcelain"]),
    })
}

fn b45_read_json(path: &Path) -> Result<Value, String> {
    let content =
        std::fs::read_to_string(path).map_err(|error| format!("{}:{error}", path.display()))?;
    serde_json::from_str(&content).map_err(|error| format!("{}:{error}", path.display()))
}

fn b45_write_json(target: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("创建 {} 失败:{error}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    std::fs::write(target, bytes).map_err(|error| format!("写 {} 失败:{error}", target.display()))
}

/// b45 钉定 issue 描述:验证命令逐字注入(AI 计划链的合同面)。
///
/// T22 终版失败现场:`SINGLE_CANDIDATE_APPROVAL_COMPILE_FAILED`
/// (`MechanicalValidationFailed`),机械报告 findings =
/// `acceptance_path_not_in_baseline`(work item WI-001 引用路径 `[.]`)+
/// `integration_or_e2e_skipped_risk`(warning)。根因=podman-build 的构建
/// 上下文写成 `.`,ac_paths 提取器把 `.` 当仓库相对路径、基线树无该条目。
/// 故本描述前置两段硬约束(T24 先例:先给 AC 约束+14 节骨架,再给业务):
/// 路径 token 不得为 `.`/根路径,验证命令里的构建上下文写显式基线路径;
/// Work Item Plan 逐字使用编译器 grammar 的 14 个结构化节与必需 key。
pub(crate) fn b45_kimi_issue_description() -> String {
    format!(
        "## 验收标准(AC)硬约束(先读——AC/验证路径违反即机械校验红,plan 编译直接失败)\n\
- 验收标准与验证计划的路径字段只能引用 plan 基线树内【已提交】的既有路径,一律写成 `fixtures/` 开头的仓库内相对路径;\n\
- 禁止把 `.`、`./`、仓库根、绝对路径、`$PWD`、通配符当作 AC statement 或验证命令里的路径 token(基线树没有根条目→`acceptance_path_not_in_baseline` Error);容器构建上下文必须写显式基线路径(如 `fixtures/podman-fixture`),不得写 `.`;\n\
- coder 交付后才新建的文件(如 `CHECK-EVIDENCE.log`)不得写进 AC 或验证路径(基线外→Error);\n\
- AC 一律 EARS 模板(`WHEN <条件> THE SYSTEM SHALL <可观察结果>`),每条单谓语;`required_evidence` 只用白名单值(`source_diff`/`non_zero_test_execution`/`manual_check`/`handoff_field`)。\n\
\n\
## Work Item Plan 结构骨架(plan 编译器硬合同——节标题逐字使用,不得自创/翻译/增删)\n\
文档头必须精确为 `# Work Item Plan`(前面不得有前言/解释/空行/代码围栏);Work Item 头必须为 `## Work Item WI-001`。下列 14 个结构化节(编译器白名单全集)按序各出现一次,节内用 `- key: value` 行:\n\
`Identity`(schema_version/logical_work_item_id/title/kind)、`Goal`(summary)、`Non Goals`(non_goals)、`Dependencies`(depends_on)、`Plan Intent`(intent/provider_work_item_id/intent_target_kind/intent_target_repo;意图节按需,声明时 intent 只能 `create`/`existing`)、`Inputs`、`Outputs`(contract_id/capabilities)、`Tasks`(task_id/statement/requirement_refs/done_when_refs)、`Write Policy`(exclusive_scopes/forbidden_scopes)、`Acceptance Criteria`(criterion_id/statement/required_evidence)、`Verification`(check_id/required/non_zero_test_execution_required/command)、`Handoff Schema`(required_fields/provided_contract_refs/reviewer_check_refs)、`Blockers`(允许零字段=无阻塞)、`Traceability`(source_type/source_id/requirement_id)。\n\
禁止:白名单外节标题、自造结构化 key、节外游离的非空内容;缺任一必需节或 key=编译拒绝(`unknown_structured_key`/`missing_section`)。同一 contract 的重复 `capabilities`/`requirement_refs` 按出现序合并,不要重复堆叠同一行。\n\
骨架示例(照此填值;`<...>` 为待填内容,不是占位字面量):\n\
```markdown\n\
# Work Item Plan\n\
## Work Item WI-001\n\
### Identity\n\
- schema_version: 1\n\
- logical_work_item_id: WI-001\n\
- title: <标题>\n\
- kind: integration\n\
### Goal\n\
- summary: <一句话目标>\n\
### Non Goals\n\
- non_goals: <非目标>\n\
### Dependencies\n\
- depends_on: []\n\
### Plan Intent\n\
- intent: create\n\
- provider_work_item_id: WI-001\n\
- intent_target_kind: single_repository\n\
- intent_target_repo: <成员物理仓 id>\n\
### Inputs\n\
### Outputs\n\
- contract_id: CT-001\n\
- capabilities: <能力>\n\
### Tasks\n\
- task_id: TASK-001\n\
- statement: WHEN <条件> THE SYSTEM SHALL <可观察结果>\n\
- requirement_refs: REQ-001\n\
- done_when_refs: AC-001\n\
### Write Policy\n\
- exclusive_scopes: lib.rs\n\
- forbidden_scopes: .claude/\n\
### Acceptance Criteria\n\
- criterion_id: AC-001\n\
- statement: WHEN <条件> THE SYSTEM SHALL <可观察结果>\n\
- required_evidence: non_zero_test_execution\n\
### Verification\n\
- check_id: CHECK-001\n\
- required: true\n\
- non_zero_test_execution_required: true\n\
- command: cd fixtures/npm-fixture && npm test\n\
### Handoff Schema\n\
- required_fields: implementation_summary\n\
- provided_contract_refs: []\n\
- reviewer_check_refs: AC-001\n\
### Blockers\n\
### Traceability\n\
- source_type: issue\n\
- source_id: issue_0001\n\
- requirement_id: REQ-001\n\
```\n\
\n\
本需求:在 alpha 仓新增 fixtures/ 三套宿主验证样例并完成 CHECK。本需求不修改 beta 仓。\
代码改动很小:在 alpha 仓 src/lib.rs 增加 pub fn host_check_marker() -> &'static str {{ \
return \"b45-host-check\"; }}。工作项验证命令(verification commands)是本需求主体,\
必须逐字包含以下六条命令(cwd 为仓库内相对路径):\
(1) maven-container-check: cd fixtures/maven-fixture && podman run --rm \
-v /tmp/aria-bug45/t22/m2-repo:/root/.m2 -v \"$PWD\":/workspace -w /workspace \
{B45_MAVEN_IMAGE} mvn -B test ;\
(2) npm-ci: cd fixtures/npm-fixture && npm ci ;\
(3) npm-test: cd fixtures/npm-fixture && npm test ;\
(4) npm-build: cd fixtures/npm-fixture && npm run build ;\
(5) podman-build: podman build -f fixtures/podman-fixture/Containerfile -t aria-t22-check fixtures/podman-fixture ;\
(6) podman-run: cd fixtures/podman-fixture && podman run --rm aria-t22-check 。\
六条验证命令都必须由 Coder 自己在 CHECK 阶段真实执行并在完成报告记录退出码与测试数量。\
执行形态必须逐字保持:cd 目标目录、-v 挂载的 $PWD 形态、第 (5) 条的显式构建上下文路径\
(以及 -f 指定的 Containerfile)都不得改写——不得改用 . 上下文、不得改为绝对路径挂载执行。\
工作项划分约束:全部六条命令归属同一个工作项(即代码改动工作项),验收标准与验证计划\
的路径字段一律填写 fixtures/ 开头的具体仓库内相对路径(机械校验拒绝基线树外路径);\
禁止为 CHECK-EVIDENCE.log 或仓库根单独建立工作项或以 [.]/根路径作为验收/验证路径。\
证据落盘要求:每条命令执行时把完整输出(含计数行,如 Maven 的 \"Tests run:\" 行、\
npm 的 passed/failed 计数行、各命令的 EXIT 提示行)追加写入仓库根目录文件 \
CHECK-EVIDENCE.log(可经 tee -a 或重定向后 cat 追加;该文件随工作区提交;不要把该路径写进 AC)。\n\
\n\
{ISSUE_AUTHOR_READONLY_CLAUSE}"
    )
}

#[cfg(test)]
mod b45_t22_diagnostics_tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn write_file(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("夹具目录");
        }
        std::fs::write(path, body).expect("夹具文件");
    }

    fn scratch_root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("b45-t22-diag-{}-{tag}", std::process::id()))
    }

    /// 先红后绿:有机械报告时证据根必须含副本,且摘要直接可见 Error finding
    /// 的 code/message(T22 现场缺的正是这一项)。
    #[test]
    fn capture_copies_mechanical_report_and_surfaces_findings() {
        let base = scratch_root("capture");
        let _ = std::fs::remove_dir_all(&base);
        let aria = base.join(".aria");
        let issue = aria.join("projects/project_0001/issues/issue_0001");
        let plan_id = "issue_work_item_plan_0001";
        let report = json!({
            "id": "report-x",
            "source_revision_id": "source-1",
            "ir_id": "ir-1",
            "report": {
                "source_revision_hash": "hash",
                "compiler_version": "v1",
                "findings": [{
                    "severity": "error",
                    "code": "acceptance_path_not_in_baseline",
                    "message": "work item WI-001 的验收标准/验证计划引用路径 [.] 不存在于 plan 基线树",
                    "work_item_ids": ["WI-001"]
                }]
            },
            "content_hash": "content-hash",
        });
        let report_relative =
            format!("work-item-plan-sources/{plan_id}/mechanical_report/report-x.json");
        write_file(&issue.join(&report_relative), &report.to_string());
        let source_relative =
            format!("work-item-plan-sources/{plan_id}/source_revision/source-1.json");
        write_file(
            &issue.join(&source_relative),
            &json!({"id": "source-1", "source": "# Work Item Plan"}).to_string(),
        );
        // 编译事务(顶层目录名无前导 `/`,分类必须按目录族名匹配)。
        let compile_relative = format!("work_item_plan_compiles/{plan_id}/compile-1.json");
        write_file(
            &issue.join(&compile_relative),
            &json!({
                "compile_id": "compile-1",
                "status": "failed",
                "failure_reason": "mechanical_validation_failed",
                "mechanical_report_ref": "project/project_0001/issue/issue_0001/plan/issue_work_item_plan_0001/mechanical_report/report-x",
                "validator_findings": [{
                    "severity": "error",
                    "code": "acceptance_path_not_in_baseline",
                    "message": "引用路径 [.] 不存在于 plan 基线树",
                    "work_item_ids": ["WI-001"]
                }]
            })
            .to_string(),
        );
        // draft 记录(single candidate 候选本体所在族)。
        let draft_relative = format!("work_item_plan_drafts/{plan_id}/round_1/draft_1.json");
        write_file(
            &issue.join(&draft_relative),
            &json!({
                "draft_id": "draft_1",
                "generation_round_id": "round_1",
                "status": "active",
                "active": true
            })
            .to_string(),
        );
        // 产品文件锁是空占位,不得进证据根。
        let lock_relative =
            format!("work-item-plan-sources/{plan_id}/mechanical_report/.report-x.json.lock");
        write_file(&issue.join(&lock_relative), "");

        let evidence = base.join("host-check");
        let summary =
            b45_capture_plan_diagnostics(&aria, ("project_0001", "issue_0001", plan_id), &evidence);

        let copied = evidence.join("plan-diagnostics").join(&report_relative);
        assert!(
            copied.is_file(),
            "机械报告副本必须进证据根:{}",
            copied.display()
        );
        let copied_body = std::fs::read_to_string(&copied).expect("副本可读");
        assert!(
            copied_body.contains("acceptance_path_not_in_baseline"),
            "副本必须是原始记录"
        );
        assert!(
            summary
                .pointer("/mechanical_reports/0/findings/0/code")
                .and_then(Value::as_str)
                == Some("acceptance_path_not_in_baseline"),
            "摘要必须直出 finding code:{summary}"
        );
        assert!(
            summary
                .pointer("/mechanical_reports/0/findings/0/message")
                .and_then(Value::as_str)
                .is_some_and(|message| message.contains("不存在于 plan 基线树")),
            "摘要必须直出 finding message:{summary}"
        );
        assert_eq!(
            summary
                .pointer("/source_revisions/0/source_chars")
                .and_then(Value::as_u64),
            Some("# Work Item Plan".chars().count() as u64),
        );
        assert!(
            evidence.join("plan_diagnostics.json").is_file(),
            "摘要必须落盘:{}",
            evidence.join("plan_diagnostics.json").display()
        );
        // 编译事务:顶层目录名 `work_item_plan_compiles/` 无前导 `/`,
        // 分类必须命中(否则失败原因/validator_findings 只剩裸文件)。
        assert_eq!(
            summary
                .pointer("/compile_transactions/0/status")
                .and_then(Value::as_str),
            Some("failed"),
            "编译事务必须进摘要:{summary}"
        );
        assert_eq!(
            summary
                .pointer("/compile_transactions/0/failure_reason")
                .and_then(Value::as_str),
            Some("mechanical_validation_failed"),
        );
        assert_eq!(
            summary
                .pointer("/compile_transactions/0/validator_findings/0/code")
                .and_then(Value::as_str),
            Some("acceptance_path_not_in_baseline"),
        );
        assert_eq!(
            summary
                .pointer("/drafts/0/draft_id")
                .and_then(Value::as_str),
            Some("draft_1"),
            "draft 记录必须进摘要:{summary}"
        );
        assert!(
            !evidence
                .join("plan-diagnostics")
                .join(&lock_relative)
                .exists(),
            "产品文件锁不得进证据根"
        );
        assert!(
            summary
                .get("errors")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty),
            "无 IO 失败时 errors 必须为空:{summary}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 钉定 issue 描述必须带全 14 节骨架、显式 AC 硬约束,且 podman-build 上下文
    /// 不再是 `.`(t25 现场 `acceptance_path_not_in_baseline` 的唯一来源)。
    #[test]
    fn pinned_issue_description_matches_compiler_grammar() {
        let text = b45_kimi_issue_description();
        let sections = cadence_aria::product::work_item_plan_compiler::grammar::STRUCTURED_SECTIONS;
        assert_eq!(sections.len(), 14, "编译器结构化节清单口径");
        for section in sections {
            assert!(text.contains(section), "缺节标题 {section}");
        }
        assert!(text.contains("验收标准(AC)硬约束"), "缺 AC 硬约束段");
        assert!(
            text.contains("acceptance_path_not_in_baseline"),
            "缺基线路径违规码的显式禁令"
        );
        assert!(
            !text.contains("podman build -t aria-t22-check ."),
            "podman-build 上下文不得是 `.`(基线树外路径→机械校验 Error)"
        );
        assert!(
            text.contains("podman build -f fixtures/podman-fixture/Containerfile"),
            "podman-build 必须用显式基线路径上下文"
        );
        assert!(text.contains("六条"), "六条验证命令的钉定口径不得丢");
    }

    /// 负测:.aria 副本已销毁(issue 根缺失)时不 panic,错误如实进摘要。
    #[test]
    fn capture_without_aria_root_reports_error_without_panic() {
        let base = scratch_root("missing");
        let _ = std::fs::remove_dir_all(&base);
        let summary = b45_capture_plan_diagnostics(
            &base.join(".aria"),
            ("project_0001", "issue_0001", "issue_work_item_plan_0001"),
            &base.join("host-check"),
        );
        assert!(
            summary
                .get("errors")
                .and_then(Value::as_array)
                .is_some_and(|errors| !errors.is_empty()),
            "缺 .aria 必须记错误:{summary}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// coding-attempt 400 响应体必须原样落证据(分类码不丢)。
    #[test]
    fn record_http_failure_persists_status_and_body() {
        let base = scratch_root("http");
        let _ = std::fs::remove_dir_all(&base);
        b45_record_http_failure(
            &base,
            "create-main",
            400,
            &json!({"error": "work_item_plan_not_confirmed"}),
        );
        let written: Value = serde_json::from_str(
            &std::fs::read_to_string(base.join("coding-attempt-create-main.json"))
                .expect("响应体已落盘"),
        )
        .expect("响应体 JSON");
        assert_eq!(written.get("status").and_then(Value::as_u64), Some(400));
        assert_eq!(
            written.pointer("/body/error").and_then(Value::as_str),
            Some("work_item_plan_not_confirmed"),
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
