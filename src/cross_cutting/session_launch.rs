//! Validated provider launch inputs:把已校验的 `ValidatedSessionLaunchPolicy`
//! 与真实 provider input 绑定为单一不可分割类型。
//!
//! `LogicalCodebaseProviderGateway` 是逻辑代码库真实 provider 启动的唯一入口。
//! 裸 `StreamingProviderInput`/`AdapterInput` 不能跨越该边界:它们必须先与一个
//! 经 gateway `validate` 产出的 `ValidatedSessionLaunchPolicy` 组装成
//! `ValidatedStreamingProviderInput`/`ValidatedAdapterInput`,才能进入
//! `start_streaming`/`run_sync`。这把「真实 provider 必须持有一个 validated
//! policy 才能启动」从编译期约束(无 public constructor)落到了启动边界上。
//!
//! `into_parts` 是 `pub(crate)`:外部调用方只能消费 validated input,无法拆出
//! 裸 input 绕过 spawn 前复验。
//!
//! `ValidatedSessionLaunchPolicy` 没有 public constructor(字段 private),因此
//! 本模块的类型只能由 `LogicalCodebaseProviderGateway::validate` 产出的 policy
//! 组装。组装/拆解的往返由 gateway 内联测试覆盖(那里能取得 validated policy)。

use crate::cross_cutting::streaming_provider::StreamingProviderInput;
use crate::product::logical_codebase::provider_gateway::ValidatedSessionLaunchPolicy;
use crate::protocol::contracts::AdapterInput;

/// 已绑定 validated policy 的 streaming provider input。spawn 前复验在
/// `LogicalCodebaseProviderGateway::start_streaming` 内基于其中的 validated
/// policy 与 input 的 effective working directory(Task 2.5:优先独立
/// `working_directory`,否则回填 `working_dir`)重新执行。
#[derive(Debug, Clone)]
pub struct ValidatedStreamingProviderInput {
    input: StreamingProviderInput,
    launch: ValidatedSessionLaunchPolicy,
    /// B1-1:本次 launch 的受信资源归属(engine 三处真实 scope 铸造,
    /// `provider_resources::TrustedLaunchOwnership`)。`None` = 无归属
    /// 请求(存量构造/测试)。crate 内绑定/读取——外部不可伪造归属。
    launch_ownership: Option<crate::cross_cutting::provider_resources::TrustedLaunchOwnership>,
}

impl ValidatedStreamingProviderInput {
    /// Task 1b 段③(裁决 A):构造入口收紧为 crate 内——生产 caller 一律经
    /// gateway `prepare_streaming_launch`(prepare 前 bind run-bound sink);
    /// crate 外不可再凭空组装 validated input(raw `start` 仅保留单仓 direct)。
    pub(crate) fn new(input: StreamingProviderInput, launch: ValidatedSessionLaunchPolicy) -> Self {
        Self {
            input,
            launch,
            launch_ownership: None,
        }
    }

    pub(crate) fn into_parts(self) -> (StreamingProviderInput, ValidatedSessionLaunchPolicy) {
        (self.input, self.launch)
    }

    /// 批2 Task 2:绑定冻结合同后返回新载体(crate 内受信 engine builder
    /// 专用,gateway `prepare_trusted_*_launch` 消费)。普通 caller 无绑定
    /// 入口——serde 输入/role 字符串不能铸造启动能力。
    pub(crate) fn with_frozen_execution(
        mut self,
        execution: crate::cross_cutting::provider_execution::FrozenProviderExecution,
    ) -> Self {
        self.launch = self.launch.with_frozen_execution(execution);
        self
    }

    /// 批2 Task 2:冻结的执行合同(REQ-ENV-HOST-01)。`None` = 新执行合同
    /// **显式缺失**(存量构造/旧记录):消费方(adapter/engine/恢复面)不得
    /// 把缺失默认为 HostCoding 续接旧 session,只能走显式标缺/显式新会话
    /// 路径。adapter 消费面(T11–T13)以此读取冻结合同事实。
    pub fn frozen_execution(
        &self,
    ) -> Option<&crate::cross_cutting::provider_execution::FrozenProviderExecution> {
        self.launch.frozen_execution()
    }

