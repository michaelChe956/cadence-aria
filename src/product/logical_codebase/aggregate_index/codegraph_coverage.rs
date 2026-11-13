//! codegraph SDK 成员覆盖验收（只读）。
//!
//! 背景：`codegraph files --json` 的全量文件清单随聚合根规模线性膨胀
//! （现场 3.1 GB 库 ⇒ ~230 MB 载荷），1 MiB/64 MiB 抓取上限都装不下 ⇒
//! 验收必失败。而验收真正需要的只是「每个成员目录有没有被索引的文件」。
//! 本模块走 codegraph 库 API（`CodeGraph.open` + `hasFilesUnder`）直接
//! 回答是/否，载荷几百字节；顺带取索引健康字段（stale/pending/state/
//! fileCount）供上层做健康闸（warning，不阻断激活）。
//!
//! SDK 是全局安装的 npm 包，不能被普通脚本直接 import ⇒ 按绝对路径
//! `import(<npm-sdk.js>)`（CJS interop：`mod.default ?? mod` 取
//! `CodeGraph`）。Node 用 codegraph 平台包自带的二进制（不要求用户装
//! Node）；PATH 上的 node 需 ≥ 22.5 才回退采用。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    TokioBoundedCommandRunner,
};
use crate::product::json_store::validate_relative_id;

use super::AggregateIndexError;

/// codegraph 的 npm 包名（`<prefix>/lib/node_modules/<pkg>` 布局用）。
pub const CODEGRAPH_NPM_PACKAGE: &str = "@colbymchenry/codegraph";

/// PATH 回退 node 的最低版本（自带 node 无此门槛）。
pub const MIN_NODE_VERSION: (u64, u64, u64) = (22, 5, 0);

/// helper 执行超时（SDK 只做目录覆盖判定 + 健康字段，实测 ~8 ms/open）。
const HELPER_TIMEOUT_SECS: u64 = 60;
/// `npm root -g` 探针超时——口径同 provider_installation 的
/// `CommandNpmGlobalPrefixProbe`（10s / 4096 B）。
const NPM_PROBE_TIMEOUT_SECS: u64 = 10;
const PROBE_OUTPUT_LIMIT_BYTES: usize = 4096;
const HELPER_OUTPUT_LIMIT_BYTES: usize = 1024 * 1024;
const HELPER_STDERR_LIMIT_BYTES: usize = 64 * 1024;

/// SDK 覆盖验收的结果：每成员覆盖判定 + 原样健康字段。
#[derive(Debug, Clone, PartialEq)]
pub struct CoverageOutcome {
    /// 成员名 ⇒ `hasFilesUnder(member)`（目录边界精确，优于 CLI 裸前缀）。
    pub member_covered: BTreeMap<String, bool>,
    /// `getStats()` 原样（取不到则 None）。
    pub stats: Option<Value>,
    /// 健康字段聚合（state/pending/stale；取不到的字段被省略）。
    pub index: Option<Value>,
}

/// 聚合成员覆盖探针——生产实现走 codegraph SDK（[`CodeGraphCoverage`]），
/// DI/测试可注入桩。
pub trait AggregateCoverageProbe: Send + Sync {
    fn coverage(
        &self,
        root: &Path,
        member_names: &[String],
    ) -> Result<CoverageOutcome, AggregateIndexError>;
}

/// DI/测试构造器的默认探针：恒报 SDK 不可用 ⇒ 验收回退既有 CLI files
/// 路径（与今天的行为一致）。生产入口 `AggregateIndexOperation::new`
/// 装配 [`CodeGraphCoverage`] 的环境解析探针。
pub struct CliFallbackCoverageProbe;

impl AggregateCoverageProbe for CliFallbackCoverageProbe {
    fn coverage(
        &self,
        _root: &Path,
        _member_names: &[String],
    ) -> Result<CoverageOutcome, AggregateIndexError> {
        Err(AggregateIndexError::Failed {
            code: "codegraph_sdk_not_found",
            message: "coverage probe not configured for this operation; accepting via the \
                      CLI files fallback"
                .to_string(),
        })
    }
}

