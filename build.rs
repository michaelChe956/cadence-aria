use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    emit_build_fingerprint();

    let dist = Path::new("web/dist");
    let index = dist.join("index.html");

    println!("cargo:rerun-if-changed=web/dist");
    println!("cargo:rerun-if-changed=web/dist/index.html");

    if !index.is_file() {
        panic!(
            "web/dist/index.html 不存在——前端产物未构建。\n\
             请先运行：pnpm -C web install && pnpm -C web build\n\
             （web/dist 被 web/.gitignore 忽略、不在版本控制中，干净 checkout 后需手动构建。\n\
             aria 二进制通过 rust-embed 在编译期嵌入 web/dist，缺失会导致运行时全站白屏。）"
        );
    }

    // 校验非空：assets 目录或至少一个资源存在
    let has_assets = std::fs::read_dir(dist)
        .map(|mut entries| entries.any(|e| e.is_ok()))
        .unwrap_or(false);
    if !has_assets {
        panic!("web/dist 为空目录——请重新运行 pnpm -C web build 生成完整前端产物。");
    }
}

fn emit_build_fingerprint() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    if let Some(git_sha) = git_output(&["rev-parse", "--short=12", "HEAD"]) {
        println!("cargo:rustc-env=ARIA_GIT_SHA={git_sha}");
    } else {
        println!("cargo:rustc-env=ARIA_GIT_SHA=unknown");
    }
    if let Some(branch) = git_output(&["branch", "--show-current"]) {
        println!("cargo:rustc-env=ARIA_GIT_BRANCH={branch}");
    } else {
        println!("cargo:rustc-env=ARIA_GIT_BRANCH=unknown");
    }
    let built_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=ARIA_BUILT_AT_UNIX={built_at}");
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
