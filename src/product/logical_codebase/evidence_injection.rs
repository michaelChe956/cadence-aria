// C-4 跨仓只读证据中介：worktree 注入脚本模板（Task 7）。
//
// 契约 REQ-COD-05「受控接口」；设计 §3.2/§3.3/§5.4。本模块内嵌注入脚本
// 模板 `EVIDENCE_QUERY_SCRIPT`（bash：读 `.aria/evidence-token` 与
// `.aria/web-endpoint`，`curl -s POST` 到 `/api/evidence-query`，`--role` 参数自报
// Coder/Reviewer），并暴露 `write_evidence_query_script` 供
// `execute_worktree_prepare` 在 worktree 创建完成后幂等写入
// `.aria/bin/aria-evidence-query`（0755 可执行）。

use std::path::Path;

use crate::product::logical_codebase::evidence_index::EvidenceError;

/// 注入脚本在 worktree 内的相对路径：`.aria/bin/aria-evidence-query`。
pub const EVIDENCE_QUERY_SCRIPT_PATH: &str = ".aria/bin/aria-evidence-query";

/// 注入脚本模板内容（bash）。写入 worktree 后由 Coder/Reviewer 在 worktree 内调用。
pub const EVIDENCE_QUERY_SCRIPT: &str = r#"#!/usr/bin/env bash
# Aria C-4 跨仓只读证据查询脚本（注入到 worktree `.aria/bin/aria-evidence-query`）。
# 用法: aria-evidence-query --role <coder|reviewer> --query <symbol>
# 预算规则: 收到 429 即停止查询（attempt 累计配额已耗尽）。
set -euo pipefail

usage() {
    echo "usage: $0 --role <coder|reviewer> --query <symbol>" >&2
    exit 2
}

role=""
query=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --role)
            role="${2:-}"
            shift 2
            ;;
        --query)
            query="${2:-}"
            shift 2
            ;;
        *)
            usage
            ;;
    esac
done

if [[ -z "${role}" || -z "${query}" ]]; then
    usage
fi

worktree_root="$(git rev-parse --show-toplevel)"
token="$(cat "${worktree_root}/.aria/evidence-token")"
port="$(cat "${worktree_root}/.aria/web-endpoint")"
body="$(printf '{"token":"%s","role":"%s","query":"%s"}' "$token" "$role" "$query")"

curl -s -X POST "http://127.0.0.1:${port}/api/evidence-query" -H 'Content-Type: application/json' --data "$body"
"#;

/// 把查询脚本幂等写入 worktree `.aria/bin/aria-evidence-query`（0755）。
pub fn write_evidence_query_script(worktree: &Path) -> Result<(), EvidenceError> {
    let dir = worktree.join(EVIDENCE_QUERY_SCRIPT_PATH);
    let parent = dir.parent().ok_or_else(|| EvidenceError::Io {
        message: format!("invalid script path: {}", dir.display()),
    })?;
    std::fs::create_dir_all(parent).map_err(|error| EvidenceError::Io {
        message: format!("create {}: {error}", parent.display()),
    })?;
    std::fs::write(&dir, EVIDENCE_QUERY_SCRIPT).map_err(|error| EvidenceError::Io {
        message: format!("write {}: {error}", dir.display()),
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).map_err(
            |error| EvidenceError::Io {
                message: format!("chmod {}: {error}", dir.display()),
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn script_path(worktree: &Path) -> std::path::PathBuf {
        worktree.join(EVIDENCE_QUERY_SCRIPT_PATH)
    }

    #[test]
    fn evidence_query_script_reads_token_endpoint_and_role() {
        assert!(
            EVIDENCE_QUERY_SCRIPT.contains(".aria/evidence-token"),
            "script must read the injected token file"
        );
        assert!(
            EVIDENCE_QUERY_SCRIPT.contains(".aria/web-endpoint"),
            "script must read the injected endpoint port file"
        );
        assert!(
            EVIDENCE_QUERY_SCRIPT.contains("curl -s"),
            "script must invoke curl silently"
        );
        assert!(
            EVIDENCE_QUERY_SCRIPT.contains("--role"),
            "script must accept the --role parameter"
        );
        assert!(
            EVIDENCE_QUERY_SCRIPT.contains("/api/evidence-query"),
            "script must POST to the evidence query endpoint"
        );
        assert!(
            EVIDENCE_QUERY_SCRIPT.contains("POST"),
            "script must use HTTP POST"
        );
    }

    #[test]
    fn evidence_query_script_writes_executable_file_idempotently() {
        let tmp = TempDir::new().expect("tempdir");
        let worktree = tmp.path().join("worktree");

        write_evidence_query_script(&worktree).expect("write script");

        let path = script_path(&worktree);
        assert_eq!(
            fs::read_to_string(&path).expect("read script"),
            EVIDENCE_QUERY_SCRIPT
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path)
                .expect("script metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755, "script must be 0755, got {mode:o}");
        }

        // 幂等：二次写入不报错、内容不变。
        write_evidence_query_script(&worktree).expect("re-write script");
        assert_eq!(
            fs::read_to_string(&path).expect("re-read script"),
            EVIDENCE_QUERY_SCRIPT
        );
    }
}
