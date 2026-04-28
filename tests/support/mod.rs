#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub fn write_executable_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    fs::create_dir_all(dir).expect("create script dir");
    let path = dir.join(name);
    fs::write(&path, body).expect("write script");
    let mut permissions = fs::metadata(&path).expect("script metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("chmod script");
    path
}

pub fn successful_provider_script() -> &'static str {
    r#"#!/bin/sh
set -eu
case "${1:-run}" in
  probe)
    echo "ok"
    exit 0
    ;;
  version)
    echo "fixture-provider 1.2.3"
    exit 0
    ;;
  auth)
    echo "authorized"
    exit 0
    ;;
  run)
    cat >/dev/null
    if [ "${2:-}" != "" ]; then
      printf '%s' "changed" > "$2/generated.txt"
    fi
    echo "provider log line"
    echo "<ARIA_STRUCTURED_OUTPUT>"
    echo '{"artifact_kind":"clarification_record","goal_summary":"fixture goal","constraints":["fixture constraint"],"open_questions":[],"assumptions":["fixture"],"suggested_scope":"fixture scope"}'
    echo "</ARIA_STRUCTURED_OUTPUT>"
    exit 0
    ;;
esac
"#
}

pub fn unauthorized_provider_script() -> &'static str {
    r#"#!/bin/sh
set -eu
case "${1:-run}" in
  probe|version)
    echo "fixture-provider 1.2.3"
    exit 0
    ;;
  auth|run)
    echo "not logged in" >&2
    exit 42
    ;;
esac
"#
}

pub fn permission_denied_provider_script() -> &'static str {
    r#"#!/bin/sh
set -eu
case "${1:-run}" in
  probe|version|auth)
    echo "ok"
    exit 0
    ;;
  run)
    echo "permission denied" >&2
    exit 13
    ;;
esac
"#
}

pub fn parse_error_provider_script() -> &'static str {
    r#"#!/bin/sh
set -eu
case "${1:-run}" in
  probe|version|auth)
    echo "ok"
    exit 0
    ;;
  run)
    echo "provider log without sentinel"
    exit 0
    ;;
esac
"#
}

pub fn incompatible_output_provider_script() -> &'static str {
    r#"#!/bin/sh
set -eu
case "${1:-run}" in
  probe|version|auth)
    echo "ok"
    exit 0
    ;;
  run)
    echo "<ARIA_STRUCTURED_OUTPUT>"
    echo '{"artifact_kind":"spec"}'
    echo "</ARIA_STRUCTURED_OUTPUT>"
    exit 0
    ;;
esac
"#
}

pub fn timeout_provider_script() -> &'static str {
    r#"#!/bin/sh
set -eu
case "${1:-run}" in
  probe|version|auth)
    echo "ok"
    exit 0
    ;;
  run)
    sleep 5
    exit 0
    ;;
esac
"#
}
