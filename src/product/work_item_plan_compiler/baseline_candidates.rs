//! D5 事后：越界路径回灌基线树最近候选（REQ-WSC-06 场景「越界路径回灌最近候选」）。
//!
//! 编译器判定某验收标准引用的仓库内路径不存在于 plan 基线树时，把**同一棵
//! 基线树**中最近的可用路径**附进 finding message（走既有教学重驱通道，让
//! 作者看到可改成的真实路径，而不是拿到一条「去改」的空洞指令）。
//!
//! 口径只有两步（**不引入其它相似度算法**：无编辑距离、无 LCS、无模糊匹配）：
//! 1. 缺失路径有父目录前缀 ⇒ 取基线树中以该前缀 + `/` 开头的路径；
//! 2. 上一步为空 ⇒ 取基线树中 basename 与缺失路径 basename 相等的路径。
//!
//! 返回值**全部 ∈ `baseline_tree`**（平台不得臆造路径——臆造即制造新的越界
//! 引用）；顺序为 `BTreeSet` 字典序。

use std::collections::BTreeSet;

/// 给定缺失路径与 plan 基线树，返回最近可用候选路径。
///
/// 返回空 `Vec` 表示基线树内既无同父目录路径、也无同名路径（调用方据此
/// 如实说明「无可用候选」，不得编造）。
pub(crate) fn nearest_baseline_candidates<'a>(
    missing: &str,
    baseline_tree: &'a BTreeSet<String>,
) -> Vec<&'a str> {
    // ① 同父目录前缀（`a/b/c.js` ⇒ 前缀 `a/b`；无 `/` ⇒ 无目录前缀）。
    let directory_prefix = missing
        .rsplit_once('/')
        .map(|(directory, _)| directory)
        .filter(|directory| !directory.is_empty());
    let mut candidates: Vec<&str> = Vec::new();
    if let Some(directory) = directory_prefix {
        let needle = format!("{directory}/");
        candidates.extend(
            baseline_tree
                .iter()
                .filter(|path| path.starts_with(&needle))
                .map(String::as_str),
        );
    }
    // ② 同父目录无候选 ⇒ 同名（basename 相等）路径。
    if candidates.is_empty() {
        let basename = missing.rsplit('/').next().unwrap_or(missing);
        candidates.extend(
            baseline_tree
                .iter()
                .filter(|path| path.rsplit('/').next() == Some(basename))
                .map(String::as_str),
        );
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|path| path.to_string()).collect()
    }

    #[test]
    fn same_parent_directory_candidates_are_returned() {
        let baseline = tree(&["scripts/build.mjs", "scripts/release.mjs", "src/main.rs"]);
        assert_eq!(
            nearest_baseline_candidates("scripts/run.mjs", &baseline),
            vec!["scripts/build.mjs", "scripts/release.mjs"]
        );
    }

    #[test]
    fn same_basename_is_the_fallback_when_directory_has_no_candidate() {
        let baseline = tree(&["tests/unit/notice.test.mjs", "src/main.rs"]);
        assert_eq!(
            nearest_baseline_candidates("tests/e2e/notice.test.mjs", &baseline),
            vec!["tests/unit/notice.test.mjs"]
        );
    }

    #[test]
    fn no_candidate_yields_empty_list() {
        let baseline = tree(&["src/main/App.vue"]);
        assert!(nearest_baseline_candidates("scripts/run.mjs", &baseline).is_empty());
    }

    #[test]
    fn candidates_always_belong_to_the_baseline_tree() {
        let baseline = tree(&["tests/unit/notice.test.mjs", "scripts/build.mjs"]);
        for missing in [
            "tests/e2e/notice.test.mjs",
            "scripts/run.mjs",
            "notice.test.mjs",
        ] {
            for candidate in nearest_baseline_candidates(missing, &baseline) {
                assert!(
                    baseline.contains(candidate),
                    "候选必须 ∈ 基线树：{candidate}"
                );
            }
        }
    }

    #[test]
    fn root_level_missing_path_falls_back_to_same_basename() {
        let baseline = tree(&["notice.test.mjs", "src/main.rs"]);
        assert_eq!(
            nearest_baseline_candidates("notice.test.mjs", &baseline),
            vec!["notice.test.mjs"]
        );
    }
}
