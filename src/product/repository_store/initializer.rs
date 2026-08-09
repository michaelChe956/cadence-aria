//! 单仓 repository 初始化 provider 驱动。
//!
//! `ClaudeRepositoryInitializer` 仅供**单仓** registration 路径
//! (`RepositoryRegistrationCoordinator`)使用,在单个成员仓 git root 下运行
//! Claude Code 初始化命令。聚合初始化(`AggregateInitializationCoordinator`)
//! 走独立的 `GatewayBackedAggregateProviderTurnDriver` + `AggregateAssetPublisher`,
//! **不**复用本初始化器,也不调用单仓 `git_finalize` 切点——聚合 provider turn
//! 经 `LogicalCodebaseProviderGateway` 启动,产出只发布到 `.aria/aggregate/**`。
//! 该隔离由 `aggregate_initialization_coordinator::tests::
//! aggregate_coordinator_isolation_locked_against_single_repository_persistence_and_git_finalize`
//! 锁定回归。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use super::types::{
    RepositoryInitializationCommandSummary, RepositoryInitializationProgress,
    RepositoryInitializationStepKind, RepositoryRegistrationError,
};
use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, ProviderPermissionMode, ProviderSession, ProviderStatus,
    StreamingProviderInput,
};
use crate::product::models::ProviderName;
use crate::protocol::contracts::{AdapterRole, ProviderType};

pub struct ClaudeRepositoryInitializer {
    gate: Arc<ProviderAvailabilityGate>,
    registry: Arc<ProviderRegistry>,
    output_limit: usize,
    /// Task 11:逻辑代码库 aggregate 初始化 provider turn 的唯一启动入口。非空时,
    /// 初始化命令的 streaming provider 启动改由 `LogicalCodebaseProviderGateway::
    /// start_streaming` 完成;为 `None` 时保留既有直接 `adapter.start` 路径。
    /// aggregate provider turn 在 Task 16 通过同一准备方法进入 gateway。
    logical_provider_gateway:
        Option<Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>>,
}

impl ClaudeRepositoryInitializer {
    pub fn new(
        gate: Arc<ProviderAvailabilityGate>,
        registry: Arc<ProviderRegistry>,
        output_limit: usize,
    ) -> Self {
        Self {
            gate,
            registry,
            output_limit,
            logical_provider_gateway: None,
        }
    }

    /// 注入逻辑代码库 provider gateway(Task 11)。Web 接入 task 在聚合初始化路径
    /// 构造 gateway 后调用此方法,使初始化 provider turn 经 gateway 启动并留 audit;
    /// 未调用时保留直接 `adapter.start` 路径。
    pub fn with_logical_provider_gateway(
        mut self,
        gateway: Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>,
    ) -> Self {
        self.logical_provider_gateway = Some(gateway);
        self
    }

    pub async fn initialize(
        &self,
        git_root: &Path,
        command_timeout: Duration,
        cancellation: CancellationToken,
        progress: Arc<dyn RepositoryInitializationProgress>,
    ) -> Result<Vec<RepositoryInitializationCommandSummary>, RepositoryRegistrationError> {
        let commands = RepositoryInitializationStepKind::ALL
            .into_iter()
            .filter_map(|step| step.command().map(|command| (step, command)));
        let mut summaries = Vec::with_capacity(4);
        for (offset, (step, command)) in commands.enumerate() {
            let command_index = offset + 1;
            progress.step_started(step).map_err(|error| *error)?;
            self.gate
                .ensure_available(&ProviderName::ClaudeCode)
                .map_err(|error| {
                    RepositoryRegistrationError::for_command(
                        "repository_init_provider_gate",
                        error.code(),
                        command_index,
                        command,
                        Some(sanitize_and_truncate(error.reason(), self.output_limit)),
                        true,
                        "Restore Claude Code availability, then retry repository initialization.",
                    )
                })?;
            let adapter = self
                .registry
                .get(&ProviderName::ClaudeCode)
                .ok_or_else(|| {
                    RepositoryRegistrationError::for_command(
                        "repository_init_provider_lookup",
                        "provider_unavailable",
                        command_index,
                        command,
                        Some("Claude Code provider is not registered".to_string()),
                        true,
                        "Register the gated Claude Code provider, then retry.",
                    )
                })?;
            let input = StreamingProviderInput {
                provider_type: ProviderType::ClaudeCode,
                role: AdapterRole::Executor,
                prompt: command.to_string(),
                working_dir: git_root.to_path_buf(),
                workspace_session_id: None,
                resume_provider_session_id: None,
                permission_mode: ProviderPermissionMode::Auto,
                structured_output_contract: None,
                env_vars: BTreeMap::new(),
                timeout_secs: command_timeout.as_secs().max(1),
            };
            let summary = self
                .run_turn(
                    adapter,
                    input,
                    command_index,
                    command,
                    command_timeout,
                    cancellation.clone(),
                )
                .await?;
            progress.step_completed(step).map_err(|error| *error)?;
            summaries.push(summary);
        }
        Ok(summaries)
    }

