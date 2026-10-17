//! 批2 Task 3(C1.2 / D S1/S2 / B 环境合同 / REQ-ENV-HOST-01 / R12):
//! 统一白名单环境构造器与 trust 发现一致性。
//!
//! 一处 `env_clear` 构造器统一四家子进程环境(§环境精确合同,Plan
//! `2026-10-17_实施计划_批量2_Bug45合并包_v1.0`):
//!
//! 1. 顺序固定:`env_clear` → 允许的宿主基础变量(精确继承表)→ 受信配置
//!    批准的工具链/连接/Provider 密钥(精确批准表)→ 平台冻结字段
//!    (`HOME`=受信 owner home、`CODEX_HOME`=`owner_home/.codex`)。
//!    后层覆盖前层;模型/ACP 请求中的 env 不参加合并,平台入站认证令牌
//!    永远不允许。
//! 2. 默认精确继承:只复制 [`PROVIDER_ENV_BASE_INHERIT_KEYS`] 中**存在**的
//!    键,不存在则不伪造;没有 `*_TOKEN`/`*_KEY`/`LC_*`/`XDG_*` 等 glob
//!    继承。
//! 3. `approved` 只由受信配置解析器按§环境键表构造:本函数对 `approved`
//!    的**每个键**做精确表校验,表外键一律 fail-closed 拒绝
//!    (`provider_environment_approved_key_rejected`),模型/任意 caller 的
//!    map 冒充在构造期即被拒绝。
//! 4. `HOME` 由启动时受信 owner home 解析并冻结,禁止改为 worktree/
//!    private_home;`XDG_*`/`TMPDIR` 只可经精确批准注入,无配置按工具原生
//!    默认。
//! 5. `CODEX_HOME` 不做网络参数化、不默认继承任意另一个目录:平台冻结为
//!    `CodexTrustAdapter::for_home(owner_home)` 的 `owner_home/.codex`
//!    (与 trust adapter 同一函数、同一来源);`inherited`/`approved` 携带
//!    **另一个** `CODEX_HOME` 时显式拒绝并给配置发现不一致诊断。Kimi 同理
//!    由 `KimiTrustAdapter::for_home(owner_home)` 的
//!    `owner_home/.kimi-code/workspace-trust` 同源冻结发现引用。
//! 6. 产物 [`FrozenProviderEnvironment`] 含最终键值、policy version(复用
//!    T2 [`PROVIDER_ENV_POLICY_VERSION`] 同一版本源)、**键/来源**摘要
//!    (值不入摘要)与 config 发现引用;secret 值不进摘要/`Debug`/audit。
//!    spawn 用最终 `PATH`,无 `PATH` 显式缺工具,不回退未冻结宿主 `PATH`
//!    (见 `process_manager::path_env` 的 T3 收口)。
//!
//! 传输与消费:最终键值经既有 env map 传输(`StreamingProviderInput::
//! env_vars` 等)进入 `ProcessManager` spawn(env_clear 后只注入该 map);
//! 同步 CLI 旁路(`cli_adapter`)消费同一产物,不隐式全继承。四家 adapter
//! 的实际接线(engine/gateway 把本产物放进 env_vars)归 T11/T12/T20 验收;
//! 各 adapter 禁止复制白名单——键表只在本模块定义。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::provider_execution::{
    FrozenProviderExecution, PROVIDER_ENV_POLICY_VERSION,
};
use crate::product::logical_codebase::provider_trust_adapters::{
    CodexTrustAdapter, KimiTrustAdapter,
};
/// 默认精确继承的宿主基础变量(§环境精确合同:不存在则不伪造;无任何
/// glob 继承)。表本身即合同——增删键属环境策略变更,必须先升
/// [`PROVIDER_ENV_POLICY_VERSION`]。
pub const PROVIDER_ENV_BASE_INHERIT_KEYS: &[&str] = &[
    "PATH",
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_CTYPE",
    "LC_COLLATE",
    "LC_MESSAGES",
    "LC_NUMERIC",
    "LC_TIME",
    "TERM",
    "COLORTERM",
    "TZ",
    "USER",
    "LOGNAME",
    "SHELL",
];

/// 受信配置显式批准后才注入的工具链精确键。
pub const PROVIDER_ENV_TOOLCHAIN_KEYS: &[&str] = &[
    "JAVA_HOME",
    "JDK_HOME",
    "MAVEN_HOME",
    "M2_HOME",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "GOPATH",
    "GOBIN",
    "NVM_DIR",
];

/// 能向子进程注入执行参数的键(`MAVEN_OPTS`/`JAVA_TOOL_OPTIONS`/
/// `NODE_OPTIONS`/`RUSTFLAGS`):不默认继承,须显式配置批准并进入环境策略
/// 摘要(键名入摘要,值不入)。
pub const PROVIDER_ENV_EXECUTION_OPTION_KEYS: &[&str] = &[
    "MAVEN_OPTS",
    "JAVA_TOOL_OPTIONS",
    "NODE_OPTIONS",
    "RUSTFLAGS",
];

