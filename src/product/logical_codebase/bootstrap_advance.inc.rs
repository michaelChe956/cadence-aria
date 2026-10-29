// ---------------------------------------------------------------------------
// A（REQ-REG-10「自动推进」）：首建五步的顺序推进器
// ---------------------------------------------------------------------------
//
// 契约场景「全新 LC 无手工铺底完成首建」要求系统按
// identity → manifest/checkout → rules/policy → member index → aggregate
// index active 的依赖顺序**自动推进**，而不是等客户端逐步点按钮。
//
// 本推进器只做三件事：
// 1. 读 durable 投影（与 GET 同一 projector，零写入）；
// 2. 对依赖顺序上第一个「可动」步骤执行其 `allowed_actions` 里的
//    Prepare/Continue（复用既有 `apply` 派发，不新造状态机语义）；
// 3. 推进一格后重新投影并循环，直到 planning_ready / failed /
//    waiting_for_human / 无动作可做 / 有界上限。
//
// 幂等与并发：
// - 每步 command_id 从 `lc_id + step + membership_revision` 稳定派生，同
//   command 重放不会产生第二份 durable 事实（既有 `dispatch_*` 的 replay
//   语义原样复用）；
// - 推进期间持有 LC scope 的 `.bootstrap-advance.lock`（跨进程排他），两个
//   并发 advance 不会双推；后到者看到的是前者的 durable 结果。
//
// 不做的事：绝不自动重试 Failed 步骤（人工决定）；绝不在读路径推进（GET
// 仍走 projector，不触达本文件）。

/// advance 的 LC scope 排他锁文件名。
const BOOTSTRAP_ADVANCE_LOCK_FILE: &str = ".bootstrap-advance.lock";
/// 循环上限：五步 + 少量余量（每步最多一轮推进复投影）。
const BOOTSTRAP_ADVANCE_MAX_ROUNDS: usize = 7;

/// 首建自动推进请求：身份 + 无客户端状态（幂等由服务端派生）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapAdvanceRequest {
    pub project_id: String,
    pub logical_codebase_id: String,
}

/// 推进停在哪里（稳定、可解释；前端按 `reason()` 呈现）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapAdvanceStop {
    /// 五步全部 Completed。
    PlanningReady,
    /// 依赖顺序上的步骤正在运行（provider/index 长任务未收口）。
    InProgress {
        step: LogicalCodebaseBootstrapStep,
    },
    /// 步骤失败：不自动重试，等待人工决定。
    Failed {
        step: LogicalCodebaseBootstrapStep,
        reason_code: String,
    },
    /// 步骤等待人工（缺材料/缺 trust/漂移等），投影里的 allowed_actions
    /// 就是可点的按钮。
    WaitingForHuman {
        step: LogicalCodebaseBootstrapStep,
        reason_code: String,
    },
    /// 步骤投影没有 Prepare/Continue 可做（例如只允许 Retry/Revalidate）。
    NoActionableStep {
        step: LogicalCodebaseBootstrapStep,
        reason_code: String,
    },
    /// 有界循环用尽（防死循环）。
    IterationLimit,
}

impl BootstrapAdvanceStop {
    /// 稳定的停止原因名（DTO/事件/前端共用，不引第二套字符串协议）。
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::PlanningReady => "planning_ready",
            Self::InProgress { .. } => "in_progress",
            Self::Failed { .. } => "failed",
            Self::WaitingForHuman { .. } => "waiting_for_human",
            Self::NoActionableStep { .. } => "no_actionable_step",
            Self::IterationLimit => "iteration_limit",
        }
    }

    /// 停住的步骤（PlanningReady / IterationLimit 没有具体步骤）。
    pub const fn step(&self) -> Option<LogicalCodebaseBootstrapStep> {
        match self {
            Self::InProgress { step }
            | Self::Failed { step, .. }
            | Self::WaitingForHuman { step, .. }
            | Self::NoActionableStep { step, .. } => Some(*step),
            Self::PlanningReady | Self::IterationLimit => None,
        }
    }

    /// 停住步骤的稳定原因码（来自既有投影 failure，或推进器自身的 stable 码）。
    pub fn reason_code(&self) -> Option<&str> {
        match self {
            Self::Failed { reason_code, .. }
            | Self::WaitingForHuman { reason_code, .. }
            | Self::NoActionableStep { reason_code, .. } => Some(reason_code.as_str()),
            _ => None,
        }
    }
}

