// ---------------------------------------------------------------------------
// Gitignore-aware snapshot filtering (Bug2 2026-10-16)
// ---------------------------------------------------------------------------
//
// 报告:cadence/notes/2026-10-16_bug-filesystem-audit-不感知gitignore.md。
// root recipe 全量快照/预算门此前按磁盘实际字节累计,node_modules/target
// 等被忽略路径撞线导致正常前端/Java 仓 fail-closed。本文件实现快照侧的
// gitignore 语义(基本模式子集):
//
// - 解析审计根及每个被遍历目录的 `.gitignore`(嵌套支持——聚合根下的
//   成员仓各自生效),以及 git 元数据排除面 `.git/info/exclude`
//   (`dir/.git` 为目录直接取;为 worktree gitfile 时解析 `gitdir:` 指向)。
// - 模式语义:裸名字(不含 `/`)按 basename 在作用域下任意深度匹配;
//   含 `/`(含前导 `/`)锚定作用域目录按完整相对路径逐段匹配;尾缀
//   `/` 仅目录;`!` 否定(同层最后命中者胜,被忽略目录不下降故无法
//   重新包含其内部——与 git 一致);`*`/`?`/`[...]` 段内通配(不跨段),
//   `**` 整段匹配零或多段。
// - 层间优先级:更深的 .gitignore 覆盖更浅;同目录内 `.gitignore` 覆盖
//   `.git/info/exclude`(与 git 一致)。
//
// 已知简化(与 git 的差异,均为非阻断边界):不支持 `core.excludesFile`
// 全局排除;尾部空白裁剪未支持 `\ ` 转义;`foo/**` 会连带匹配 `foo`
// 本身(git 仅匹配其内部);`.gitignore`/exclude 读取失败(权限/编码)
// 按无规则降级(git 同样尽力而为),不影响 fail-closed 审计面。
// 被忽略路径不计入预算、不进快照、其变更不可观测——这是产品语义:
// 用户以 gitignore 声明的非版本面不属于审计观测面。

/// 单条 gitignore 模式(解析后的规范形态)。
#[derive(Debug, Clone)]
struct GitignorePattern {
    negated: bool,
    /// 尾缀 `/`:仅匹配目录。
    dir_only: bool,
    /// 不含 `/` 的裸名字模式:对作用域下任意深度的同名条目生效;
    /// 否则锚定作用域目录,按完整相对路径逐段匹配。
    basename_scope: bool,
    segments: Vec<String>,
}

impl GitignorePattern {
    /// `scoped_path`:相对模式所在目录的路径(正斜杠);`is_dir` 为该
    /// 条目自身是否目录(symlink 按非目录处理,与 git 不跟随 symlink 一致)。
    fn matches(&self, scoped_path: &str, is_dir: bool) -> bool {
        if self.dir_only && !is_dir {
            return false;
        }
        if self.basename_scope {
            let name = scoped_path.rsplit('/').next().unwrap_or(scoped_path);
            return glob_segment_match(&self.segments[0], name);
        }
        let path_segments: Vec<&str> = scoped_path.split('/').collect();
        segments_match(&self.segments, &path_segments)
    }
}

/// 解析一行 gitignore 规则;空行/注释/退化模式返回 None。
fn parse_gitignore_line(line: &str) -> Option<GitignorePattern> {
    let mut text = line.trim_end_matches([' ', '\t', '\r']);
    if text.is_empty() || text.starts_with('#') {
        return None;
    }
    let negated = text.starts_with('!');
    if negated {
        text = &text[1..];
    } else if let Some(unescaped) = text.strip_prefix("\\!") {
        text = unescaped;
    }
    if let Some(unescaped) = text.strip_prefix("\\#") {
        text = unescaped;
    }
    if text.is_empty() {
        return None;
    }
    let dir_only = text.ends_with('/');
    if dir_only {
        text = text.trim_end_matches('/');
    }
    let anchored = text.contains('/');
    let text = text.strip_prefix('/').unwrap_or(text);
    let segments: Vec<String> = text
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect();
    if segments.is_empty() {
        return None;
    }
    Some(GitignorePattern {
        negated,
        dir_only,
        basename_scope: !anchored,
        segments,
    })
}

/// 逐段匹配:`**` 段吸收零或多段;其余段按段内通配严格一一对应。
fn segments_match(pattern: &[String], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((first, rest)) => {
            if first == "**" {
                (0..=path.len()).any(|skip| segments_match(rest, &path[skip..]))
            } else {
                !path.is_empty()
                    && glob_segment_match(first, path[0])
                    && segments_match(rest, &path[1..])
            }
        }
    }
}

/// 段内 glob(`*`/`?`/`[...]`/`\` 转义;`*` 不跨段,段即路径分量)。
fn glob_segment_match(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    glob_chars(&pattern, &name)
}

fn glob_chars(pattern: &[char], name: &[char]) -> bool {
    let Some(&head) = pattern.first() else {
        return name.is_empty();
    };
    match head {
        '*' => {
            let mut rest = 1;
            while rest < pattern.len() && pattern[rest] == '*' {
                rest += 1;
            }
            (0..=name.len()).any(|take| glob_chars(&pattern[rest..], &name[take..]))
        }
        '?' => !name.is_empty() && glob_chars(&pattern[1..], &name[1..]),
        '[' => {
            let Some(candidate) = name.first() else {
                return false;
            };
            match char_class_matches(pattern, 0, *candidate) {
                Some((hit, next)) => hit && glob_chars(&pattern[next..], &name[1..]),
                // 未闭合 `[` 按字面量处理。
                None => *candidate == '[' && glob_chars(&pattern[1..], &name[1..]),
            }
        }
        '\\' if pattern.len() > 1 => {
            !name.is_empty() && pattern[1] == name[0] && glob_chars(&pattern[2..], &name[1..])
        }
        literal => !name.is_empty() && literal == name[0] && glob_chars(&pattern[1..], &name[1..]),
    }
}

