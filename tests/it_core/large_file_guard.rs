use std::fs;
use std::path::{Path, PathBuf};

const MAX_PRODUCT_CODE_LINES: usize = 800;
const SCAN_ROOTS: &[&str] = &["src", "tests", "web/src"];
const CODE_EXTENSIONS: &[&str] = &["rs", "ts", "tsx"];

#[test]
fn product_source_and_test_files_stay_under_line_limit() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut oversized = Vec::new();

    for scan_root in SCAN_ROOTS {
        collect_oversized_files(&repo_root.join(scan_root), &repo_root, &mut oversized);
    }

    oversized.sort();
    assert!(
        oversized.is_empty(),
        "产品源码与测试文件必须保持在 {MAX_PRODUCT_CODE_LINES} 行以内，当前超限：\n{}",
        oversized.join("\n"),
    );
}

fn collect_oversized_files(path: &Path, repo_root: &Path, oversized: &mut Vec<String>) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };

    if metadata.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            collect_oversized_files(&entry.path(), repo_root, oversized);
        }
        return;
    }

    if !metadata.is_file() || !is_product_code_file(path) {
        return;
    }

    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    let line_count = content.lines().count();
    if line_count > MAX_PRODUCT_CODE_LINES {
        let relative = path.strip_prefix(repo_root).unwrap_or(path);
        oversized.push(format!("{}: {line_count} 行", relative.display()));
    }
}

fn is_product_code_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| CODE_EXTENSIONS.contains(&extension))
}