/// 本次实际推进的一步（只记真正产生 durable 进展的派发）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapAdvanceAttempt {
    pub step: LogicalCodebaseBootstrapStep,
    pub action: BootstrapActionKind,
    pub command_id: String,
}

/// 推进结果：最终投影（与 GET 同构）+ 本次推进了哪些步骤 + 停在哪。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapAdvanceResult {
    pub project_id: String,
    pub logical_codebase_id: String,
    pub advanced_steps: Vec<BootstrapAdvanceAttempt>,
    pub stopped: BootstrapAdvanceStop,
    pub projection: LogicalCodebaseBootstrapProjection,
}

/// 「准备」派发请求：rules/policy 与 member index 的 Prepare 语义是把根配方
/// operation 跑起来（provider 长任务），产品层不引 coordinator 依赖。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapPrepareRequest {
    pub project_id: String,
    pub logical_codebase_id: String,
    /// 稳定派生命令身份（同时作为 aggregate initialization 的
    /// idempotency_key：同 lc + 同 membership revision 只跑一次）。
    pub command_id: String,
    pub membership_revision: Option<u64>,
}

/// 「准备」派发结果。只有真正启动了长任务才算进展；已在跑与等人工都不算。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapPrepareOutcome {
    /// 根配方长任务已由本次调用启动（步骤进入 Running）。
    Started,
    /// 根配方长任务早已在跑：本次没有推进任何东西（幂等拒绝重复启动）。
    AlreadyRunning,
    /// 无法自动准备（trust 未就绪/provider 不可用/已失败等）。
    WaitingForHuman { reason_code: String },
}

/// C4/A：把「跑起根配方 operation」注入产品层（web 层提供既有编排链）。
pub type BootstrapPrepareDispatcher =
    std::sync::Arc<dyn Fn(&BootstrapPrepareRequest) -> BootstrapPrepareOutcome + Send + Sync>;

/// 单步推进决策（纯函数，便于钉住「失败不自动重试」等契约）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum AdvanceDecision {
    Dispatch(BootstrapActionKind),
    Stop(BootstrapAdvanceStop),
}

fn advance_decision(step: &BootstrapStepProjection) -> AdvanceDecision {
    let failure_reason = || {
        step.failure
            .as_ref()
            .map(|failure| failure.reason_code.clone())
            .unwrap_or_else(|| format!("bootstrap_{}_not_ready", step.step.as_str()))
    };
    match step.status {
        LogicalCodebaseBootstrapStepStatus::Completed => AdvanceDecision::Stop(
            BootstrapAdvanceStop::NoActionableStep {
                step: step.step,
                reason_code: "bootstrap_step_already_completed".to_string(),
            },
        ),
        // 失败绝不自动重试：这是人工决定（也可能是要修材料/授权）。
        LogicalCodebaseBootstrapStepStatus::Failed => AdvanceDecision::Stop(
            BootstrapAdvanceStop::Failed {
                step: step.step,
                reason_code: failure_reason(),
            },
        ),
        LogicalCodebaseBootstrapStepStatus::WaitingForHuman => AdvanceDecision::Stop(
            BootstrapAdvanceStop::WaitingForHuman {
                step: step.step,
                reason_code: failure_reason(),
            },
        ),
        // Running：provider/index 长任务在跑，等它收口；GET 只是投影。
        LogicalCodebaseBootstrapStepStatus::Running => AdvanceDecision::Stop(
            BootstrapAdvanceStop::InProgress { step: step.step },
        ),
        LogicalCodebaseBootstrapStepStatus::NotStarted => {
            // 契约：对第一个可动步骤执行 prepare（有 checkpoint 时 continue）。
            let has_continuable_checkpoint =
                step.checkpoint.is_some() && step.allowed_actions.contains(&BootstrapActionKind::Continue);
            if has_continuable_checkpoint {
                return AdvanceDecision::Dispatch(BootstrapActionKind::Continue);
            }
            if step.allowed_actions.contains(&BootstrapActionKind::Prepare) {
                return AdvanceDecision::Dispatch(BootstrapActionKind::Prepare);
            }
            if step.allowed_actions.contains(&BootstrapActionKind::Continue) {
                return AdvanceDecision::Dispatch(BootstrapActionKind::Continue);
            }
            AdvanceDecision::Stop(BootstrapAdvanceStop::NoActionableStep {
                step: step.step,
                reason_code: format!("bootstrap_{}_no_actionable", step.step.as_str()),
            })
        }
    }
}