/// 字符类 `[abc]`/`[a-z]`/`[!a-z]`(可在类内以 `\` 转义)。返回
/// (是否命中, 消费后的模式下标);未闭合返回 None。
fn char_class_matches(pattern: &[char], start: usize, candidate: char) -> Option<(bool, usize)> {
    let mut index = start + 1;
    let mut negate = false;
    if index < pattern.len() && (pattern[index] == '!' || pattern[index] == '^') {
        negate = true;
        index += 1;
    }
    let mut matched = false;
    let mut first = true;
    while index < pattern.len() {
        if pattern[index] == ']' && !first {
            return Some((matched != negate, index + 1));
        }
        first = false;
        let low = if pattern[index] == '\\' && index + 1 < pattern.len() {
            index += 1;
            pattern[index]
        } else {
            pattern[index]
        };
        if index + 2 < pattern.len() && pattern[index + 1] == '-' && pattern[index + 2] != ']' {
            let high = pattern[index + 2];
            if low <= candidate && candidate <= high {
                matched = true;
            }
            index += 3;
        } else {
            if candidate == low {
                matched = true;
            }
            index += 1;
        }
    }
    None
}

/// 逐行解析 ignore 文件(UTF-8 有损、读取失败按无规则降级)。
fn parse_ignore_file(path: &Path, sink: &mut Vec<GitignorePattern>) {
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    for line in String::from_utf8_lossy(&bytes).lines() {
        if let Some(pattern) = parse_gitignore_line(line) {
            sink.push(pattern);
        }
    }
}

/// 一个作用域目录(其 `.gitignore` 或 `.git/info/exclude`)的全部规则。
struct GitignoreLevel {
    /// 作用域目录相对审计根的正斜杠路径;审计根自身为空串。
    scope: String,
    patterns: Vec<GitignorePattern>,
}

impl GitignoreLevel {
    /// 相对审计根的 `relative` 是否被本层规则裁决;不在作用域之下或本层
    /// 无命中返回 None(交由更浅层继续裁决)。
    fn verdict(&self, relative: &str, is_dir: bool) -> Option<bool> {
        let scoped: &str = if self.scope.is_empty() {
            relative
        } else {
            let prefix = format!("{}/", self.scope);
            relative.strip_prefix(prefix.as_str())?
        };
        let mut verdict = None;
        for pattern in &self.patterns {
            if pattern.matches(scoped, is_dir) {
                verdict = Some(!pattern.negated);
            }
        }
        verdict
    }
}

/// 遍历栈:进入目录时压入该目录的忽略层,离开时弹出。层间后入栈优先
/// (更深的 .gitignore 覆盖更浅;同目录内先入栈 exclude、后入栈
/// `.gitignore`,使后者优先)。
#[derive(Default)]
struct AuditIgnoreStack {
    levels: Vec<GitignoreLevel>,
}

impl AuditIgnoreStack {
    /// 进入目录:收集 `.git/info/exclude`(若为 git 仓)与 `.gitignore`,
    /// 返回压入的层数(供 [`Self::leave_dir`] 弹出)。
    fn enter_dir(&mut self, root: &Path, dir: &Path) -> Result<usize, ProductStoreError> {
        let scope = relative_audit_path(root, dir)?;
        let mut pushed = 0;
        if let Some(exclude) = git_exclude_path(dir) {
            let mut patterns = Vec::new();
            parse_ignore_file(&exclude, &mut patterns);
            if !patterns.is_empty() {
                self.levels.push(GitignoreLevel {
                    scope: scope.clone(),
                    patterns,
                });
                pushed += 1;
            }
        }
        let mut patterns = Vec::new();
        parse_ignore_file(&dir.join(".gitignore"), &mut patterns);
        if !patterns.is_empty() {
            self.levels.push(GitignoreLevel { scope, patterns });
            pushed += 1;
        }
        Ok(pushed)
    }

    fn leave_dir(&mut self, pushed: usize) {
        self.levels.truncate(self.levels.len() - pushed);
    }

    /// 相对审计根的路径是否被任一层忽略(深层优先裁决)。
    fn is_ignored(&self, relative: &str, is_dir: bool) -> bool {
        self.levels
            .iter()
            .rev()
            .find_map(|level| level.verdict(relative, is_dir))
            .unwrap_or(false)
    }
}

/// `dir` 的 `.git/info/exclude` 候选路径:`dir/.git` 为目录直接取;为
/// worktree gitfile(`gitdir: <path>`)时解析指向(相对路径以 `dir` 为
/// 基点词法归一);非 git 目录返回 None。
fn git_exclude_path(dir: &Path) -> Option<PathBuf> {
    let dot_git = dir.join(".git");
    let metadata = std::fs::metadata(&dot_git).ok()?;
    if metadata.is_dir() {
        return Some(dot_git.join("info").join("exclude"));
    }
    if !metadata.is_file() {
        return None;
    }
    let text = std::fs::read_to_string(&dot_git).ok()?;
    let gitdir = text.lines().next()?.strip_prefix("gitdir:")?.trim();
    let path = if Path::new(gitdir).is_absolute() {
        PathBuf::from(gitdir)
    } else {
        normalize_lexical(dir.join(gitdir))
    };
    Some(path.join("info").join("exclude"))
}

/// 词法归一(`.` 折叠、`..` 弹出,不触盘)。
fn normalize_lexical(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}
