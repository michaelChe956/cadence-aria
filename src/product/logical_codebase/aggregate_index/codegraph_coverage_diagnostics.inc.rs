// 覆盖验收的可诊断性事实（纯数据 + 纯函数 + node 候选解析），由
// `codegraph_coverage.rs` 以 `include!` 引入（同一模块作用域）。
//
// 拆出理由：`codegraph_coverage.rs` 触到 large_file_guard 的 1200 行硬线；
// 这些事实只服务于「日志说清走了哪条路径、node 从哪来、覆盖了多少成员」，
// 与 SDK 探测主流程可分离。日志只是这些事实的消费者。

/// 覆盖判定的成员汇总（日志 `member_count` / `covered_count` /
/// `uncovered_members` 的事实来源）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageSummary {
    pub member_count: usize,
    pub covered_count: usize,
    /// 未覆盖成员名（顺序稳定：按成员名排序）。
    pub uncovered: Vec<String>,
}

/// 汇总每成员覆盖判定（纯函数；日志消费者）。
pub fn summarize_coverage(member_covered: &BTreeMap<String, bool>) -> CoverageSummary {
    CoverageSummary {
        member_count: member_covered.len(),
        covered_count: member_covered.values().filter(|covered| **covered).count(),
        uncovered: member_covered
            .iter()
            .filter(|(_, covered)| !**covered)
            .map(|(member, _)| member.clone())
            .collect(),
    }
}

/// 索引健康字段（日志 `index_state` / `index_pending` / `index_stale` 的
/// 事实来源；取不到的字段保持 `None`，不伪造默认值）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoverageHealth {
    pub state: Option<String>,
    pub pending: Option<u64>,
    pub stale: Option<bool>,
}

/// 从 SDK 返回的 `index` 原样字段里抽取健康字段（纯函数）。
pub fn coverage_health(index: Option<&Value>) -> CoverageHealth {
    let Some(index) = index else {
        return CoverageHealth::default();
    };
    CoverageHealth {
        state: index
            .get("state")
            .and_then(Value::as_str)
            .map(str::to_string),
        pending: index.get("pending").and_then(Value::as_u64),
        stale: index.get("stale").and_then(Value::as_bool),
    }
}

/// 解析出来的 node 的**来源**（日志 `node_source`，稳定可 grep；客户机
/// darwin-arm64 上「自带 / PATH / 缓存」一眼可分）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeSource {
    /// codegraph 平台包自带的 node（nested / hoisted 布局）。
    Bundled,
    /// codegraph 自愈缓存 `<缓存根>/bundles/<target>-<ver>/node`。
    Cache,
    /// PATH 上的 node（需 ≥ 22.5）。
    Path,
}

impl NodeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bundled => "bundled",
            Self::Cache => "cache",
            Self::Path => "path",
        }
    }
}

/// node 可执行 + 来源（日志与用例共享的结构化事实）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeResolution {
    pub path: PathBuf,
    pub source: NodeSource,
}

/// node 解析：自带 node（存在且可执行）优先；PATH node 需 `--version`
/// 实测 ≥ 22.5；都不可用 ⇒ `codegraph_node_unavailable`。
pub fn resolve_node_with_source(
    paths: &CoveragePaths,
    sdk_path: &Path,
    runner: &Arc<dyn BoundedCommandRunner>,
) -> Result<NodeResolution, AggregateIndexError> {
    let mut bundled = if paths.bundled_nodes.is_empty() {
        bundled_node_resolutions(sdk_path, paths.home_dir.as_deref())
    } else {
        // 注入的候选（DI / 测试）没有布局信息 ⇒ 统一标 bundled。
        paths
            .bundled_nodes
            .iter()
            .cloned()
            .map(|path| NodeResolution {
                path,
                source: NodeSource::Bundled,
            })
            .collect()
    };
    if let Some(resolution) = bundled
        .drain(..)
        .find(|candidate| is_executable_file(&candidate.path))
    {
        return Ok(resolution);
    }
    if let Some(path_node) = paths
        .path_node
        .as_deref()
        .filter(|candidate| is_executable_file(candidate))
        && path_node_meets_minimum(path_node, runner)
    {
        return Ok(NodeResolution {
            path: path_node.to_path_buf(),
            source: NodeSource::Path,
        });
    }
    Err(AggregateIndexError::Failed {
        code: "codegraph_node_unavailable",
        message: "需要 Node.js ≥ 22.5 运行 codegraph SDK 覆盖验收：未找到 codegraph 平台\
                 包自带的 node，PATH 上的 node 缺失或版本低于 22.5；可安装 Node.js \
                 ≥ 22.5，或重装 codegraph（npm install -g @colbymchenry/codegraph）"
            .to_string(),
    })
}

/// 同 [`resolve_node_with_source`]，只要路径（既有调用口径不变）。
pub fn resolve_node_executable(
    paths: &CoveragePaths,
    sdk_path: &Path,
    runner: &Arc<dyn BoundedCommandRunner>,
) -> Result<PathBuf, AggregateIndexError> {
    resolve_node_with_source(paths, sdk_path, runner).map(|resolution| resolution.path)
}