/// 稳定 command 身份：`bootstrap-advance-{lc}-{step}-rev{n}`。
///
/// 用 relative-id 安全字符集（`/`、`\`、`.` 段一律不许进 id，见
/// `validate_relative_id`）；非安全字符折叠为 `-`，保持确定性。
fn advance_command_id(
    logical_codebase_id: &str,
    step: LogicalCodebaseBootstrapStep,
    membership_revision: Option<u64>,
) -> String {
    let mut sanitized = String::with_capacity(logical_codebase_id.len());
    for character in logical_codebase_id.chars() {
        if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
            sanitized.push(character);
        } else {
            sanitized.push('-');
        }
    }
    format!(
        "bootstrap-advance-{sanitized}-{}-rev{}",
        step.as_str(),
        membership_revision.unwrap_or(0)
    )
}

fn bootstrap_step_of(
    projection: &LogicalCodebaseBootstrapProjection,
    step: LogicalCodebaseBootstrapStep,
) -> Option<&BootstrapStepProjection> {
    projection.steps.iter().find(|candidate| candidate.step == step)
}

impl LogicalCodebaseBootstrapService {
    /// 首建自动推进：按依赖顺序把第一个可动步骤推进一格，复投影后继续，
    /// 直到 planning_ready / failed / waiting_for_human / 无动作 / 有界上限。
    ///
    /// 幂等：每步 command 身份稳定派生；重复调用命中既有 replay 语义，
    /// `advanced_steps` 为空。并发：持有 LC 级 `.bootstrap-advance.lock`，
    /// 两个并发 advance 不会双推。
    pub async fn advance(
        &self,
        request: BootstrapAdvanceRequest,
    ) -> Result<BootstrapAdvanceResult, BootstrapActionError> {
        let lock_path = crate::product::logical_codebase::store::lc_scope_root(
            &self.paths,
            &request.project_id,
            &Some(request.logical_codebase_id.clone()),
        )?
        .join(BOOTSTRAP_ADVANCE_LOCK_FILE);
        let _lock =
            crate::product::coding_attempt_store::locking::ExclusiveFileLock::acquire_async(
                &lock_path,
            )
            .await?;
        self.advance_locked(request).await
    }

