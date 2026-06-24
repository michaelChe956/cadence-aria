use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

use serde_json::{Value, json};

use super::types::{MAX_LISTED_FILES, MAX_SEARCH_MATCHES};

pub(super) fn read_file_tool(input: &Value, worktree_path: &Path) -> Result<String, String> {
    let path = input_path(input, "path", ".")?;
    let path = resolve_existing_worktree_path(worktree_path, &path)?;
    std::fs::read_to_string(&path)
        .map_err(|error| format!("读取文件失败 {}: {error}", path.display()))
}

pub(super) fn list_files_tool(input: &Value, worktree_path: &Path) -> Result<String, String> {
    let path = input_path(input, "path", ".")?;
    let root = resolve_existing_worktree_path(worktree_path, &path)?;
    let mut files = Vec::new();
    collect_files(&root, worktree_path, &mut files, MAX_LISTED_FILES)?;
    Ok(json!({ "files": files }).to_string())
}

pub(super) fn search_code_tool(input: &Value, worktree_path: &Path) -> Result<String, String> {
    let query = input
        .get("query")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "search_code 缺少 query 参数".to_string())?;
    let path = input_path(input, "path", ".")?;
    let root = resolve_existing_worktree_path(worktree_path, &path)?;
    let mut matches = Vec::new();
    search_files(
        &root,
        worktree_path,
        query,
        &mut matches,
        MAX_SEARCH_MATCHES,
    )?;
    Ok(json!({ "matches": matches }).to_string())
}

fn input_path(input: &Value, field: &str, default: &str) -> Result<PathBuf, String> {
    let value = input
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(default);
    let path = PathBuf::from(value);
    if path.is_absolute() {
        return Err("工具路径必须是 worktree 内的相对路径".to_string());
    }
    Ok(path)
}

fn resolve_existing_worktree_path(
    worktree_path: &Path,
    relative_path: &Path,
) -> Result<PathBuf, String> {
    let root = worktree_path
        .canonicalize()
        .map_err(|error| format!("解析 worktree 路径失败: {error}"))?;
    let path = worktree_path
        .join(relative_path)
        .canonicalize()
        .map_err(|error| format!("解析工具路径失败 {}: {error}", relative_path.display()))?;
    if !path.starts_with(&root) {
        return Err("工具路径不能逃逸 worktree".to_string());
    }
    Ok(path)
}

fn collect_files(
    path: &Path,
    worktree_path: &Path,
    files: &mut Vec<String>,
    max_files: usize,
) -> Result<(), String> {
    if files.len() >= max_files || ignored_path(path) {
        return Ok(());
    }
    if path.is_file() {
        files.push(relative_display_path(path, worktree_path));
        return Ok(());
    }
    let entries = std::fs::read_dir(path)
        .map_err(|error| format!("列出目录失败 {}: {error}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("读取目录项失败: {error}"))?;
        collect_files(&entry.path(), worktree_path, files, max_files)?;
        if files.len() >= max_files {
            break;
        }
    }
    Ok(())
}

fn search_files(
    path: &Path,
    worktree_path: &Path,
    query: &str,
    matches: &mut Vec<Value>,
    max_matches: usize,
) -> Result<(), String> {
    if matches.len() >= max_matches || ignored_path(path) {
        return Ok(());
    }
    if path.is_dir() {
        let entries = std::fs::read_dir(path)
            .map_err(|error| format!("读取目录失败 {}: {error}", path.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("读取目录项失败: {error}"))?;
            search_files(&entry.path(), worktree_path, query, matches, max_matches)?;
            if matches.len() >= max_matches {
                break;
            }
        }
        return Ok(());
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    for (line_index, line) in content.lines().enumerate() {
        if !line.contains(query) {
            continue;
        }
        matches.push(json!({
            "path": relative_display_path(path, worktree_path),
            "line": line_index + 1,
            "text": line
        }));
        if matches.len() >= max_matches {
            break;
        }
    }
    Ok(())
}

fn ignored_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| matches!(name, ".git" | ".aria" | "target" | "node_modules"))
}

fn relative_display_path(path: &Path, worktree_path: &Path) -> String {
    path.strip_prefix(worktree_path)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

pub(super) fn detect_changed_files(worktree_path: &Path) -> Vec<String> {
    let Ok(output) = StdCommand::new("git")
        .arg("-C")
        .arg(worktree_path)
        .arg("status")
        .arg("--short")
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.get(3..).map(str::trim))
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
        .collect()
}