/// codegraph 自带 node 的全部候选（按采纳优先级）：
/// ① **nested** —— `<主包>/node_modules/@colbymchenry/codegraph-<target>/node`；
/// ② **hoisted** —— 与主包**同级**的 `<node_modules>/@colbymchenry/codegraph-<target>/node`
///    （实测 1.6.2 真实布局：`libtest/node_modules/@colbymchenry/` 下
///    `codegraph` 与 `codegraph-linux-x64` 并列，且 node 在平台包里）；
/// ③ **自愈缓存** —— `<缓存根>/bundles/<target>-<version>/node`
///    （`npm-shim.js` 下载缓存，同平台多版本时版本高者优先；缓存根见
///    [`codegraph_cache_root`]）。
///
/// 目录名按**当前平台** `<platform>-<arch>` 匹配（不得硬编码单一平台）。
///
/// 测试 seam：只取路径口径（生产走 [`bundled_node_resolutions`] 以保留来源）。
#[cfg(test)]
fn bundled_node_candidates(sdk_path: &Path, home_dir: Option<&Path>) -> Vec<PathBuf> {
    bundled_node_resolutions(sdk_path, home_dir)
        .into_iter()
        .map(|resolution| resolution.path)
        .collect()
}

/// 同 [`bundled_node_candidates`]，但每个候选带来源（生产 `resolve_node_with_source`
/// 用；`CODEGRAPH_INSTALL_DIR` / `HOME` 口径与 [`codegraph_cache_root`] 一致）。
fn bundled_node_resolutions(sdk_path: &Path, home_dir: Option<&Path>) -> Vec<NodeResolution> {
    let cache_root = codegraph_cache_root(
        home_dir,
        std::env::var_os("CODEGRAPH_INSTALL_DIR").map(PathBuf::from),
    );
    bundled_node_candidates_with_source(sdk_path, cache_root)
}

/// 自愈缓存根（`bundles` 的父目录）：`CODEGRAPH_INSTALL_DIR` 优先，否则
/// `<home>/.codegraph`。
///
/// 与上游同口径（`scripts/npm-shim.js:113`
/// `path.join(process.env.CODEGRAPH_INSTALL_DIR || path.join(os.homedir(), '.codegraph'), 'bundles')`；
/// `scripts/npm-sdk.js:68` 同）：只硬编码 `~/.codegraph` 会漏掉自定义安装目录
/// 里的自愈 bundle。空值视为未设置（宿主把变量设成空串时不改变默认根）。
fn codegraph_cache_root(home_dir: Option<&Path>, install_dir: Option<PathBuf>) -> Option<PathBuf> {
    install_dir
        .filter(|dir| !dir.as_os_str().is_empty())
        .or_else(|| home_dir.map(|home| home.join(".codegraph")))
}

/// 同 [`bundled_node_candidates`]，但自愈缓存根**显式**给出（测试 seam，
/// 避免依赖进程环境）。
#[cfg(test)]
fn bundled_node_candidates_with_cache_root(
    sdk_path: &Path,
    cache_root: Option<PathBuf>,
) -> Vec<PathBuf> {
    bundled_node_candidates_with_source(sdk_path, cache_root)
        .into_iter()
        .map(|resolution| resolution.path)
        .collect()
}

/// 同 [`bundled_node_candidates_with_cache_root`]，但每个候选带**来源**
/// （自带 / 自愈缓存）⇒ 日志能说明 node 是从哪儿来的。
pub(crate) fn bundled_node_candidates_with_source(
    sdk_path: &Path,
    cache_root: Option<PathBuf>,
) -> Vec<NodeResolution> {
    let Some(package_root) = sdk_path.parent() else {
        return Vec::new();
    };
    let target = codegraph_platform_target();
    let mut candidates: Vec<NodeResolution> = Vec::new();
    // ① 主包内 node_modules（nested 布局）。
    candidates.extend(
        platform_package_nodes(
            &package_root.join("node_modules").join(CODEGRAPH_NPM_SCOPE),
            &target,
        )
        .into_iter()
        .map(|path| NodeResolution {
            path,
            source: NodeSource::Bundled,
        }),
    );
    // ② 每一层上层 `node_modules/@colbymchenry`（hoisted 布局 —— 真实资产）。
    for ancestor in package_root.ancestors().skip(1) {
        if ancestor
            .file_name()
            .is_some_and(|name| name == "node_modules")
        {
            candidates.extend(
                platform_package_nodes(&ancestor.join(CODEGRAPH_NPM_SCOPE), &target)
                    .into_iter()
                    .map(|path| NodeResolution {
                        path,
                        source: NodeSource::Bundled,
                    }),
            );
        }
    }
    // ③ 自愈缓存。
    if let Some(root) = cache_root {
        candidates.extend(cached_bundle_nodes(&root, &target).into_iter().map(|path| {
            NodeResolution {
                path,
                source: NodeSource::Cache,
            }
        }));
    }
    candidates
}