    async fn run_turn(
        &self,
        adapter: Arc<dyn crate::cross_cutting::streaming_provider::StreamingProviderAdapter>,
        input: StreamingProviderInput,
        command_index: usize,
        command: &str,
        command_timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<RepositoryInitializationCommandSummary, RepositoryRegistrationError> {
        let started = Instant::now();
        let start = adapter.start(input, cancellation.clone());
        tokio::pin!(start);
        let timeout = tokio::time::sleep(command_timeout);
        tokio::pin!(timeout);
        let session = tokio::select! {
            _ = cancellation.cancelled() => {
                return Err(command_failure(command_index, command, "initialization cancelled", true, self.output_limit));
            }
            _ = &mut timeout => {
                return Err(command_failure(command_index, command, "initialization timed out before session start", true, self.output_limit));
            }
            result = &mut start => result.map_err(|error| {
                let summary = if error.stderr.is_empty() {
                    error.stdout
                } else {
                    error.stderr
                };
                command_failure(command_index, command, &summary, true, self.output_limit)
            })?,
        };

        let remaining = command_timeout.saturating_sub(started.elapsed());
        self.consume_session(session, command_index, command, remaining, cancellation)
            .await
    }

    /// Task 11:逻辑代码库 aggregate 初始化 provider turn 经 gateway 启动。与
    /// `run_turn` 对称,但 session 改由 `LogicalCodebaseProviderGateway::start_streaming`
    /// 产出,使初始化命令的真实 provider 启动唯一由 gateway 完成并留 audit。
    ///
    /// 调用方(Task 16/Web 接入)在构造 gateway 后,经 `gateway.validate` +
    /// `ValidatedStreamingProviderInput::new` 组装 validated input 传入。传统/非
    /// 逻辑初始化仍走 `run_turn` 的直接 `adapter.start` 路径。
    #[allow(dead_code)]
    async fn run_turn_via_gateway(
        &self,
        validated_input: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        command_index: usize,
        command: &str,
        command_timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<RepositoryInitializationCommandSummary, RepositoryRegistrationError> {
        let gateway = self
            .logical_provider_gateway
            .clone()
            .expect("logical provider gateway must be injected before gateway turn");
        let started = Instant::now();
        let start = gateway.start_streaming(validated_input, cancellation.clone());
        tokio::pin!(start);
        let timeout = tokio::time::sleep(command_timeout);
        tokio::pin!(timeout);
        let session = tokio::select! {
            _ = cancellation.cancelled() => {
                return Err(command_failure(command_index, command, "initialization cancelled", true, self.output_limit));
            }
            _ = &mut timeout => {
                return Err(command_failure(command_index, command, "initialization timed out before session start", true, self.output_limit));
            }
            result = &mut start => result.map_err(|error| {
                command_failure(command_index, command, &error.to_string(), true, self.output_limit)
            })?,
        };
        let remaining = command_timeout.saturating_sub(started.elapsed());
        self.consume_session(session, command_index, command, remaining, cancellation)
            .await
    }

    async fn consume_session(
        &self,
        mut session: ProviderSession,
        command_index: usize,
        command: &str,
        remaining: Duration,
        cancellation: CancellationToken,
    ) -> Result<RepositoryInitializationCommandSummary, RepositoryRegistrationError> {
        let mut output = LimitedOutput::new(self.output_limit);
        let timeout = tokio::time::sleep(remaining);
        tokio::pin!(timeout);
        loop {
            let event = tokio::select! {
                _ = cancellation.cancelled() => {
                    best_effort_abort(&session);
                    return Err(command_failure_with_output(command_index, command, "initialization cancelled", &output, true));
                }
                _ = &mut timeout => {
                    best_effort_abort(&session);
                    return Err(command_failure_with_output(command_index, command, "initialization timed out", &output, true));
                }
                event = session.events.recv() => event,
            };
            match event {
                Some(ProviderEvent::TextDelta { content }) => output.push(&content),
                Some(ProviderEvent::Execution(execution)) => {
                    if let Some(event_output) = execution.output {
                        output.push(&event_output);
                    }
                }
                Some(ProviderEvent::ToolResult(result)) => output.push(&result.output),
                Some(ProviderEvent::Completed(completion)) => {
                    if output.is_empty() {
                        output.push(&completion.full_output);
                    }
                    return Ok(RepositoryInitializationCommandSummary {
                        command_index,
                        command: command.to_string(),
                        status: "completed".to_string(),
                        output_summary: output.summary(),
                    });
                }
                Some(ProviderEvent::Failed { message }) => {
                    output.push(&message);
                    return Err(command_failure_with_output(
                        command_index,
                        command,
                        "provider reported failure",
                        &output,
                        true,
                    ));
                }
                Some(ProviderEvent::ProtocolError { code, message, .. }) => {
                    output.push(&format!("{code}: {message}"));
                    return Err(command_failure_with_output(
                        command_index,
                        command,
                        "provider protocol error",
                        &output,
                        true,
                    ));
                }
                Some(ProviderEvent::PermissionTimeout { permission_id }) => {
                    output.push(&format!("permission request {permission_id} timed out"));
                    return Err(command_failure_with_output(
                        command_index,
                        command,
                        "provider permission timeout",
                        &output,
                        true,
                    ));
                }
                Some(ProviderEvent::StatusChanged(ProviderStatus::Failed)) => {
                    return Err(command_failure_with_output(
                        command_index,
                        command,
                        "provider status failed",
                        &output,
                        true,
                    ));
                }
                Some(ProviderEvent::StatusChanged(ProviderStatus::Aborted)) => {
                    return Err(command_failure_with_output(
                        command_index,
                        command,
                        "provider status aborted",
                        &output,
                        true,
                    ));
                }
                Some(ProviderEvent::PermissionRequest(_))
                | Some(ProviderEvent::ChoiceRequest(_)) => {
                    best_effort_abort(&session);
                    return Err(RepositoryRegistrationError::for_command(
                        "repository_initialization",
                        "repository_init_interaction_required",
                        command_index,
                        command,
                        output.summary(),
                        false,
                        "Run the initialization command manually and resolve the requested interaction before retrying.",
                    ));
                }
                Some(ProviderEvent::StatusChanged(_)) | Some(ProviderEvent::ToolCall(_)) => {}
                None => {
                    return Err(command_failure_with_output(
                        command_index,
                        command,
                        "provider event stream closed before completion",
                        &output,
                        true,
                    ));
                }
            }
        }
    }
}

fn best_effort_abort(session: &ProviderSession) {
    let _ = session.commands.try_send(ProviderCommand::Abort);
}

fn command_failure(
    command_index: usize,
    command: &str,
    message: &str,
    retryable: bool,
    output_limit: usize,
) -> RepositoryRegistrationError {
    RepositoryRegistrationError::for_command(
        "repository_initialization",
        "repository_init_command_failed",
        command_index,
        command,
        Some(sanitize_and_truncate(message, output_limit)),
        retryable,
        "Inspect the repository for partial changes, resolve the Claude Code failure, then retry.",
    )
}

fn command_failure_with_output(
    command_index: usize,
    command: &str,
    message: &str,
    output: &LimitedOutput,
    retryable: bool,
) -> RepositoryRegistrationError {
    // 失败原因 message 必须总是可见：output 非空时追加到末尾，
    // 不得因 provider 已产生探索输出而把超时/错误原因覆盖丢失。
    let mut details = output.summary().unwrap_or_else(|| message.to_string());
    if !details.contains(message) {
        if !details.is_empty() {
            details.push('\n');
        }
        details.push_str(message);
    }
    command_failure(command_index, command, &details, retryable, output.limit)
}

struct LimitedOutput {
    value: String,
    limit: usize,
    truncated: bool,
}

impl LimitedOutput {
    fn new(limit: usize) -> Self {
        Self {
            value: String::new(),
            limit,
            truncated: false,
        }
    }

    fn push(&mut self, value: &str) {
        for character in value.chars() {
            if self.value.len() + character.len_utf8() > self.limit {
                self.truncated = true;
                break;
            }
            self.value.push(character);
        }
        if self.value.len() < value.len() {
            self.truncated = true;
        }
    }

    fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    fn summary(&self) -> Option<String> {
        if self.value.is_empty() {
            return None;
        }
        let mut value = sanitize_and_truncate(&self.value, self.limit);
        if self.truncated {
            value.push_str("…[truncated]");
        }
        Some(value)
    }
}

fn sanitize_and_truncate(value: &str, limit: usize) -> String {
    let mut cleaned = String::new();
    let mut in_escape = false;
    for character in value.chars() {
        if in_escape {
            if character.is_ascii_alphabetic() {
                in_escape = false;
            }
            continue;
        }
        if character == '\u{1b}' {
            in_escape = true;
            continue;
        }
        if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
            continue;
        }
        if cleaned.len() + character.len_utf8() > limit {
            cleaned.push_str("…[truncated]");
            break;
        }
        cleaned.push(character);
    }
    redact_sensitive_assignments(&cleaned)
}

fn redact_sensitive_assignments(value: &str) -> String {
    value
        .split_whitespace()
        .map(|token| {
            let Some((key, _)) = token.split_once('=') else {
                return token.to_string();
            };
            let upper = key.to_ascii_uppercase();
            if ["KEY", "TOKEN", "SECRET", "PASSWORD"]
                .iter()
                .any(|marker| upper.contains(marker))
            {
                format!("{key}=[REDACTED]")
            } else {
                token.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests;