/// SDK（npm-sdk.js）与 node 可执行的解析候选；测试可整体注入替换，
/// 生产由 [`CoveragePaths::resolve_from_environment`] 从环境解析。
#[derive(Clone, Default)]
pub struct CoveragePaths {
    /// 候选1：我们安装前缀下 npm 标准布局的 `npm-sdk.js` 绝对路径。
    pub install_prefix_sdk: Option<PathBuf>,
    /// 候选2：`npm root -g` 探针结果（全局 node_modules 根）。
    pub npm_global_root: Option<PathBuf>,
    /// 候选3：codegraph 可执行路径（真实路径反推包根）。
    pub codegraph_executable: Option<PathBuf>,
    /// node 候选1：codegraph 平台包自带的 node 可执行（按序取第一个
    /// 存在且可执行者；空 ⇒ 从解析出的 SDK 包根下扫描
    /// `node_modules/@colbymchenry/codegraph-*/node`）。
    pub bundled_nodes: Vec<PathBuf>,
    /// node 候选2：PATH 上的 node 可执行（需版本 ≥ 22.5 才采用）。
    pub path_node: Option<PathBuf>,
}

impl CoveragePaths {
    /// 生产解析：读进程环境与 PATH；`npm root -g` 探针按需惰性执行
    /// （只有候选1/3 都落空时才会花这笔子进程开销）。
    pub fn resolve_from_environment(runner: &Arc<dyn BoundedCommandRunner>) -> Self {
        let _ = runner;
        todo!("green: install-prefix / npm root -g / codegraph 反推候选解析")
    }
}

/// SDK 覆盖验收执行器。
pub struct CodeGraphCoverage {
    runner: Arc<dyn BoundedCommandRunner>,
    /// 候选注入（测试 seam）；`None` = 生产环境解析。
    paths: Option<CoveragePaths>,
    /// helper 脚本源（生产 `include_str!` 内嵌；测试可注入）。
    helper_source: String,
    /// helper 落盘目录（`None` = 系统 temp；测试注入以断言清理）。
    scratch_dir: Option<PathBuf>,
}

impl CodeGraphCoverage {
    /// 生产装配：Tokio 运行器的环境解析探针。
    pub fn production() -> Self {
        Self {
            runner: Arc::new(TokioBoundedCommandRunner),
            paths: None,
            helper_source: include_str!("codegraph_coverage_helper.mjs").to_string(),
            scratch_dir: None,
        }
    }

    /// 测试/DI 装配：固定候选与 helper 落盘目录。
    pub fn with_paths(
        runner: Arc<dyn BoundedCommandRunner>,
        paths: CoveragePaths,
        scratch_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            runner,
            paths: Some(paths),
            helper_source: include_str!("codegraph_coverage_helper.mjs").to_string(),
            scratch_dir,
        }
    }
}

impl AggregateCoverageProbe for CodeGraphCoverage {
    fn coverage(
        &self,
        root: &Path,
        member_names: &[String],
    ) -> Result<CoverageOutcome, AggregateIndexError> {
        let _ = (root, member_names);
        todo!("green: 写临时 helper → node 执行 → 解析 JSON → CoverageOutcome")
    }
}

/// 按序取第一个真实存在的 `npm-sdk.js`：
/// 1. 安装前缀 npm 标准布局；2. `npm root -g`；3. codegraph 可执行反推。
pub fn resolve_sdk_path(paths: &CoveragePaths) -> Option<PathBuf> {
    let _ = paths;
    todo!("green: SDK 路径解析")
}

/// node 解析：自带 node（存在且可执行）优先；PATH node 需 `--version`
/// 实测 ≥ 22.5；都不可用 ⇒ `codegraph_node_unavailable`。
pub fn resolve_node_executable(
    paths: &CoveragePaths,
    sdk_path: &Path,
    runner: &Arc<dyn BoundedCommandRunner>,
) -> Result<PathBuf, AggregateIndexError> {
    let _ = (paths, sdk_path, runner);
    todo!("green: node 解析与版本闸")
}

/// 解析 `node --version` 输出（`v22.5.0`、`v24.16.0-nightly.1` 等）；
/// 无法解析 ⇒ None（fail-closed，不采用）。
pub fn parse_node_version(output: &str) -> Option<(u64, u64, u64)> {
    let _ = output;
    todo!("green: node 版本解析")
}