    async fn advance_locked(
        &self,
        request: BootstrapAdvanceRequest,
    ) -> Result<BootstrapAdvanceResult, BootstrapActionError> {
        let mut advanced = Vec::new();
        let mut projection = self.project_async(&request).await?;
        for _ in 0..BOOTSTRAP_ADVANCE_MAX_ROUNDS {
            if projection.planning_ready {
                return Ok(BootstrapAdvanceResult {
                    project_id: request.project_id,
                    logical_codebase_id: request.logical_codebase_id,
                    advanced_steps: advanced,
                    stopped: BootstrapAdvanceStop::PlanningReady,
                    projection,
                });
            }
            let Some(step) = projection
                .steps
                .iter()
                .find(|step| step.status != LogicalCodebaseBootstrapStepStatus::Completed)
                .cloned()
            else {
                // 防御：五步都完成等价于 planning_ready。
                return Ok(BootstrapAdvanceResult {
                    project_id: request.project_id,
                    logical_codebase_id: request.logical_codebase_id,
                    advanced_steps: advanced,
                    stopped: BootstrapAdvanceStop::PlanningReady,
                    projection,
                });
            };

            let action = match advance_decision(&step) {
                AdvanceDecision::Stop(stopped) => {
                    return Ok(BootstrapAdvanceResult {
                        project_id: request.project_id,
                        logical_codebase_id: request.logical_codebase_id,
                        advanced_steps: advanced,
                        stopped,
                        projection,
                    });
                }
                AdvanceDecision::Dispatch(action) => action,
            };

            let command_id = advance_command_id(
                &request.logical_codebase_id,
                step.step,
                projection.membership_revision,
            );

            // rules/policy 与 member index 的 Prepare 是「跑根配方 operation」
            // ——provider 长任务，由 web 层注入既有编排链接管。
            if action == BootstrapActionKind::Prepare
                && matches!(
                    step.step,
                    LogicalCodebaseBootstrapStep::RulesPolicy
                        | LogicalCodebaseBootstrapStep::MemberIndex
                )
            {
                let Some(prepare) = self.prepare_root_recipe.as_ref() else {
                    return Ok(BootstrapAdvanceResult {
                        project_id: request.project_id,
                        logical_codebase_id: request.logical_codebase_id,
                        advanced_steps: advanced,
                        stopped: BootstrapAdvanceStop::NoActionableStep {
                            step: step.step,
                            reason_code: "bootstrap_prepare_unavailable".to_string(),
                        },
                        projection,
                    });
                };
                let outcome = prepare(&BootstrapPrepareRequest {
                    project_id: request.project_id.clone(),
                    logical_codebase_id: request.logical_codebase_id.clone(),
                    command_id: command_id.clone(),
                    membership_revision: projection.membership_revision,
                });
                match outcome {
                    BootstrapPrepareOutcome::WaitingForHuman { reason_code } => {
                        return Ok(BootstrapAdvanceResult {
                            project_id: request.project_id,
                            logical_codebase_id: request.logical_codebase_id,
                            advanced_steps: advanced,
                            stopped: BootstrapAdvanceStop::WaitingForHuman {
                                step: step.step,
                                reason_code,
                            },
                            projection,
                        });
                    }
                    BootstrapPrepareOutcome::Started => {
                        advanced.push(BootstrapAdvanceAttempt {
                            step: step.step,
                            action,
                            command_id,
                        });
                        projection = self.project_async(&request).await?;
                        let advanced_status = bootstrap_step_of(&projection, step.step)
                            .map(|candidate| candidate.status);
                        if advanced_status == Some(LogicalCodebaseBootstrapStepStatus::Running) {
                            // 长任务已接管：本次推进到此为止，由运行方收口。
                            return Ok(BootstrapAdvanceResult {
                                project_id: request.project_id,
                                logical_codebase_id: request.logical_codebase_id,
                                advanced_steps: advanced,
                                stopped: BootstrapAdvanceStop::InProgress { step: step.step },
                                projection,
                            });
                        }
                        // 短链（快速 provider/测试）可能已跨过 Running，继续循环。
                        continue;
                    }
                    BootstrapPrepareOutcome::AlreadyRunning => {
                        // 幂等：长任务早已在跑，本次不宣称任何推进。
                        return Ok(BootstrapAdvanceResult {
                            project_id: request.project_id,
                            logical_codebase_id: request.logical_codebase_id,
                            advanced_steps: advanced,
                            stopped: BootstrapAdvanceStop::InProgress { step: step.step },
                            projection,
                        });
                    }
                }
            }

            let result = self
                .apply(BootstrapActionRequest {
                    command_id: command_id.clone(),
                    project_id: request.project_id.clone(),
                    logical_codebase_id: request.logical_codebase_id.clone(),
                    step: step.step,
                    action,
                    expected_revision: projection.membership_revision,
                    expected_object_id: step.object_id.clone(),
                })
                .await;
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    // 派发报错但 durable 事实已落到 failed（例如 index CLI
                    // 失败会先落 Failed 记录再返回错误）：按「停在失败」返回，
                    // 不把可解释的失败变成 500，也不自动重试。
                    let after = self.project_async(&request).await?;
                    let after_step = bootstrap_step_of(&after, step.step);
                    if let Some(after_step) = after_step
                        && after_step.status == LogicalCodebaseBootstrapStepStatus::Failed
                    {
                        return Ok(BootstrapAdvanceResult {
                            project_id: request.project_id,
                            logical_codebase_id: request.logical_codebase_id,
                            advanced_steps: advanced,
                            stopped: BootstrapAdvanceStop::Failed {
                                step: step.step,
                                reason_code: after_step
                                    .failure
                                    .as_ref()
                                    .map(|failure| failure.reason_code.clone())
                                    .unwrap_or_else(|| format!("bootstrap_{}_failed", step.step.as_str())),
                            },
                            projection: after,
                        });
                    }
                    return Err(error);
                }
            };

