//! Provider 写边界(write boundary)的纯 DTO owner(Task 1a,REQ-LCG-03/07)。
//!
//! `ProviderBoundaryPlan` 是 validated launch 持有的不可伪造边界意图:
//! 进程 cwd(canonical LC root)、唯一可写 target(Coding;read-only action
//! 为 `None`)与受保护根(`.git`/`.aria` 等元数据位置)。`ProviderBoundary
//! Evidence` 是真实 probe(6c)产出并经三方一致性校验(2d)后导入 durable
//! capability 的证据记录。本文件只拥有 DTO 形状;`ProcessManager::
//! spawn_with_boundary` 等 launcher 行为归 Task 6a,probe 归 Task 6c。
//!
//! 字段私有+`pub(crate)` 构造:plan/evidence 只能由 gateway/probe 装配,
//! 调用方不能自造;读取经 pub getter(只读投影)。

use std::path::{Path, PathBuf};

use crate::product::models::ProviderName;

/// 边界模式:read-only action 无任何可写 target;Coding 恰一个 canonical
/// target 可写(target-only)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderBoundaryMode {
    /// Planning/Review:root、成员与元数据均不可写。
    ReadOnly,
    /// Coding:唯一 writable root 恰为 target worktree。
    TargetWriteOnly,
}

/// validated launch 持有的不可伪造边界计划(Task 1a 冻结形状)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderBoundaryPlan {
    mode: ProviderBoundaryMode,
    /// 进程 cwd(canonical `provider_context_root`)。target 独立解析,
    /// 不得替代 cwd(Global Constraints)。
    working_directory: PathBuf,
    /// 唯一可写 target(Coding);read-only action 为 `None`。
    target_root: Option<PathBuf>,
    /// 受保护根/路径(`.git`、`.aria` 等元数据位置;root 与非 target 的
    /// 保护由 launcher/负向探针执行)。
    protected_roots: Vec<PathBuf>,
}

impl ProviderBoundaryPlan {
    /// crate 内构造(gateway/probe 装配;调用方不能自造)。
    pub(crate) fn new(
        mode: ProviderBoundaryMode,
        working_directory: PathBuf,
        target_root: Option<PathBuf>,
        protected_roots: Vec<PathBuf>,
    ) -> Self {
        Self {
            mode,
            working_directory,
            target_root,
            protected_roots,
        }
    }

    pub fn mode(&self) -> ProviderBoundaryMode {
        self.mode
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    pub fn target_root(&self) -> Option<&Path> {
        self.target_root.as_deref()
    }

    pub fn protected_roots(&self) -> &[PathBuf] {
        &self.protected_roots
    }
}

/// 真实 boundary probe 的证据记录(Task 1a 冻结形状;6c 产出、2d 导入)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderBoundaryEvidence {
    provider: ProviderName,
    /// 被探测 provider 的 exact CLI version(与 capability record 比对)。
    exact_version: String,
    boundary_mode: ProviderBoundaryMode,
    /// 当次 probe 比对的 projection digest(与 session projection、
    /// capability action row 三方一致才可导入 Confirmed)。
    projection_digest: String,
    /// probe 工件引用(快照/日志的 durable 引用)。
    artifact_ref: String,
    probed_at: String,
}

impl ProviderBoundaryEvidence {
    /// crate 内构造(6c probe 产出;调用方不能自造)。
    pub(crate) fn new(
        provider: ProviderName,
        exact_version: String,
        boundary_mode: ProviderBoundaryMode,
        projection_digest: String,
        artifact_ref: String,
        probed_at: String,
    ) -> Self {
        Self {
            provider,
            exact_version,
            boundary_mode,
            projection_digest,
            artifact_ref,
            probed_at,
        }
    }

    pub fn provider(&self) -> &ProviderName {
        &self.provider
    }

    pub fn exact_version(&self) -> &str {
        &self.exact_version
    }

    pub fn boundary_mode(&self) -> ProviderBoundaryMode {
        self.boundary_mode
    }

    pub fn projection_digest(&self) -> &str {
        &self.projection_digest
    }

