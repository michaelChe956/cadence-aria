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
/// policy 与 `input.working_dir` 重新执行。
#[derive(Debug, Clone)]
pub struct ValidatedStreamingProviderInput {
    input: StreamingProviderInput,
    launch: ValidatedSessionLaunchPolicy,
}

impl ValidatedStreamingProviderInput {
    pub fn new(input: StreamingProviderInput, launch: ValidatedSessionLaunchPolicy) -> Self {
        Self { input, launch }
    }

    pub(crate) fn into_parts(self) -> (StreamingProviderInput, ValidatedSessionLaunchPolicy) {
        (self.input, self.launch)
    }
}

/// 已绑定 validated policy 的同步 adapter input。spawn 前复验在
/// `LogicalCodebaseProviderGateway::run_sync` 内基于其中的 validated policy 与
/// `input.worktree_path` 重新执行。
#[derive(Debug, Clone)]
pub struct ValidatedAdapterInput {
    input: AdapterInput,
    launch: ValidatedSessionLaunchPolicy,
}

impl ValidatedAdapterInput {
    pub fn new(input: AdapterInput, launch: ValidatedSessionLaunchPolicy) -> Self {
        Self { input, launch }
    }

    pub(crate) fn into_parts(self) -> (AdapterInput, ValidatedSessionLaunchPolicy) {
        (self.input, self.launch)
    }
}