            let after_step = bootstrap_step_of(&result.projection, step.step);
            match result.outcome {
                BootstrapActionOutcome::Accepted => {
                    advanced.push(BootstrapAdvanceAttempt {
                        step: step.step,
                        action,
                        command_id,
                    });
                    projection = result.projection;
                    continue;
                }
                BootstrapActionOutcome::Completed => {
                    advanced.push(BootstrapAdvanceAttempt {
                        step: step.step,
                        action,
                        command_id,
                    });
                    projection = result.projection;
                    continue;
                }
                BootstrapActionOutcome::Replayed => {
                    let progressed = after_step
                        .is_some_and(|candidate| candidate.status != step.status);
                    if progressed {
                        advanced.push(BootstrapAdvanceAttempt {
                            step: step.step,
                            action,
                            command_id,
                        });
                        projection = result.projection;
                        continue;
                    }
                    // 同命令重放且 durable 事实未变：幂等返回，不再推进。
                    return Ok(BootstrapAdvanceResult {
                        project_id: request.project_id,
                        logical_codebase_id: request.logical_codebase_id,
                        advanced_steps: advanced,
                        stopped: BootstrapAdvanceStop::NoActionableStep {
                            step: step.step,
                            reason_code: "bootstrap_advance_replayed".to_string(),
                        },
                        projection: result.projection,
                    });
                }
                BootstrapActionOutcome::WaitingForHuman => {
                    let reason_code = after_step
                        .and_then(|candidate| candidate.failure.as_ref())
                        .map(|failure| failure.reason_code.clone())
                        .unwrap_or_else(|| {
                            format!("bootstrap_{}_waiting_for_human", step.step.as_str())
                        });
                    return Ok(BootstrapAdvanceResult {
                        project_id: request.project_id,
                        logical_codebase_id: request.logical_codebase_id,
                        advanced_steps: advanced,
                        stopped: BootstrapAdvanceStop::WaitingForHuman {
                            step: step.step,
                            reason_code,
                        },
                        projection: result.projection,
                    });
                }
            }
        }
        Ok(BootstrapAdvanceResult {
            project_id: request.project_id,
            logical_codebase_id: request.logical_codebase_id,
            advanced_steps: advanced,
            stopped: BootstrapAdvanceStop::IterationLimit,
            projection,
        })
    }

    /// blocking 线程池上取投影（与 `apply` 一致，不占 async worker）。
    async fn project_async(
        &self,
        request: &BootstrapAdvanceRequest,
    ) -> Result<LogicalCodebaseBootstrapProjection, BootstrapActionError> {
        let paths = self.paths.clone();
        let project_id = request.project_id.clone();
        let logical_codebase_id = request.logical_codebase_id.clone();
        tokio::task::spawn_blocking(move || {
            LogicalCodebaseBootstrapProjector::new(paths).project(&project_id, &logical_codebase_id)
        })
        .await
        .map_err(|error| {
            BootstrapActionError::Store(ProductStoreError::Io(format!(
                "bootstrap projection task failed: {error}"
            )))
        })?
        .map_err(BootstrapActionError::Store)
    }
}