    pub fn artifact_ref(&self) -> &str {
        &self.artifact_ref
    }

    pub fn probed_at(&self) -> &str {
        &self.probed_at
    }
}

/// 边界执行/校验错误(稳定判别码 + 上下文)。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderBoundaryError {
    /// 当前 OS/运行时不支持可控写边界,不得退回无隔离 spawn。
    #[error("provider_boundary_unsupported: {0}")]
    Unsupported(String),
    /// plan 非法(cwd/target/protected roots 缺失或不一致)。
    #[error("provider_boundary_invalid_plan: {0}")]
    InvalidPlan(String),
    /// 真实 probe 失败(越界写成功/保护位置可写等)。
    #[error("provider_boundary_probe_failed: {0}")]
    ProbeFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DTO 形状锁:构造字段与 getter 往返一致(read-only 无 target)。
    #[test]
    fn provider_boundary_plan_round_trips_read_only_shape() {
        let plan = ProviderBoundaryPlan::new(
            ProviderBoundaryMode::ReadOnly,
            PathBuf::from("/lc-root"),
            None,
            vec![
                PathBuf::from("/lc-root/.git"),
                PathBuf::from("/lc-root/.aria"),
            ],
        );
        assert_eq!(plan.mode(), ProviderBoundaryMode::ReadOnly);
        assert_eq!(plan.working_directory(), std::path::Path::new("/lc-root"));
        assert_eq!(plan.target_root(), None);
        assert_eq!(plan.protected_roots().len(), 2);
    }

    /// target-only 形状:唯一 target 可写,cwd 与 target 分离。
    #[test]
    fn provider_boundary_plan_round_trips_target_write_only_shape() {
        let plan = ProviderBoundaryPlan::new(
            ProviderBoundaryMode::TargetWriteOnly,
            PathBuf::from("/lc-root"),
            Some(PathBuf::from("/work/api/.worktrees/issue_1")),
            vec![],
        );
        assert_eq!(plan.mode(), ProviderBoundaryMode::TargetWriteOnly);
        assert_eq!(
            plan.target_root(),
            Some(std::path::Path::new("/work/api/.worktrees/issue_1"))
        );
    }

    /// evidence getter 往返(2d 三方一致性校验消费的固定字段)。
    #[test]
    fn provider_boundary_evidence_exposes_probe_fields() {
        let evidence = ProviderBoundaryEvidence::new(
            ProviderName::ClaudeCode,
            "2.1.0".to_string(),
            ProviderBoundaryMode::TargetWriteOnly,
            "sha256:abc".to_string(),
            "probe://run/1".to_string(),
            "2026-10-03T00:00:00Z".to_string(),
        );
        assert_eq!(evidence.provider(), &ProviderName::ClaudeCode);
        assert_eq!(evidence.exact_version(), "2.1.0");
        assert_eq!(
            evidence.boundary_mode(),
            ProviderBoundaryMode::TargetWriteOnly
        );
        assert_eq!(evidence.projection_digest(), "sha256:abc");
        assert_eq!(evidence.artifact_ref(), "probe://run/1");
        assert_eq!(evidence.probed_at(), "2026-10-03T00:00:00Z");
    }
}

