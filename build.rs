use std::path::Path;

fn main() {
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
