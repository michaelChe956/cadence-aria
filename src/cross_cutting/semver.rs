//! 通用 semver 精确比较（REQ-PCI-04 / tasks 11.3）。
//!
//! `major.minor.patch` 三段数字 + 可选预发布后缀（`1.5.0-rc.1`）；预发布
//! 版本视为**低于**对应正式版本。只做语义比较——不做字符串前缀或浮点
//! 近似（spec 明令禁止）。解析宽容前缀噪声（`v`、厂商名），与
//! `provider_health` 的 `parse_version_token` 范式同口径。

use std::cmp::Ordering;

/// 解析后的语义版本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// `1.5.0-rc.1` 的 `rc.1` 部分；`None` = 正式版（高于任何预发布）。
    pub pre_release: Option<String>,
}

/// 取首个以数字开头的空白分隔词（容忍 `v` 前缀与厂商前缀噪声）。
fn version_token(text: &str) -> Option<&str> {
    text.split_whitespace().find_map(|token| {
        let token = token.trim_start_matches(['v', 'V']);
        token
            .starts_with(|character: char| character.is_ascii_digit())
            .then_some(token)
    })
}
pub fn parse_semver(text: &str) -> Option<SemVer> {
    let token = version_token(text)?;
    let mut parts = [0u64; 3];
    let mut chars = token.chars().peekable();
    let mut filled = 0usize;
    while filled < 3 {
        let mut number = String::new();
        while let Some(&character) = chars.peek() {
            if character.is_ascii_digit() {
                number.push(character);
                chars.next();
            } else {
                break;
            }
        }
        if number.is_empty() {
            break;
        }
        parts[filled] = number.parse().ok()?;
        filled += 1;
        if matches!(chars.peek(), Some('.')) {
            chars.next();
        } else {
            break;
        }
    }
    if filled == 0 {
        return None;
    }
    let rest: String = chars.collect();
    // `-rc.1` 预发布后缀保留；`+build` 构建元数据剥离。
    let suffix = rest.split('+').next().unwrap_or(&rest);
    let pre_release = suffix
        .strip_prefix('-')
        .filter(|release| !release.is_empty())
        .map(str::to_string);
    Some(SemVer {
        major: parts[0],
        minor: parts[1],
        patch: parts[2],
        pre_release,
    })
}

/// 精确比较两个版本串。不可解析的一侧视为「无法判定」→ `Equal`
///（调用方把 `Less` 之外的结果一律当「无新版」处理，不会误报更新）。
pub fn compare_semver(a: &str, b: &str) -> Ordering {
    let (Some(a), Some(b)) = (parse_semver(a), parse_semver(b)) else {
        // 任一侧不可解析 → 无法判定；调用方把非 Less 一律当「无新版」。
        return Ordering::Equal;
    };
    let core = (a.major, a.minor, a.patch).cmp(&(b.major, b.minor, b.patch));
    if core != Ordering::Equal {
        return core;
    }
    match (&a.pre_release, &b.pre_release) {
        (None, None) => Ordering::Equal,
        // 预发布 < 正式（REQ-PCI-04）。
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (Some(left), Some(right)) => left.cmp(right),
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::{compare_semver, parse_semver};

    #[test]
    fn semver_numeric_compare_prefers_major_over_minor() {
        assert_eq!(compare_semver("1.5.0", "0.85.0"), Ordering::Greater);
        assert_eq!(compare_semver("0.85.0", "1.5.0"), Ordering::Less);
    }

    #[test]
    fn semver_prerelease_is_lower_than_release() {
        assert_eq!(compare_semver("1.5.0-rc.1", "1.5.0"), Ordering::Less);
        assert_eq!(compare_semver("1.5.0", "1.5.0-rc.1"), Ordering::Greater);
    }
    #[test]
    fn semver_equal_versions_compare_equal() {
        assert_eq!(compare_semver("1.5.0", "1.5.0"), Ordering::Equal);
    }

    #[test]
    fn semver_parses_noisy_cli_output() {
        let version = parse_semver("codex-cli 0.85.3+debug").expect("version token");
        assert_eq!((version.major, version.minor, version.patch), (0, 85, 3));
        assert_eq!(version.pre_release, None, "build 元数据不构成预发布");

        let release = parse_semver("v2.0.1").expect("v 前缀版本");
        assert_eq!((release.major, release.minor, release.patch), (2, 0, 1));

        let pre = parse_semver("kimi 1.5.0-rc.1").expect("预发布版本");
        assert_eq!(pre.pre_release.as_deref(), Some("rc.1"));
    }

    #[test]
    fn semver_numeric_core_not_lexicographic() {
        // 1.10.0 > 1.9.0：数字比较，不是字符串比较。
        assert_eq!(compare_semver("1.10.0", "1.9.0"), Ordering::Greater);
        // 两个预发布之间仍有全序（按后缀字典序）。
        assert_eq!(compare_semver("1.5.0-rc.1", "1.5.0-rc.2"), Ordering::Less);
    }
}