/// 包仓库/网络/容器连接精确键(含代理小写同名)。
pub const PROVIDER_ENV_PACKAGE_REGISTRY_KEYS: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    "NPM_CONFIG_REGISTRY",
    "NPM_CONFIG_USERCONFIG",
    "DOCKER_HOST",
    "DOCKER_CONTEXT",
    "CONTAINER_HOST",
    "CONTAINER_CONNECTION",
    "REGISTRY_AUTH_FILE",
    "DOCKER_CONFIG",
];

/// 目录类精确键:只可来自受信启动配置/宿主明确值;无配置按工具原生默认,
/// 不按它们的值推导可删除目录。
pub const PROVIDER_ENV_DIRECTORY_KEYS: &[&str] = &[
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
    "XDG_RUNTIME_DIR",
    "TMPDIR",
];

/// Provider 密钥/镜像认证精确键(受信配置按精确键与适用 Provider/角色
/// 登记;`approved` 本身已由受信配置解析器按适用 provider 过滤,本模块只
/// 做精确键校验)。当前仅收平台自身登记的 codegraph MCP 令牌;后续受信
/// 配置解析器(T6/T11/T12 接线)新增精确键时扩本表并升环境策略版本。
pub const PROVIDER_ENV_PROVIDER_SECRET_KEYS: &[&str] = &["CODEGRAPH_TOKEN"];

/// 键/来源摘要的 schema 前缀(与 [`PROVIDER_ENV_POLICY_VERSION`] 同源)。
pub const PROVIDER_ENV_KEYSET_SCHEMA: &str = "provider-environment-keyset/1";

/// 平台冻结字段:owner home(受信启动配置解析)。
pub const PROVIDER_ENV_HOME_KEY: &str = "HOME";
/// 平台冻结字段:Codex CLI 配置发现根(与 `CodexTrustAdapter::for_home`
/// 同一来源;另一取值显式拒绝)。
pub const PROVIDER_ENV_CODEX_HOME_KEY: &str = "CODEX_HOME";

/// 环境键的注入来源(优先级固定:后层覆盖前层)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvKeySource {
    /// 精确继承表里从宿主环境复制的键。
    BaseInherited,
    /// 受信配置按§环境键表显式批准注入的键。
    ApprovedInjection,
    /// 平台冻结字段(`HOME`/`CODEX_HOME`)。
    FrozenPlatform,
}

impl EnvKeySource {
    pub fn as_str(&self) -> &'static str {
        match self {
            EnvKeySource::BaseInherited => "inherited",
            EnvKeySource::ApprovedInjection => "approved",
            EnvKeySource::FrozenPlatform => "frozen",
        }
    }
}

/// 统一白名单环境产物:最终键值 + policy version + 键/来源摘要 + config
/// 发现引用。字段 private、不实现 `Deserialize`——serde 输入不能铸造环境;
/// 唯一构造口是 crate 内受信构造器 [`build_provider_environment`]。
///
/// secret 值不进 `Debug`/摘要/audit:手动 `Debug` 只输出键名/来源/摘要;
/// [`FrozenProviderEnvironment::env`] 仅供 spawn 传输消费,禁止进入日志、
/// 指纹或证据。
pub struct FrozenProviderEnvironment {
    policy_version: u32,
    env: BTreeMap<String, String>,
    key_sources: BTreeMap<String, EnvKeySource>,
    keyset_digest: String,
    owner_home: PathBuf,
    codex_config_ref: PathBuf,
    kimi_trust_ref: PathBuf,
    env_policy_digest: String,
}

impl std::fmt::Debug for FrozenProviderEnvironment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // T3 合同:值(可能含 secret)一律不进 Debug,只输出键名与来源。
        let keys = self
            .key_sources
            .iter()
            .map(|(key, source)| format!("{key}:{}", source.as_str()))
            .collect::<Vec<_>>()
            .join(",");
        formatter
            .debug_struct("FrozenProviderEnvironment")
            .field("policy_version", &self.policy_version)
            .field("keys", &keys)
            .field("keyset_digest", &self.keyset_digest)
            .field("owner_home", &self.owner_home)
            .field("codex_config_ref", &self.codex_config_ref)
            .field("kimi_trust_ref", &self.kimi_trust_ref)
            .finish_non_exhaustive()
    }
}

impl FrozenProviderEnvironment {
    /// 环境策略版本(与 T2 执行合同同一冻结版本源)。
    pub fn policy_version(&self) -> u32 {
        self.policy_version
    }

    /// 最终键值(spawn 传输专用:进入 `StreamingProviderInput::env_vars`
    /// 等 env map 通道;禁止进入日志/指纹/证据)。
    pub fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    /// 键/来源摘要(sha256,长度前缀域哈希;**不含任何值**——同键集不同
    /// secret 值摘要不变,值漂移由 resume 指纹的执行合同域负责)。
    pub fn keyset_digest(&self) -> &str {
        &self.keyset_digest
    }

    /// 冻结的受信 owner home(三环境档同源;`HOME` 即取此值)。
    pub fn owner_home(&self) -> &Path {
        &self.owner_home
    }

    /// Codex CLI 配置发现引用:与 `CodexTrustAdapter::for_home(owner_home)`
    /// 的 `owner_home/.codex/config.toml` 完全相同(同一函数产出)。
    pub fn codex_config_discovery_ref(&self) -> &Path {
        &self.codex_config_ref
    }