#[cfg(test)]
mod launcher_tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::{
        ProviderBoundaryLauncher, ProviderBoundaryMode, ProviderBoundaryPlan, frozen_git_dir_binds,
        provider_runtime_writable_roots,
    };
    use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;

    fn argv_text(argv: &[OsString]) -> Vec<String> {
        argv.iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect()
    }

    fn coding_plan(root: &Path, target: &Path) -> ProviderBoundaryPlan {
        ProviderBoundaryPlan::new(
            ProviderBoundaryMode::TargetWriteOnly,
            root.to_path_buf(),
            Some(target.to_path_buf()),
            Vec::new(),
        )
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .expect("run git fixture");
        assert!(status.success(), "git fixture {args:?} failed");
    }

    /// launcher 核心:只读 host、冻结 target 原路径、保持网络与既有配置发现,
    /// 隔离 temp;不照搬 Kimi terminal 的 --unshare-net 与 /tmp/work cwd 改写。
    #[test]
    fn lcg_t06_boundary_argv_keeps_network_root_paths_and_readonly_host() {
        let base = tempdir().expect("base dir");
        let root = base.path().join("lc-root");
        let target = base.path().join("target");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::create_dir_all(&target).expect("target");
        let plan = coding_plan(&root, &target);
        let launcher = ProviderBoundaryLauncher::from_bwrap(Some(PathBuf::from("/usr/sbin/bwrap")));

        let argv = launcher
            .build_boundary_argv("sh", &["-c", "true"], &root, &BTreeMap::new(), &plan)
            .expect("boundary argv");
        let text = argv_text(&argv);

        // 保持网络:provider API/MCP 需要,不使用 --unshare-net。
        assert!(!text.iter().any(|arg| arg == "--unshare-net"));
        // 只读 host 必须最早挂载(bwrap argv 顺序语义,后续 bind 才能遮蔽)。
        assert_eq!(
            &text[0..3],
            ["--ro-bind".to_string(), "/".to_string(), "/".to_string()]
        );
        // 冻结 target 以原路径 rw 挂载,且是唯一 rw bind。
        let target_text = target.to_string_lossy().into_owned();
        assert!(
            text.windows(3).any(|window| {
                window == ["--bind".to_string(), target_text.clone(), target_text]
            })
        );
        assert_eq!(
            text.iter().filter(|arg| *arg == "--bind").count(),
            1,
            "target 是 Coding 唯一宽写 bind:{text:?}"
        );
        // cwd 保持 root 原路径:不照搬 Kimi terminal 的 /tmp/work cwd 改写。
        assert!(!text.iter().any(|arg| arg.contains("/tmp/work")));
        let root_text = root.to_string_lossy().into_owned();
        assert!(
            text.windows(2)
                .any(|window| window == ["--chdir".to_string(), root_text])
        );
        // 既有配置发现保持:不 --clearenv,环境经继承+overlay(非 bwrap --setenv)。
        assert!(!text.iter().any(|arg| arg == "--clearenv"));
        assert!(!text.iter().any(|arg| arg == "--setenv"));
        // 隔离 temp:每沙箱私有 /tmp。
        assert!(
            text.windows(2)
                .any(|window| window == ["--tmpfs".to_string(), "/tmp".to_string()])
        );
        // payload 命令收尾。
        assert_eq!(text.last().map(String::as_str), Some("sh"));
    }

    /// read-only action:无任何 rw bind,root/成员/元数据全部落在只读挂载面。
    #[test]
    fn lcg_t06_boundary_readonly_argv_has_no_writable_binds() {
        let base = tempdir().expect("base dir");
        let root = base.path().join("lc-root");
        std::fs::create_dir_all(&root).expect("root");
        let plan = ProviderBoundaryPlan::new(
            ProviderBoundaryMode::ReadOnly,
            root.clone(),
            None,
            Vec::new(),
        );
        let launcher = ProviderBoundaryLauncher::from_bwrap(Some(PathBuf::from("/usr/sbin/bwrap")));

        let argv = launcher
            .build_boundary_argv("sh", &["-c", "true"], &root, &BTreeMap::new(), &plan)
            .expect("readonly boundary argv");
        let text = argv_text(&argv);

        assert!(!text.iter().any(|arg| arg == "--bind"));
        let root_text = root.to_string_lossy().into_owned();
        assert!(
            text.windows(3).any(|window| {
                window == ["--ro-bind".to_string(), root_text.clone(), root_text]
            })
        );
    }

    /// 各 provider 自然 session/runtime 可写集合:HOME 下窄面目录,用途固定;
    /// 不得整 HOME、整 root 或 MCP 目录宽写。
    #[test]
    fn lcg_t06_provider_runtime_writable_roots_stay_narrow() {
        let base = tempdir().expect("base dir");
        let home = base.path().join("home");
        for dir in [".claude", ".codex", ".pi", ".kimi-code", ".config"] {
            std::fs::create_dir_all(home.join(dir)).expect("runtime dir");
        }
        std::fs::write(home.join(".claude.json"), "{}").expect("claude state file");
        let mut env = BTreeMap::new();
        env.insert("HOME".to_string(), home.to_string_lossy().into_owned());

        let roots = provider_runtime_writable_roots(&env);
        let expected: Vec<PathBuf> = [".claude", ".claude.json", ".codex", ".pi", ".kimi-code"]
            .iter()
            .map(|dir| home.join(dir))
            .collect();
        assert_eq!(roots, expected);
        assert!(!roots.iter().any(|path| path == &home));
        assert!(!roots.iter().any(|path| path.ends_with(".config")));

        // 不存在的自然目录不预创建、不扩大写面。
        std::fs::remove_dir_all(home.join(".pi")).expect("remove .pi");
        let narrowed = provider_runtime_writable_roots(&env);
        assert!(!narrowed.iter().any(|path| path.ends_with(".pi")));
    }

    /// git identity 授权链:plain repo 不加额外 bind;linked worktree 冻结
    /// 常见 git 目录(bind 面=common dir 祖先),冻结面持久化在 host 领土,
    /// 后续轮次即使 `.git` 指针被改写也信任冻结结果。
    #[test]
    fn lcg_t06_frozen_git_dir_binds_follow_identity_chain() {
        let base = tempdir().expect("base dir");

        let plain = base.path().join("plain");
        std::fs::create_dir_all(&plain).expect("plain repo");
        git(&plain, &["init", "-q"]);
        git(
            &plain,
            &[
                "-c",
                "user.name=aria",
                "-c",
                "user.email=aria@aria",
                "commit",
                "--allow-empty",
                "-m",
                "init",
            ],
        );
        assert_eq!(frozen_git_dir_binds(&plain), Vec::<PathBuf>::new());

        let repo = base.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        git(&repo, &["init", "-q"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=aria",
                "-c",
                "user.email=aria@aria",
                "commit",
                "--allow-empty",
                "-m",
                "init",
            ],
        );
        let target = base.path().join("target-wt");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                target.to_string_lossy().as_ref(),
                "-b",
                "probe-wt",
            ],
        );

        let binds = frozen_git_dir_binds(&target);
        let common = std::fs::canonicalize(repo.join(".git")).expect("canonical git common");
        assert_eq!(binds, vec![common]);

        // 冻结面持久化在 target 父目录 host 领土(.provider-session-cache)。
        let cache_root = target
            .parent()
            .expect("target parent")
            .join(".provider-session-cache");
        let persisted = std::fs::read_dir(cache_root)
            .expect("provider session cache")
            .filter_map(|entry| entry.ok())
            .any(|entry| entry.path().join("writable_git_paths.json").exists());
        assert!(
            persisted,
            "frozen git face must persist outside the writable target"
        );

        // 指针被改写后(round N+1)仍信任冻结面,不重新解析到伪造 git dir。
        std::fs::write(target.join(".git"), "gitdir: /definitely/not/real\n")
            .expect("swap pointer");
        let refrozen = frozen_git_dir_binds(&target);
        assert_eq!(
            refrozen, binds,
            "frozen face must survive a coder-swapped .git pointer"
        );
    }

    /// 能力视角:缺 bwrap/user namespace 与「可用但未经真实 probe」都保持
    /// Unknown;launcher 自身永不签发 Confirmed(6c 真实正负探针才可)。
    #[test]
    fn lcg_t06_boundary_state_stays_unknown_with_or_without_sandbox() {
        let unavailable = ProviderBoundaryLauncher::from_bwrap(None);
        assert!(!unavailable.is_available());
        assert_eq!(
            unavailable.write_boundary_state(),
            ProviderCapabilityEvidence::Unknown
        );

        let available =
            ProviderBoundaryLauncher::from_bwrap(Some(PathBuf::from("/usr/sbin/bwrap")));
        assert!(available.is_available());
        assert_eq!(
            available.write_boundary_state(),
            ProviderCapabilityEvidence::Unknown
        );
    }
}
