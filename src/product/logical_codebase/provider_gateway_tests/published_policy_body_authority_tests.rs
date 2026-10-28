//! REQ（owner 2026-10-26 裁定）：政策校验以「当前已发布政策」为唯一权威，
//! 不比对冷启动回执的冻结摘要——规则变更后回执必然落后。
#[cfg(test)]
use super::super::verify_published_policy_body;
use crate::product::logical_codebase::policy::AggregatePolicyArtifact;
use sha2::{Digest, Sha256};

fn artifact(root: &std::path::Path, body: &str, revision: u64) -> AggregatePolicyArtifact {
    let _ = root;
    AggregatePolicyArtifact {
        policy_id: format!("policy/project_0001/lc-uuid/{revision}"),
        project_id: "project_0001".to_string(),
        logical_codebase_id: "lc-uuid".to_string(),
        revision,
        digest: format!("sha256:{:x}", Sha256::digest(body.as_bytes())),
        policy_text: body.to_string(),
        created_at: "2026-10-26T00:00:00Z".to_string(),
    }
}

fn fixture(body: &str, rule: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir_all(root.join("policy/project_0001/lc-uuid")).unwrap();
    std::fs::write(root.join("policy/project_0001/lc-uuid/4"), body).unwrap();
    std::fs::write(root.join("AGENTS.md"), rule).unwrap();
    (temp, root)
}

#[test]
fn accepts_current_policy_and_returns_current_rule_digest() {
    let body = "# 聚合政策\n\n## 来源：AGENTS.md\n\n规则v2\n";
    let rule = "规则v2\n";
    let (_temp, root) = fixture(body, rule);
    let artifact = artifact(&root, body, 4);

    let rule_digest = verify_published_policy_body(&root, &artifact).unwrap();
    assert_eq!(
        rule_digest,
        format!("sha256:{:x}", Sha256::digest(rule.as_bytes())),
        "规则摘要必须现场取当前根规则"
    );
}

#[test]
fn rejects_tampered_body_and_inconsistent_digest() {
    let body = "# 聚合政策\n\n## 来源：AGENTS.md\n\n规则v2\n";
    let (_temp, root) = fixture(body, "规则v2\n");
    let artifact = artifact(&root, body, 4);

    std::fs::write(root.join("policy/project_0001/lc-uuid/4"), "手改进正文\n").unwrap();
    let error = verify_published_policy_body(&root, &artifact).unwrap_err();
    assert!(
        error.to_string().contains("policy_body"),
        "文件被手改必须拒绝:{error}"
    );

    std::fs::write(root.join("policy/project_0001/lc-uuid/4"), body).unwrap();
    let mut forged = artifact.clone();
    forged.digest = "sha256:deadbeef".to_string();
    let error = verify_published_policy_body(&root, &forged).unwrap_err();
    assert!(
        error.to_string().contains("policy_digest_chain"),
        "artifact 摘要不自洽必须拒绝:{error}"
    );
}