    /// Kimi workspace-trust 发现引用:与 `KimiTrustAdapter::for_home(
    /// owner_home)` 的 `owner_home/.kimi-code/workspace-trust` 完全相同。
    pub fn kimi_workspace_trust_ref(&self) -> &Path {
        &self.kimi_trust_ref
    }

    /// 消费方执行合同的环境策略摘要(透传 T2 冻结事实,审计关联用)。
    pub fn env_policy_digest(&self) -> &str {
        &self.env_policy_digest
    }

    /// audit 面非敏感事实(键集合与来源、摘要、发现引用;**不含任何值**)。
    pub fn audit_facts(&self) -> BTreeMap<String, String> {
        let mut facts = BTreeMap::new();
        facts.insert(
            "policy_version".to_string(),
            self.policy_version.to_string(),
        );
        facts.insert("keyset_digest".to_string(), self.keyset_digest.clone());
        facts.insert(
            "keys".to_string(),
            self.key_sources
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(","),
        );
        facts.insert(
            "key_sources".to_string(),
            self.key_sources
                .iter()
                .map(|(key, source)| format!("{key}:{}", source.as_str()))
                .collect::<Vec<_>>()
                .join(","),
        );
        facts.insert(
            "owner_home".to_string(),
            self.owner_home.display().to_string(),
        );
        facts.insert(
            "codex_config_ref".to_string(),
            self.codex_config_ref.display().to_string(),
        );
        facts.insert(
            "kimi_workspace_trust_ref".to_string(),
            self.kimi_trust_ref.display().to_string(),
        );
        facts.insert(
            "env_policy_digest".to_string(),
            self.env_policy_digest.clone(),
        );
        facts
    }
}

/// crate 内受信构造器:按§环境精确合同从冻结合同 + 受信启动配置产出统一
/// 白名单环境。
///
/// - `execution`:T2 冻结执行合同(环境策略版本必须与
///   [`PROVIDER_ENV_POLICY_VERSION`] 一致,漂移拒绝);
/// - `owner_home`:启动时受信 owner home(必须绝对路径;`HOME` 冻结取此
///   值,禁止改为 worktree/private_home);
/// - `inherited`:宿主环境快照(只按 [`PROVIDER_ENV_BASE_INHERIT_KEYS`]
///   精确复制,表外键——含平台入站令牌、`NODE_OPTIONS`、任意 `*_TOKEN`
///   ——一律不进);
/// - `approved`:受信配置解析器按§环境键表构造的精确批准表;表外键
///   fail-closed 拒绝,不能由模型/任意 caller map 冒充。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn build_provider_environment(
    execution: &FrozenProviderExecution,
    owner_home: &Path,
    inherited: &BTreeMap<String, String>,
    approved: &BTreeMap<String, String>,
) -> Result<FrozenProviderEnvironment, ProviderAdapterError> {
    // T2 冻结版本源一致:执行合同的环境策略版本必须与白名单合同同一
    // 版本冻结,漂移即拒绝(不静默续用旧语义)。
    if execution.env_policy_version() != PROVIDER_ENV_POLICY_VERSION {
        return Err(ProviderAdapterError::provider_unavailable(format!(
            "provider_environment_policy_version_mismatch: execution={} frozen={}",
            execution.env_policy_version(),
            PROVIDER_ENV_POLICY_VERSION
        )));
    }
    if !owner_home.is_absolute() {
        return Err(ProviderAdapterError::provider_unavailable(format!(
            "provider_environment_owner_home_not_absolute: {}",
            owner_home.display()
        )));
    }

    // trust 发现同源:Codex/Kimi 的配置发现引用与 trust adapter 完全
    // 相同(同一 for_home(owner_home) 构造,不另造路径拼接)。
    let codex_adapter = CodexTrustAdapter::for_home(owner_home);
    let kimi_adapter = KimiTrustAdapter::for_home(owner_home);
    let codex_config_ref = codex_adapter.config_path().to_path_buf();
    let codex_home = codex_config_ref
        .parent()
        .expect("codex config parent")
        .to_path_buf();
    let kimi_trust_ref = kimi_adapter.workspace_trust_root().to_path_buf();

    // CODEX_HOME 冻结发现一致性:另一个取值显式拒绝并给配置发现不一致
    // 诊断(不做网络参数化,不改 trust adapter 悄悄支持新 HOME)。
    for (carrier, map) in [("inherited", inherited), ("approved", approved)] {
        if let Some(requested) = map.get(PROVIDER_ENV_CODEX_HOME_KEY)
            && requested != &codex_home.display().to_string()
        {
            return Err(ProviderAdapterError::provider_unavailable(format!(
                "provider_environment_codex_home_conflict: {carrier} CODEX_HOME={requested} != frozen discovery {} (CodexTrustAdapter::for_home(owner_home)); 修复配置或保持默认发现,不得改 trust adapter 支持新 HOME",
                codex_config_ref.display()
            )));
        }
    }

    // approved 精确键表校验(受信配置解析器合同):表外键一律拒绝——
    // 模型/任意 caller 的 map 冒充在构造期 fail-closed。
    for key in approved.keys() {
        if !approved_environment_keys().contains(&key.as_str()) {
            return Err(ProviderAdapterError::provider_unavailable(format!(
                "provider_environment_approved_key_rejected: key={key} 不在§环境键表(工具链/执行参数/包仓库网络容器/目录/Provider密钥精确键);受信配置解析器只能按精确键构造 approved"
            )));
        }
    }

    // 层1(精确继承表):存在才复制,不存在不伪造——没有 glob 继承,
    // 平台入站令牌/UNRELATED_TOKEN/NODE_OPTIONS 等表外宿主键零透传。
    let mut env = BTreeMap::new();
    let mut key_sources = BTreeMap::new();
    for key in PROVIDER_ENV_BASE_INHERIT_KEYS {
        if let Some(value) = inherited.get(*key) {
            env.insert((*key).to_string(), value.clone());
            key_sources.insert((*key).to_string(), EnvKeySource::BaseInherited);
        }
    }
    // 层2(受信配置显式批准):后层覆盖前层。
    for (key, value) in approved {
        env.insert(key.clone(), value.clone());
        key_sources.insert(key.clone(), EnvKeySource::ApprovedInjection);
    }
    // 层3(平台冻结字段):HOME=受信 owner home(三环境档同源,禁止改为
    // worktree/private_home);CODEX_HOME=trust 发现同源目录。
    env.insert(
        PROVIDER_ENV_HOME_KEY.to_string(),
        owner_home.display().to_string(),
    );
    key_sources.insert(
        PROVIDER_ENV_HOME_KEY.to_string(),
        EnvKeySource::FrozenPlatform,
    );
    env.insert(
        PROVIDER_ENV_CODEX_HOME_KEY.to_string(),
        codex_home.display().to_string(),
    );
    key_sources.insert(
        PROVIDER_ENV_CODEX_HOME_KEY.to_string(),
        EnvKeySource::FrozenPlatform,
    );

    // 键/来源摘要(值不入摘要:同键集不同 secret 值摘要不变;值漂移由
    // resume 指纹的执行合同域负责)。
    let mut domains = vec![PROVIDER_ENV_POLICY_VERSION.to_be_bytes().to_vec()];
    for (key, source) in &key_sources {
        domains.push(key.as_bytes().to_vec());
        domains.push(source.as_str().as_bytes().to_vec());
    }
    let keyset_digest = hash_domains(PROVIDER_ENV_KEYSET_SCHEMA, &domains);

    Ok(FrozenProviderEnvironment {
        policy_version: PROVIDER_ENV_POLICY_VERSION,
        env,
        key_sources,
        keyset_digest,
        owner_home: owner_home.to_path_buf(),
        codex_config_ref,
        kimi_trust_ref,
        env_policy_digest: execution.env_policy_digest().to_string(),
    })
}