    /// B1-1:绑定本次 launch 的受信资源归属(crate 内受信装配面——launch
    /// 分流点 `launch_provider_session` 的 validated+gateway 臂与同步桥接
    /// 透传消费)。字段 private、载体 crate 内铸造,caller/模型不可伪造。
    pub(crate) fn with_launch_ownership(
        mut self,
        ownership: crate::cross_cutting::provider_resources::TrustedLaunchOwnership,
    ) -> Self {
        self.launch_ownership = Some(ownership);
        self
    }

    /// B1-1:本次 launch 的受信资源归属(`None` = 无归属请求)。真实
    /// adapter 的 B1 接线片在 spawn 点消费(预留→spawn→回填 PID+starttime
    pub(crate) fn launch_ownership(
        &self,
    ) -> Option<&crate::cross_cutting::provider_resources::TrustedLaunchOwnership> {
        self.launch_ownership.as_ref()
    }
}

/// 已绑定 validated policy 的同步 adapter input。spawn 前复验在
/// `LogicalCodebaseProviderGateway::run_sync` 内基于其中的 validated policy 与
/// input 的 effective working directory(Task 2.5:优先独立 `working_directory`,
/// 否则回填 `worktree_path`)重新执行;`worktree_path` 仍是 target。
#[derive(Debug, Clone)]
pub struct ValidatedAdapterInput {
    input: AdapterInput,
    launch: ValidatedSessionLaunchPolicy,
    /// B1-1:同步载体同样可携带受信 launch 归属(同步桥接
    /// `GatewaySyncProvider::run_validated` 透传进 streaming validated
    /// input;单仓同步路 cli_adapter 明确不在 v1)。
    launch_ownership: Option<crate::cross_cutting::provider_resources::TrustedLaunchOwnership>,
}

impl ValidatedAdapterInput {
    /// Task 1b 段③(裁决 A):同 streaming 侧收紧——生产 caller 经 gateway
    /// `prepare_sync_launch`;crate 外无构造入口。
    pub(crate) fn new(input: AdapterInput, launch: ValidatedSessionLaunchPolicy) -> Self {
        Self {
            input,
            launch,
            launch_ownership: None,
        }
    }

    pub(crate) fn into_parts(self) -> (AdapterInput, ValidatedSessionLaunchPolicy) {
        (self.input, self.launch)
    }

    /// 批2 Task 2:同步载体的冻结合同绑定(crate 内受信 builder 专用,
    /// gateway `prepare_trusted_sync_launch` 消费)。
    pub(crate) fn with_frozen_execution(
        mut self,
        execution: crate::cross_cutting::provider_execution::FrozenProviderExecution,
    ) -> Self {
        self.launch = self.launch.with_frozen_execution(execution);
        self
    }

    /// B1-1:同步载体的受信归属绑定(crate 内受信装配面;与 streaming 侧
    /// 同型)。当前同步桥接只读 [`Self::launch_ownership`] 透传,尚无可铸造
    /// 归属的同步受信装配 caller;保留该安全绑定口并在接线时消费,不以删
    /// 载体消 warning。
    #[allow(dead_code)] // 同步受信装配接线后消费;安全契约载体不删
    pub(crate) fn with_launch_ownership(
        mut self,
        ownership: crate::cross_cutting::provider_resources::TrustedLaunchOwnership,
    ) -> Self {
        self.launch_ownership = Some(ownership);
        self
    }

    /// B1-1:本次同步 launch 的受信资源归属(`None` = 无归属请求)。
    pub(crate) fn launch_ownership(
        &self,
    ) -> Option<&crate::cross_cutting::provider_resources::TrustedLaunchOwnership> {
        self.launch_ownership.as_ref()
    }

    /// 批2 Task 2:冻结的执行合同;`None` = 显式缺失(不默认 HostCoding),
    /// 语义同 [`ValidatedStreamingProviderInput::frozen_execution`]。
    pub fn frozen_execution(
        &self,
    ) -> Option<&crate::cross_cutting::provider_execution::FrozenProviderExecution> {
        self.launch.frozen_execution()
    }
}