/// 以同步方式跑一次有界命令（镜像 `codegraph_cli` 的线程 + 单线程
/// runtime 阻塞口径）。
fn run_bounded_sync(
    runner: &Arc<dyn BoundedCommandRunner>,
    request: BoundedCommandRequest,
) -> Result<BoundedCommandResult, BoundedCommandError> {
    let _ = (runner, request);
    todo!("green: 同步有界命令执行")
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// 覆盖验收桩 runner：按可执行名/argv 分发
    /// `--version` 与 helper 调用（`--members`），其余 panic。
    #[derive(Default)]
    struct FakeCoverageRunner {
        state: Mutex<FakeCoverageState>,
    }

    #[derive(Default)]
    struct FakeCoverageState {
        node_version: Option<String>,
        helper_stdout: Option<String>,
        helper_exit_code: Option<i32>,
        requests: Vec<BoundedCommandRequest>,
    }

    impl FakeCoverageRunner {
        fn node_version(&self, version: &str) {
            self.state.lock().unwrap().node_version = Some(version.to_string());
        }

        fn helper_stdout(&self, stdout: &str) {
            self.state.lock().unwrap().helper_stdout = Some(stdout.to_string());
        }

        fn helper_exit_code(&self, code: i32) {
            self.state.lock().unwrap().helper_exit_code = Some(code);
        }

        fn requests(&self) -> Vec<BoundedCommandRequest> {
            std::mem::take(&mut self.state.lock().unwrap().requests)
        }
    }

    #[async_trait::async_trait]
    impl BoundedCommandRunner for FakeCoverageRunner {
        async fn run(
            &self,
            request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            let mut state = self.state.lock().unwrap();
            state.requests.push(request.clone());
            let stdout = if request.argv.len() == 1 && request.argv[0] == "--version" {
                state.node_version.clone().unwrap_or_else(|| {
                    panic!("unexpected node --version probe without scripted version")
                })
            } else if request.argv.iter().any(|argument| argument == "--members") {
                state.helper_stdout.clone().unwrap_or_else(|| {
                    panic!("unexpected helper invocation without scripted stdout")
                })
            } else {
                panic!("unexpected coverage runner argv: {:?}", request.argv)
            };
            let exit_code = if request.argv.iter().any(|argument| argument == "--members") {
                state.helper_exit_code.unwrap_or(0)
            } else {
                0
            };
            Ok(BoundedCommandResult {
                exit_code: Some(exit_code),
                stdout,
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                duration_ms: 1,
            })
        }
    }

    struct CoverageFixture {
        _temp: tempfile::TempDir,
        runner: Arc<FakeCoverageRunner>,
        sdk: PathBuf,
        node: PathBuf,
        scratch: PathBuf,
        root: PathBuf,
    }

    fn executable_file(path: &Path) -> PathBuf {
        std::fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).unwrap();
        }
        path.to_path_buf()
    }

    fn coverage_fixture() -> CoverageFixture {
        let temp = tempfile::tempdir().unwrap();
        let sdk_dir = temp.path().join("codegraph-pkg");
        std::fs::create_dir_all(&sdk_dir).unwrap();
        let sdk = sdk_dir.join("npm-sdk.js");
        std::fs::write(&sdk, "module.exports = {};").unwrap();
        let node = executable_file(&temp.path().join("bundled-node"));
        let scratch = temp.path().join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        let root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(&root).unwrap();
        CoverageFixture {
            _temp: temp,
            runner: Arc::new(FakeCoverageRunner::default()),
            sdk,
            node,
            scratch,
            root,
        }
    }

    impl CoverageFixture {
        fn paths(&self) -> CoveragePaths {
            CoveragePaths {
                install_prefix_sdk: Some(self.sdk.clone()),
                npm_global_root: None,
                codegraph_executable: None,
                bundled_nodes: vec![self.node.clone()],
                path_node: None,
            }
        }

        fn coverage(&self) -> CodeGraphCoverage {
            CodeGraphCoverage::with_paths(
                self.runner.clone(),
                self.paths(),
                Some(self.scratch.clone()),
            )
        }

        fn members(&self) -> Vec<String> {
            vec!["api".to_string(), "web".to_string()]
        }

        fn runner_handle(&self) -> Arc<dyn BoundedCommandRunner> {
            self.runner.clone()
        }

        fn helper_stdout(&self, stdout: &str) {
            self.runner.helper_stdout(stdout);
        }

        fn helper_exit_code(&self, code: i32) {
            self.runner.helper_exit_code(code);
        }

        fn node_version(&self, version: &str) {
            self.runner.node_version(version);
        }
    }

    #[test]
    fn coverage_reports_member_covered_from_sdk() {
        let fixture = coverage_fixture();
        fixture.helper_stdout(
            r#"{"ok":true,"members":{"api":true,"web":false},"stats":{"fileCount":2},"index":{"state":"ready","pending":0,"stale":false}}"#,
        );

        let outcome = fixture
            .coverage()
            .coverage(&fixture.root, &fixture.members())
            .unwrap();

        assert_eq!(
            outcome.member_covered,
            BTreeMap::from([("api".to_string(), true), ("web".to_string(), false),])
        );
        assert_eq!(
            outcome
                .stats
                .as_ref()
                .and_then(|stats| stats.get("fileCount")),
            Some(&serde_json::json!(2))
        );
        assert_eq!(
            outcome.index.as_ref().and_then(|index| index.get("state")),
            Some(&serde_json::json!("ready"))
        );
        // helper 写到注入的 scratch 目录且执行后被清理。
        let requests = fixture.runner.requests();
        let helper_invocation = requests
            .iter()
            .find(|request| request.argv.iter().any(|argument| argument == "--members"))
            .expect("helper must be invoked through the injected node");
        assert_eq!(helper_invocation.executable, fixture.node);
        let helper_path = PathBuf::from(&helper_invocation.argv[0]);
        assert!(
            helper_path.starts_with(&fixture.scratch),
            "helper must be written under the injected scratch dir: {}",
            helper_path.display()
        );
        assert!(
            !helper_path.exists(),
            "helper temp file must be cleaned up after execution"
        );
    }

    #[test]
    fn coverage_errors_sdk_not_found_when_no_candidate_exists() {
        let fixture = coverage_fixture();
        let coverage = CodeGraphCoverage::with_paths(
            fixture.runner.clone(),
            CoveragePaths::default(),
            Some(fixture.scratch.clone()),
        );

        let error = coverage
            .coverage(&fixture.root, &fixture.members())
            .unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "codegraph_sdk_not_found"),
            "expected codegraph_sdk_not_found, got: {error}"
        );
    }

    #[test]
    fn coverage_maps_helper_failure_to_coverage_failed() {
        let fixture = coverage_fixture();
        fixture.helper_stdout(
            r#"{"ok":false,"code":"open_failed","message":"CodeGraph not initialized"}"#,
        );
        fixture.helper_exit_code(1);

        let error = fixture
            .coverage()
            .coverage(&fixture.root, &fixture.members())
            .unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "codegraph_coverage_failed"
                    && error.to_string().contains("open_failed")
                    && error.to_string().contains("CodeGraph not initialized")),
            "expected codegraph_coverage_failed carrying helper code/message, got: {error}"
        );
    }

    #[test]
    fn coverage_maps_sdk_incompatible_helper_report() {
        let fixture = coverage_fixture();
        fixture.helper_stdout(
            r#"{"ok":false,"code":"sdk_incompatible","message":"no hasFilesUnder"}"#,
        );
        fixture.helper_exit_code(1);

        let error = fixture
            .coverage()
            .coverage(&fixture.root, &fixture.members())
            .unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "codegraph_sdk_incompatible"),
            "expected codegraph_sdk_incompatible, got: {error}"
        );
    }

    #[test]
    fn coverage_resolves_bundled_node_before_path_node() {
        let fixture = coverage_fixture();
        fixture.runner.node_version("v99.0.0");
        let path_node = executable_file(&fixture._temp.path().join("path-node"));
        let paths = CoveragePaths {
            path_node: Some(path_node),
            bundled_nodes: vec![fixture.node.clone()],
            ..fixture.paths()
        };

        let resolved = resolve_node_executable(&paths, &fixture.sdk, &fixture.runner_handle()).unwrap();

        assert_eq!(resolved, fixture.node);
        assert!(
            fixture.runner.requests().is_empty(),
            "bundled node must be adopted without a --version probe"
        );
    }

    #[test]
    fn coverage_rejects_path_node_below_minimum_version() {
        let fixture = coverage_fixture();
        let path_node = executable_file(&fixture._temp.path().join("path-node"));
        let paths = CoveragePaths {
            path_node: Some(path_node.clone()),
            bundled_nodes: Vec::new(),
            ..fixture.paths()
        };

        // 低于 22.5 ⇒ 不采用。
        fixture.runner.node_version("v18.20.0");
        let error = resolve_node_executable(&paths, &fixture.sdk, &fixture.runner_handle()).unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "codegraph_node_unavailable"),
            "node 18 must be rejected, got: {error}"
        );

        // 恰为 22.5 ⇒ 采用。
        fixture.runner.node_version("v22.5.0");
        let resolved = resolve_node_executable(&paths, &fixture.sdk, &fixture.runner_handle()).unwrap();
        assert_eq!(resolved, path_node);

        // 版本不可解析 ⇒ fail-closed 不采用。
        fixture.runner.node_version("version unknown");
        let error = resolve_node_executable(&paths, &fixture.sdk, &fixture.runner_handle()).unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "codegraph_node_unavailable"),
            "unparseable node version must be rejected, got: {error}"
        );
    }

    #[test]
    fn coverage_reports_node_unavailable_when_no_node() {
        let fixture = coverage_fixture();
        let paths = CoveragePaths {
            bundled_nodes: Vec::new(),
            path_node: None,
            ..fixture.paths()
        };

        let error = resolve_node_executable(&paths, &fixture.sdk, &fixture.runner_handle()).unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, message, .. }
                if *code == "codegraph_node_unavailable"
                    && message.contains("22.5")),
            "node-unavailable message must state the Node.js >= 22.5 requirement, got: {error}"
        );
    }

    #[test]
    fn parse_node_version_accepts_v_prefixed_triplets() {
        assert_eq!(parse_node_version("v22.5.0\n"), Some((22, 5, 0)));
        assert_eq!(parse_node_version("v24.16.0"), Some((24, 16, 0)));
        assert_eq!(
            parse_node_version("v24.17.0-nightly.1\n"),
            Some((24, 17, 0))
        );
        assert_eq!(parse_node_version("garbage"), None);
        assert_eq!(parse_node_version(""), None);
    }

    #[test]
    fn resolve_sdk_path_prefers_first_existing_candidate() {
        let fixture = coverage_fixture();
        // 注：fixture 的 install_prefix_sdk 真实存在 ⇒ 候选1 命中。
        let paths = fixture.paths();
        assert_eq!(
            resolve_sdk_path(&paths).as_deref(),
            Some(fixture.sdk.as_path())
        );

        // 候选1 缺席 ⇒ npm root -g 布局命中。
        let global_root = fixture._temp.path().join("global-node-modules");
        let global_sdk_dir = global_root.join("@colbymchenry/codegraph");
        std::fs::create_dir_all(&global_sdk_dir).unwrap();
        let global_sdk = global_sdk_dir.join("npm-sdk.js");
        std::fs::write(&global_sdk, "module.exports = {};").unwrap();
        let paths = CoveragePaths {
            install_prefix_sdk: Some(fixture._temp.path().join("missing/npm-sdk.js")),
            npm_global_root: Some(global_root),
            ..CoveragePaths::default()
        };
        assert_eq!(
            resolve_sdk_path(&paths).as_deref(),
            Some(global_sdk.as_path())
        );

        // 候选1/2 缺席 ⇒ codegraph 可执行真实路径反推命中。
        let prefix = fixture._temp.path().join("derived-prefix");
        let bin_dir = prefix
            .join("lib")
            .join("node_modules")
            .join("@colbymchenry")
            .join("codegraph")
            .join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let codegraph_exe = bin_dir.join("codegraph");
        std::fs::write(&codegraph_exe, "#!/bin/sh\nexit 0\n").unwrap();
        let paths = CoveragePaths {
            install_prefix_sdk: None,
            npm_global_root: Some(fixture._temp.path().join("missing-node-modules")),
            codegraph_executable: Some(codegraph_exe.clone()),
            ..CoveragePaths::default()
        };
        let expected = prefix
            .join("lib")
            .join("node_modules")
            .join("@colbymchenry")
            .join("codegraph")
            .join("npm-sdk.js");
        std::fs::write(&expected, "module.exports = {};").unwrap();
        assert_eq!(
            resolve_sdk_path(&paths).as_deref(),
            Some(expected.as_path())
        );
    }
}