/// 受信 owner home 的环境解析(HOME/USERPROFILE,仅绝对路径):trust
/// adapter(`production()`)与受信启动配置的 owner_home 同源入口,禁止两处
/// 各自解析造成发现漂移。
pub fn resolve_trusted_owner_home_from_env() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
}

/// 受信配置解析器可批准的全部精确键(§环境键表单一来源):工具链、
/// 执行参数、包仓库/网络/容器、目录类与 Provider 密钥。扩表必须先升
/// 环境策略版本。
fn approved_environment_keys() -> Vec<&'static str> {
    [
        PROVIDER_ENV_TOOLCHAIN_KEYS,
        PROVIDER_ENV_EXECUTION_OPTION_KEYS,
        PROVIDER_ENV_PACKAGE_REGISTRY_KEYS,
        PROVIDER_ENV_DIRECTORY_KEYS,
        PROVIDER_ENV_PROVIDER_SECRET_KEYS,
    ]
    .concat()
}

/// 长度前缀域哈希(与 `provider_execution::hash_domains` 同型:8 字节大端
/// 长度前缀 + 域字节,schema 前置;无边界歧义)。
fn hash_domains(schema: &str, domains: &[Vec<u8>]) -> String {
    let mut hasher = Sha256::new();
    let mut domain = |part: &[u8]| {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    };
    domain(schema.as_bytes());
    for part in domains {
        domain(part);
    }
    format!("{schema}:sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::cross_cutting::provider_execution::{
        AuthorToolPolicy, ProviderExecutionProfile, TrustedProviderRole, TrustedRunIdentity,
        freeze_provider_execution,
    };
    use crate::protocol::contracts::AdapterRole;

    /// T3 测试 fixture 根(Plan GC5:测试临时物全部落 `/tmp/aria-bug45/`
    /// 的 Task 私有子目录)。
    fn t3_dir() -> tempfile::TempDir {
        let root = Path::new("/tmp/aria-bug45/t3");
        std::fs::create_dir_all(root).expect("create /tmp/aria-bug45/t3");
        tempfile::tempdir_in(root).expect("t3 fixture dir")
    }

    /// 子进程打印键名 fixture:`#!/bin/sh` 脚本输出排序后的 env 键名
    /// (只依赖 PATH 内 `/usr/bin` 工具,PATH 由冻结环境注入)。
    fn write_key_printer(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("print-env-keys");
        std::fs::write(&script, "#!/bin/sh\nenv | cut -d= -f1 | sort\n")
            .expect("write key printer fixture");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("chmod key printer");
        script
    }

    fn durable_run_record() -> crate::protocol::contracts::ProviderRunRecord {
        crate::protocol::contracts::ProviderRunRecord {
            provider_run_id: "run_t3_0001".to_string(),
            node_id: "N2".to_string(),
            provider_type: crate::protocol::contracts::ProviderType::ClaudeCode,
            runtime_role: crate::protocol::contracts::RuntimeRole::Orchestrator,
            adapter_role: AdapterRole::Executor,
            provider_capability_ref: "cap_claude_code_1_4_0".to_string(),
            adapter_compatibility_ref: "compat_1".to_string(),
            context_package_ref: "ctx_1".to_string(),
            adapter_input_ref: "ain_1".to_string(),
            adapter_output_ref: "aout_1".to_string(),
            raw_artifact_refs: Vec::new(),
            exit_code: Some(0),
            error_code: None,
            error_details: None,
            stdout_ref: None,
            stderr_ref: None,
            structured_output_ref: None,
            files_modified: Vec::new(),
            status: crate::protocol::contracts::ProviderRunStatus::Running,
            started_at: "2026-10-17T00:00:00Z".to_string(),
            completed_at: None,
            duration_ms: None,
            timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
            retry_count: 0,
            approval_policy: crate::protocol::contracts::ApprovalPolicy::OnRequest,
            sandbox_mode: crate::protocol::contracts::SandboxMode::FullAccess,
            constraint_check_ref: None,
            traceability_binding_refs: Vec::new(),
        }
    }

    fn coder_execution() -> FrozenProviderExecution {
        let identity = TrustedRunIdentity::from_durable_run(
            &durable_run_record(),
            "ws_session_t3",
            TrustedProviderRole::Coder,
        );
        freeze_provider_execution(
            &identity,
            ProviderExecutionProfile::HostCoding,
            None,
            "scope_run_t3".to_string(),
            None,
        )
        .expect("freeze coder execution")
    }

    fn author_execution() -> FrozenProviderExecution {
        let mut record = durable_run_record();
        record.adapter_role = AdapterRole::Orchestrator;
        let identity = TrustedRunIdentity::from_durable_run(
            &record,
            "ws_session_t3",
            TrustedProviderRole::Author,
        );
        freeze_provider_execution(
            &identity,
            ProviderExecutionProfile::HostCoding,
            Some(AuthorToolPolicy::author_v1()),
            "scope_run_t3".to_string(),
            None,
        )
        .expect("freeze author execution")
    }

    /// 宿主环境快照 fixture:白名单键 + 各类必须被拒的注入尝试。
    fn inherited_fixture(owner_home: &Path, worktree: &Path) -> BTreeMap<String, String> {
        let mut inherited = BTreeMap::new();
        inherited.insert(
            "PATH".to_string(),
            std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string()),
        );
        inherited.insert("LANG".to_string(), "C.UTF-8".to_string());
        // 工具链键不在精确继承表:不经批准不得进入。
        inherited.insert("JAVA_HOME".to_string(), "/host/unapproved-jdk".to_string());
        // 非白名单/入站令牌/执行参数默认不进。
        inherited.insert("UNRELATED_TOKEN".to_string(), "host-secret".to_string());
        inherited.insert(
            "ARIA_BOOTSTRAP_TOKEN".to_string(),
            "inbound-auth-material".to_string(),
        );
        inherited.insert("NODE_OPTIONS".to_string(), "--unapproved".to_string());
        // HOME 由受信 owner home 冻结,不随宿主/工作区漂移。
        inherited.insert("HOME".to_string(), worktree.display().to_string());
        let _ = owner_home;
        inherited
    }

    fn approved_fixture() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("JAVA_HOME".to_string(), "/approved/jdk-21".to_string()),
            (
                "DOCKER_HOST".to_string(),
                "unix:///run/podman/podman.sock".to_string(),
            ),
        ])
    }

    /// 经 ProcessManager spawn 子进程打印键名(证明最终环境真实到达子进程)。
    #[cfg(unix)]
    async fn child_env_keys(env: &BTreeMap<String, String>, fixture: &Path) -> Vec<String> {
        use tokio::io::AsyncReadExt;

        let mut process = crate::cross_cutting::process_manager::ProcessManager::spawn(
            &fixture.to_string_lossy(),
            &[],
            fixture.parent().expect("fixture dir"),
            env,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("spawn key printer fixture");

        let mut stdout = String::new();
        use tokio::io::AsyncRead;
        let mut reader = std::pin::pin!(process.stdout);
        reader.read_to_string(&mut stdout).await.expect("read keys");
        let _ = process.child.wait().await.expect("wait fixture");
        stdout.lines().map(str::to_string).collect()
    }

    /// Step 1 红测①:子进程打印键名 fixture——工具链 PATH/JAVA_HOME 与显式
    /// 批准容器连接可用;非白名单 `UNRELATED_TOKEN`、入站 token、模型 env
    /// 覆盖、`NODE_OPTIONS` 默认不进;HOME=owner 且非 worktree。
    #[cfg(unix)]
    #[tokio::test]
    async fn child_process_receives_exactly_whitelisted_environment() {
        let dir = t3_dir();
        let owner_home = dir.path().join("owner-home");
        std::fs::create_dir_all(&owner_home).expect("owner home");
        let worktree = dir.path().join("worktree");
        std::fs::create_dir_all(&worktree).expect("worktree");

        let frozen = build_provider_environment(
            &coder_execution(),
            &owner_home,
            &inherited_fixture(&owner_home, &worktree),
            &approved_fixture(),
        )
        .expect("build frozen environment");

        let fixture = write_key_printer(dir.path());
        let keys = child_env_keys(frozen.env(), &fixture).await;
        // /bin/sh 自身会补 PWD/SHLVL/_ 等 shell 运行时键(非继承注入),
        // 排除后必须与冻结键集完全一致。
        let keys: Vec<String> = keys
            .into_iter()
            .filter(|key| !matches!(key.as_str(), "PWD" | "SHLVL" | "_" | "OLDPWD"))
            .collect();

        assert_eq!(
            keys,
            vec![
                "CODEX_HOME",
                "DOCKER_HOST",
                "HOME",
                "JAVA_HOME",
                "LANG",
                "PATH",
            ],
            "子进程必须只看到精确继承 + 显式批准 + 平台冻结键"
        );
        assert_eq!(frozen.env()["HOME"], owner_home.display().to_string());
        assert_eq!(frozen.env()["JAVA_HOME"], "/approved/jdk-21");
        assert_eq!(
            frozen.env()["DOCKER_HOST"],
            "unix:///run/podman/podman.sock"
        );
    }

    /// Step 1 红测②:同步 CLI 旁路消费同一产物(经 `cli_adapter` 捕获通道
    /// spawn),不隐式全继承。
    #[cfg(unix)]
    #[test]
    fn sync_cli_capture_consumes_same_frozen_environment() {
        use crate::cross_cutting::adapter_compatibility::CommandSpec;

        let dir = t3_dir();
        let owner_home = dir.path().join("owner-home");
        std::fs::create_dir_all(&owner_home).expect("owner home");
        let worktree = dir.path().join("worktree");
        std::fs::create_dir_all(&worktree).expect("worktree");

        let frozen = build_provider_environment(
            &coder_execution(),
            &owner_home,
            &inherited_fixture(&owner_home, &worktree),
            &approved_fixture(),
        )
        .expect("build frozen environment");

        let fixture = write_key_printer(dir.path());
        let spec = CommandSpec::new(fixture.to_string_lossy().to_string(), Vec::new());
        let output = crate::cross_cutting::cli_adapter::run_command_capture_with_environment(
            &spec,
            None,
            frozen.env(),
            None,
            Some(std::time::Duration::from_secs(30)),
        )
        .expect("sync capture fixture");

        assert_eq!(output.exit_code, Some(0), "stderr: {}", output.stderr);
        // /bin/sh 自身会补 PWD/SHLVL/_ 等 shell 运行时键(非继承注入),
        // 排除后必须与冻结键集完全一致。
        let keys: Vec<&str> = output
            .stdout
            .lines()
            .filter(|key| !matches!(*key, "PWD" | "SHLVL" | "_" | "OLDPWD"))
            .collect();
        assert_eq!(
            keys,
            vec![
                "CODEX_HOME",
                "DOCKER_HOST",
                "HOME",
                "JAVA_HOME",
                "LANG",
                "PATH"
            ],
            "同步 CLI 旁路必须只消费冻结产物键集"
        );
    }

    /// Step 1 红测③:`approved` 表外键 fail-closed 拒绝——不能由模型/任意
    /// caller map 冒充受信配置;`NODE_OPTIONS` 默认不进但显式批准可进。
    #[test]
    fn approved_keys_outside_precise_tables_are_rejected() {
        let dir = t3_dir();
        let owner_home = dir.path().join("owner-home");
        std::fs::create_dir_all(&owner_home).expect("owner home");

        for smuggled in ["UNRELATED_TOKEN", "PATH", "HOME", "ANTHROPIC_API_KEY"] {
            let approved = BTreeMap::from([(smuggled.to_string(), "spoof".to_string())]);
            let error = build_provider_environment(
                &coder_execution(),
                &owner_home,
                &BTreeMap::new(),
                &approved,
            )
            .expect_err("表外 approved 键必须拒绝");
            assert!(
                error
                    .details
                    .contains("provider_environment_approved_key_rejected"),
                "unexpected details: {}",
                error.details
            );
        }

        // 显式批准的执行参数键可进(键名入摘要,值不入)。
        let approved = BTreeMap::from([("NODE_OPTIONS".to_string(), "--approved".to_string())]);
        let frozen = build_provider_environment(
            &coder_execution(),
            &owner_home,
            &BTreeMap::new(),
            &approved,
        )
        .expect("显式批准的 NODE_OPTIONS 必须可注入");
        assert_eq!(
            frozen.env().get("NODE_OPTIONS").map(String::as_str),
            Some("--approved")
        );
    }

    /// Step 1 红测④:trusted 注入优先级固定(base < approved < frozen)——
    /// 平台冻结字段覆盖一切;CODEX_HOME 等值去重归属 frozen。
    #[test]
    fn layer_priority_is_frozen_over_approved_over_base() {
        let dir = t3_dir();
        let owner_home = dir.path().join("owner-home");
        std::fs::create_dir_all(&owner_home).expect("owner home");
        let codex_home = owner_home.join(".codex");

        let mut inherited = BTreeMap::new();
        inherited.insert("PATH".to_string(), "/base/path".to_string());
        inherited.insert("HOME".to_string(), "/host/home".to_string());
        // 等值 CODEX_HOME:与冻结发现一致,允许(不属"另一个 CODEX_HOME")。
        inherited.insert("CODEX_HOME".to_string(), codex_home.display().to_string());
        let mut approved = approved_fixture();

        let frozen =
            build_provider_environment(&coder_execution(), &owner_home, &inherited, &mut approved)
                .expect("build frozen environment");

        assert_eq!(frozen.env()["PATH"], "/base/path");
        assert_eq!(frozen.env()["HOME"], owner_home.display().to_string());
        assert_eq!(frozen.env()["CODEX_HOME"], codex_home.display().to_string());

        let facts = frozen.audit_facts();
        assert!(facts["key_sources"].contains("CODEX_HOME:frozen"));
        assert!(facts["key_sources"].contains("HOME:frozen"));
        assert!(facts["key_sources"].contains("JAVA_HOME:approved"));
        assert!(facts["key_sources"].contains("PATH:inherited"));
    }

    /// Step 1 红测⑤:另一个 `CODEX_HOME` 显式拒绝并给配置发现不一致诊断。
    #[test]
    fn another_codex_home_is_explicitly_rejected() {
        let dir = t3_dir();
        let owner_home = dir.path().join("owner-home");
        std::fs::create_dir_all(&owner_home).expect("owner home");

        for carrier in ["inherited", "approved"] {
            let inherited = if carrier == "inherited" {
                BTreeMap::from([("CODEX_HOME".to_string(), "/other/codex-home".to_string())])
            } else {
                BTreeMap::new()
            };
            let approved = if carrier == "approved" {
                BTreeMap::from([("CODEX_HOME".to_string(), "/other/codex-home".to_string())])
            } else {
                BTreeMap::new()
            };
            let error =
                build_provider_environment(&coder_execution(), &owner_home, &inherited, &approved)
                    .expect_err("另一个 CODEX_HOME 必须显式拒绝");
            assert!(
                error
                    .details
                    .contains("provider_environment_codex_home_conflict"),
                "unexpected details: {}",
                error.details
            );
            // 配置发现不一致诊断:同时携带请求值与 trust adapter 冻结值。
            assert!(error.details.contains("/other/codex-home"));
            assert!(error.details.contains("config.toml"));
        }
    }

    /// Step 1 红测⑥:Codex/Kimi CLI 配置发现引用与 trust adapter 完全相同
    /// (同一 `for_home(owner_home)` 来源)。
    #[test]
    fn config_discovery_refs_match_trust_adapters_exactly() {
        use crate::product::logical_codebase::provider_trust_adapters::{
            CodexTrustAdapter, KimiTrustAdapter,
        };

        let dir = t3_dir();
        let owner_home = dir.path().join("owner-home");
        std::fs::create_dir_all(&owner_home).expect("owner home");

        let frozen = build_provider_environment(
            &coder_execution(),
            &owner_home,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("build frozen environment");

        assert_eq!(
            frozen.codex_config_discovery_ref(),
            CodexTrustAdapter::for_home(&owner_home).config_path()
        );
        assert_eq!(
            frozen.kimi_workspace_trust_ref(),
            KimiTrustAdapter::for_home(&owner_home).workspace_trust_root()
        );
        assert_eq!(
            frozen.env()["CODEX_HOME"],
            CodexTrustAdapter::for_home(&owner_home)
                .config_path()
                .parent()
                .expect("codex home")
                .display()
                .to_string()
        );
        let facts = frozen.audit_facts();
        assert!(facts["codex_config_ref"].ends_with("/.codex/config.toml"));
        assert!(facts["kimi_workspace_trust_ref"].ends_with("/.kimi-code/workspace-trust"));
    }

    /// Step 1 红测⑦:secret 值不进摘要/Debug/audit——键名与来源可见,值
    /// 永不出现;同键集不同值摘要不变。
    #[test]
    fn secrets_never_enter_debug_digest_or_audit() {
        let dir = t3_dir();
        let owner_home = dir.path().join("owner-home");
        std::fs::create_dir_all(&owner_home).expect("owner home");

        let approved = BTreeMap::from([(
            "CODEGRAPH_TOKEN".to_string(),
            "sk-live-supersecret".to_string(),
        )]);
        let frozen = build_provider_environment(
            &coder_execution(),
            &owner_home,
            &BTreeMap::new(),
            &approved,
        )
        .expect("precise secret key is approvable");

        assert_eq!(
            frozen.env().get("CODEGRAPH_TOKEN").map(String::as_str),
            Some("sk-live-supersecret")
        );
        let debug_text = format!("{frozen:?}");
        assert!(
            !debug_text.contains("sk-live-supersecret"),
            "Debug 不得泄漏 secret 值"
        );
        assert!(debug_text.contains("CODEGRAPH_TOKEN"), "Debug 保留键名");
        let facts = frozen.audit_facts();
        let facts_text = facts.values().cloned().collect::<Vec<_>>().join(" ");
        assert!(
            !facts_text.contains("sk-live-supersecret"),
            "audit 不含 secret 值"
        );
        assert!(
            facts["keys"].contains("CODEGRAPH_TOKEN"),
            "audit 保留键集合"
        );

        // 同键集不同 secret 值 → 键/来源摘要不变(摘要只覆盖键与来源)。
        let other = BTreeMap::from([(
            "CODEGRAPH_TOKEN".to_string(),
            "sk-live-rotated-value".to_string(),
        )]);
        let frozen_rotated =
            build_provider_environment(&coder_execution(), &owner_home, &BTreeMap::new(), &other)
                .expect("rebuild with rotated secret");
        assert_eq!(frozen.keyset_digest(), frozen_rotated.keyset_digest());
    }

    /// Step 4:trusted token 仅按精确配置进入适用 provider——批准了才进,
    /// 未批准该键的另一次 launch 不携带(per-launch 精确配置,非全局透传)。
    #[test]
    fn trusted_secret_enters_only_launches_that_approved_it() {
        let dir = t3_dir();
        let owner_home = dir.path().join("owner-home");
        std::fs::create_dir_all(&owner_home).expect("owner home");

        let approved = BTreeMap::from([("CODEGRAPH_TOKEN".to_string(), "sk-live-a".to_string())]);
        let with_token = build_provider_environment(
            &coder_execution(),
            &owner_home,
            &BTreeMap::new(),
            &approved,
        )
        .expect("build with token");
        let without_token = build_provider_environment(
            &author_execution(),
            &owner_home,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("build without token");

        assert!(with_token.env().contains_key("CODEGRAPH_TOKEN"));
        assert!(!without_token.env().contains_key("CODEGRAPH_TOKEN"));
    }

    /// 平台入站认证令牌即使混入 `inherited` 也永不进入(白名单外零透传)。
    #[test]
    fn platform_inbound_token_is_always_excluded() {
        let dir = t3_dir();
        let owner_home = dir.path().join("owner-home");
        std::fs::create_dir_all(&owner_home).expect("owner home");

        let inherited = BTreeMap::from([
            ("PATH".to_string(), "/usr/bin".to_string()),
            (
                "ARIA_BOOTSTRAP_TOKEN".to_string(),
                "boot-secret".to_string(),
            ),
            ("aria_session".to_string(), "session-secret".to_string()),
            ("UNRELATED_TOKEN".to_string(), "noise".to_string()),
        ]);
        let frozen = build_provider_environment(
            &coder_execution(),
            &owner_home,
            &inherited,
            &BTreeMap::new(),
        )
        .expect("build frozen environment");

        assert_eq!(
            frozen.env().keys().collect::<Vec<_>>(),
            vec!["CODEX_HOME", "HOME", "PATH"]
        );
    }

    /// 精确继承表外的宿主 HOME 不进;owner home 必须绝对路径。
    #[test]
    fn owner_home_must_be_absolute() {
        let error = build_provider_environment(
            &coder_execution(),
            Path::new("relative/home"),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect_err("相对 owner home 必须拒绝");
        assert!(
            error
                .details
                .contains("provider_environment_owner_home_not_absolute")
        );
    }

    /// 不存在的基础键不伪造(无 LANG/TZ 宿主值时环境不含该键)。
    #[test]
    fn absent_base_keys_are_not_fabricated() {
        let dir = t3_dir();
        let owner_home = dir.path().join("owner-home");
        std::fs::create_dir_all(&owner_home).expect("owner home");

        let inherited = BTreeMap::from([("PATH".to_string(), "/usr/bin".to_string())]);
        let frozen = build_provider_environment(
            &coder_execution(),
            &owner_home,
            &inherited,
            &BTreeMap::new(),
        )
        .expect("build frozen environment");

        assert!(!frozen.env().contains_key("LANG"));
        assert!(!frozen.env().contains_key("TZ"));
        assert!(!frozen.env().contains_key("SHELL"));
    }

    /// 摘要覆盖键集与来源:同版本不同键集 → 摘要不同。
    #[test]
    fn keyset_digest_tracks_keys_and_sources() {
        let dir = t3_dir();
        let owner_home = dir.path().join("owner-home");
        std::fs::create_dir_all(&owner_home).expect("owner home");

        let base = build_provider_environment(
            &coder_execution(),
            &owner_home,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect("base env");
        let with_toolchain = build_provider_environment(
            &coder_execution(),
            &owner_home,
            &BTreeMap::new(),
            &BTreeMap::from([("JAVA_HOME".to_string(), "/jdk".to_string())]),
        )
        .expect("toolchain env");
        assert_ne!(base.keyset_digest(), with_toolchain.keyset_digest());
        assert_eq!(base.policy_version(), PROVIDER_ENV_POLICY_VERSION);
    }

    /// owner home 环境解析:绝对路径过滤,与 trust adapter 同源。
    #[test]
    fn trusted_owner_home_resolution_matches_trust_adapter_source() {
        // 同一函数被 provider_trust_adapters::home_from_env 复用:这里验证
        // 绝对路径过滤语义本身(相对 HOME 不产生 owner home)。
        let resolved = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from);
        match resolved {
            Some(home) if home.is_absolute() => {
                assert_eq!(
                    resolve_trusted_owner_home_from_env(),
                    Some(home),
                    "绝对 HOME 必须原样解析"
                );
            }
            _ => {
                // CI 无 HOME 或相对 HOME:不伪造 owner home。
                let resolved = resolve_trusted_owner_home_from_env();
                assert!(resolved.is_none() || resolved.expect("checked").is_absolute());
            }
        }
    }
}
