//! 成员仓基线内容(harness 建仓时 `commit()` 写入 `lib.rs` 的原文)。
//! ——大文件守卫:harness 已按祖父条款登记 14966 行上限,基线原文与其
//! rustfmt 钉住单测整体落本文件,harness 只留接线。
//!
//! 现场根因(kimi r2,诊断 diag-kimi-coding-cell §3 R1):coder 的必需
//! 检查 `cargo fmt --check` 对**基线自带**的差异也报红——review_0005
//! 原文「第 4 行 cross_repo_greeting 单行函数…其中后两处属于本次功能
//! 新增代码」,第 4 行即本夹具原样。默认 `fn_single_line=false` 会把单
//! 行函数体拆成多行,于是「不改 lib.rs 就永远 fmt 失败」,而 lib.rs 又
//! 在 tests-only 工作项的 forbidden_scopes 内 → 检查不可满足。基线必须
//! 逐字 rustfmt 干净(含文件尾换行:`rustfmt --check` 会把缺尾换行的
//! 文件判红并补一行空行)。
//!
//! F1 只消除「基线自带一处 fmt 差异」这一不可达源;检查×写域冲突的
//! 根治在 issue 侧计划自洽句(见 `issue_scope_guards::
//! ISSUE_PLAN_SELF_CONSISTENCY_CLAUSE`)。

/// alpha 成员基线 `lib.rs`(rustfmt 多行函数体,含尾换行)。
pub(super) const MEMBER_LIB_RS_ALPHA: &str =
    "pub fn cross_repo_greeting() -> &'static str {\n    \"alpha\"\n}\n";

/// beta 成员基线 `lib.rs`(alpha 同形,只换返回值)。
pub(super) const MEMBER_LIB_RS_BETA: &str =
    "pub fn cross_repo_greeting() -> &'static str {\n    \"beta\"\n}\n";

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::*;

    /// 基线必须逐字 `rustfmt --check` 通过(防「不改 lib.rs 就永远 fmt
    /// 失败」的不可满足检查回归)。两个成员都要钉:beta 同形,单独漂移
    /// 同样会让跨仓格式化要求不可达。
    #[test]
    fn member_baseline_lib_rs_is_rustfmt_clean() {
        for (text, member) in [(MEMBER_LIB_RS_ALPHA, "alpha"), (MEMBER_LIB_RS_BETA, "beta")] {
            // 纯文本钉住形态:多行函数体 + 文件尾换行(fn_single_line 默认
            // false;缺尾换行同样判红)。
            assert_eq!(
                text,
                format!("pub fn cross_repo_greeting() -> &'static str {{\n    \"{member}\"\n}}\n"),
                "基线 lib.rs({member})必须是 rustfmt 多行形态且以换行结尾"
            );
            assert!(
                text.ends_with('\n'),
                "基线 lib.rs({member})必须以换行结尾(rustfmt --check 判缺尾换行)"
            );
            if let Some(diff) = rustfmt_check_diff(text, member) {
                panic!("基线 lib.rs({member})未通过 rustfmt --check:\n{diff}");
            }
        }
    }

    /// 以真实 `rustfmt --check` 复核基线原文;rustfmt 不可用(精简环境)
    /// 时返回 None(纯文本钉住仍生效),启动错误以外的失败必须冒泡。
    fn rustfmt_check_diff(text: &str, member: &str) -> Option<String> {
        let dir = std::env::temp_dir().join(format!(
            "aria-t23-member-baseline-{}-{member}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("基线校验目录");
        let path = dir.join("lib.rs");
        std::fs::write(&path, text).expect("写基线副本");
        let output = std::process::Command::new("rustfmt")
            .args(["--edition", "2024", "--check"])
            .arg(&path)
            .current_dir(&dir)
            .output();
        let _ = std::fs::remove_dir_all(&dir);
        let output = match output {
            Ok(output) => output,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                eprintln!("跳过 rustfmt 实检:环境未安装 rustfmt({error})");
                return None;
            }
            Err(error) => panic!("rustfmt 启动失败:{error}"),
        };
        if output.status.success() {
            return None;
        }
        Some(format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}
