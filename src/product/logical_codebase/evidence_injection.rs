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

/// 证据端点发现文件的相对路径：workspace 根（`serve_web` 绑定成功后写入）与
/// worktree（注入脚本 `git rev-parse --show-toplevel` 后读取）均为
/// `.aria/web-endpoint`。与 `src/web/app.rs` 的 `WEB_ENDPOINT_FILE` 值一致，但
/// 本模块不依赖 web 层，故独立声明（避免 product → web 反向依赖）。
pub const WEB_ENDPOINT_FILE: &str = ".aria/web-endpoint";

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

/// 把 workspace 根的 `.aria/web-endpoint`（`serve_web` 绑定成功后写入的端口文件）
/// 随注入物复制进 worktree 的 `.aria/web-endpoint`（与 evidence-token 同目录），
/// 使注入脚本经 `git rev-parse --show-toplevel`（linked worktree → worktree 自身）
/// 能读到自己 worktree 内的端口文件（Critical-1 根目录错位修复）。
///
/// 源缺失（serve_web 尚未启动/未写端口文件）→ 返回 `EvidenceError::Io`（message
/// 前缀 `endpoint-not-ready`），T8 挂钩据此 warning 且不阻断 attempt 启动；
/// 幂等覆盖：源更新后再次注入会覆盖 worktree 内旧端口。
pub fn inject_web_endpoint(
    workspace_root: &Path,
    worktree_path: &Path,
) -> Result<(), EvidenceError> {
    let source = workspace_root.join(WEB_ENDPOINT_FILE);
    let content = std::fs::read_to_string(&source).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            EvidenceError::Io {
                message: format!("endpoint-not-ready: {} not found", source.display()),
            }
        } else {
            EvidenceError::Io {
                message: format!("read {}: {error}", source.display()),
            }
        }
    })?;
    let target = worktree_path.join(WEB_ENDPOINT_FILE);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|error| EvidenceError::Io {
            message: format!("create {}: {error}", parent.display()),
        })?;
    }
    std::fs::write(&target, content).map_err(|error| EvidenceError::Io {
        message: format!("write {}: {error}", target.display()),
    })?;
    Ok(())
}

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
    fn inject_web_endpoint_copies_source_to_worktree() {
        let tmp = TempDir::new().expect("tempdir");
        let workspace_root = tmp.path().join("workspace-root");
        let worktree = tmp.path().join("worktree");
        fs::create_dir_all(workspace_root.join(".aria")).expect("create workspace .aria");
        fs::write(workspace_root.join(WEB_ENDPOINT_FILE), "43210\n").expect("write source");

        inject_web_endpoint(&workspace_root, &worktree).expect("inject endpoint");

        let target = worktree.join(WEB_ENDPOINT_FILE);
        assert_eq!(
            fs::read_to_string(&target).expect("read injected endpoint"),
            "43210\n",
            "injected endpoint must copy source content verbatim"
        );
    }

    #[test]
    fn inject_web_endpoint_source_missing_returns_error() {
        let tmp = TempDir::new().expect("tempdir");
        let workspace_root = tmp.path().join("workspace-root");
        let worktree = tmp.path().join("worktree");
        fs::create_dir_all(&worktree).expect("create worktree");

        let err = inject_web_endpoint(&workspace_root, &worktree).expect_err("missing source");
        assert!(
            matches!(err, EvidenceError::Io { .. }),
            "missing source must surface as EvidenceError::Io, got {err:?}"
        );
    }

    #[test]
    fn inject_web_endpoint_overwrites_idempotently() {
        let tmp = TempDir::new().expect("tempdir");
        let workspace_root = tmp.path().join("workspace-root");
        let worktree = tmp.path().join("worktree");
        fs::create_dir_all(workspace_root.join(".aria")).expect("create workspace .aria");
        fs::write(workspace_root.join(WEB_ENDPOINT_FILE), "1111").expect("write first source");

        inject_web_endpoint(&workspace_root, &worktree).expect("first inject");
        assert_eq!(
            fs::read_to_string(worktree.join(WEB_ENDPOINT_FILE)).expect("read first"),
            "1111"
        );

        // 源更新后再次注入 → worktree 内文件幂等覆盖为最新端口。
        fs::write(workspace_root.join(WEB_ENDPOINT_FILE), "2222").expect("write second source");
        inject_web_endpoint(&workspace_root, &worktree).expect("second inject");
        assert_eq!(
            fs::read_to_string(worktree.join(WEB_ENDPOINT_FILE)).expect("read second"),
            "2222"
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
