/// C1 Task 6：命令账本锁内 `ProductStoreError` → `EnrollmentError` 边界
/// 转换（账本 Conflict 语义保留，其余透传 Store）。
fn command_ledger_error(error: ProductStoreError) -> EnrollmentError {
    match error {
        ProductStoreError::Conflict {
            kind: "enrollment_command_ledger",
            ..
        } => EnrollmentError::Conflict {
            current_revision: None,
        },
        ProductStoreError::NotFound {
            kind: "automation_enrollment",
            ..
        } => EnrollmentError::NotFound,
        other => EnrollmentError::Store(other),
    }
}

fn resolve(resolution: CasResolution) -> Result<IssueAutomationEnrollment, EnrollmentError> {
    match resolution {
        CasResolution::Unchanged(saved) | CasResolution::Applied(saved) => Ok(saved),
        CasResolution::Conflict { current_revision } => {
            Err(EnrollmentError::Conflict { current_revision })
        }
        CasResolution::Missing => Err(EnrollmentError::NotFound),
        CasResolution::InvalidScope(reason) => Err(EnrollmentError::InvalidScope(reason)),
    }
}

/// 锁内绑定分支：同键幂等返回原值；异/半绑定 Conflict；(None,None) 才写入
/// 并 revision+1（P0 `bind_plan` 与 P1 `ensure_plan_binding` 共用，避免嵌套锁）。
fn bind_plan_ids_locked(
    path: &Path,
    saved: IssueAutomationEnrollment,
    plan_id: &str,
    session_id: &str,
) -> Result<CasResolution, ProductStoreError> {
    match (saved.plan_id.as_deref(), saved.session_id.as_deref()) {
        (Some(plan), Some(session)) if plan == plan_id && session == session_id => {
            Ok(CasResolution::Unchanged(saved))
        }
        (Some(_), _) | (_, Some(_)) => Ok(CasResolution::Conflict {
            current_revision: Some(saved.policy_revision),
        }),
        (None, None) => {
            let mut next = saved;
            next.plan_id = Some(plan_id.to_string());
            next.session_id = Some(session_id.to_string());
            // C1 Task 1：绑定补全 durable binding current 的 plan/session
            // 身份（enable 时以空串占位；版本不递增——绑定不是换代）。
            if let Some(history) = next.binding_history.as_mut() {
                history.current.plan_id = plan_id.to_string();
                history.current.session_id = session_id.to_string();
            }
            next.policy_revision += 1;
            next.updated_at = now_rfc3339();
            write_json(path, &next)?;
            Ok(CasResolution::Applied(next))
        }
    }
}

/// `ensure_plan_binding` 锁内判定；`Failed` 把 create 回调的 EnrollmentError
/// 原样穿出锁外（锁闭包错误类型固定为 ProductStoreError）。
enum EnsurePlanResolution {
    Unchanged(IssueAutomationEnrollment),
    Applied(IssueAutomationEnrollment),
    Conflict {
        current_revision: Option<u64>,
    },
    Missing,
    /// C5 Task 1：旧代 enrollment（无显式 target）不能驱动自动链。
    InvalidScope(String),
    Failed(EnrollmentError),
}

fn ensure_resolution_from_cas(resolution: CasResolution) -> EnsurePlanResolution {
    match resolution {
        CasResolution::Unchanged(saved) => EnsurePlanResolution::Unchanged(saved),
        CasResolution::Applied(saved) => EnsurePlanResolution::Applied(saved),
        CasResolution::Conflict { current_revision } => {
            EnsurePlanResolution::Conflict { current_revision }
        }
        CasResolution::Missing => EnsurePlanResolution::Missing,
        // bind_plan_ids_locked 不产生 InvalidScope；此处仅保持穷尽映射。
        CasResolution::InvalidScope(reason) => EnsurePlanResolution::InvalidScope(reason),
    }
}

fn resolve_ensure(
    resolution: EnsurePlanResolution,
) -> Result<IssueAutomationEnrollment, EnrollmentError> {
    match resolution {
        EnsurePlanResolution::Unchanged(saved) | EnsurePlanResolution::Applied(saved) => Ok(saved),
        EnsurePlanResolution::Conflict { current_revision } => {
            Err(EnrollmentError::Conflict { current_revision })
        }
        EnsurePlanResolution::Missing => Err(EnrollmentError::NotFound),
        EnsurePlanResolution::InvalidScope(reason) => Err(EnrollmentError::InvalidScope(reason)),
        EnsurePlanResolution::Failed(error) => Err(error),
    }
}

/// `claim_plan_generation`/`mark_plan_generation_phase` 的锁内判定。
enum GenerationResolution {
    Ready(PlanGenerationIntent),
    Conflict { current_revision: Option<u64> },
    Missing,
    InvalidScope(String),
}

fn resolve_generation(
    resolution: GenerationResolution,
) -> Result<PlanGenerationIntent, EnrollmentError> {
    match resolution {
        GenerationResolution::Ready(intent) => Ok(intent),
        GenerationResolution::Conflict { current_revision } => {
            Err(EnrollmentError::Conflict { current_revision })
        }
        GenerationResolution::Missing => Err(EnrollmentError::NotFound),
        GenerationResolution::InvalidScope(reason) => Err(EnrollmentError::InvalidScope(reason)),
    }
}

/// 批 2 N0：`upsert/resolve_automation_attention_fact` 的锁内判定。
enum AttentionWriteResolution {
    Applied,
    Conflict { current_revision: Option<u64> },
    Missing,
    InvalidScope(String),
}

fn resolve_attention_write(resolution: AttentionWriteResolution) -> Result<(), EnrollmentError> {
    match resolution {
        AttentionWriteResolution::Applied => Ok(()),
        AttentionWriteResolution::Conflict { current_revision } => {
            Err(EnrollmentError::Conflict { current_revision })
        }
        AttentionWriteResolution::Missing => Err(EnrollmentError::NotFound),
        AttentionWriteResolution::InvalidScope(reason) => {
            Err(EnrollmentError::InvalidScope(reason))
        }
    }
}

/// 批 2 N0：attention 事实写/清的精确绑定互证。仅当前代 binding 全等
/// 可写（enrollment_id 一致＋`binding_history.current` 全等；旧/别的绑定
/// 一律 Conflict）；`enabled=false` 且未 terminated 拒绝——同一已
/// terminated 绑定放行终止收尾（清旧事实/写清理失败），不因 enabled=false
/// 阻断（T1 落 terminated_at＋enabled=false）；无 durable binding history
/// 的旧代 enrollment 不驱动自动链（InvalidScope fail-closed）。
fn attest_attention_binding_locked(
    saved: &IssueAutomationEnrollment,
    expected_binding: &EnrollmentBindingIdentity,
) -> Result<(), AttentionWriteResolution> {
    if saved.enrollment_id != expected_binding.enrollment_id {
        return Err(AttentionWriteResolution::Conflict {
            current_revision: Some(saved.policy_revision),
        });
    }
    let Some(history) = saved.binding_history.as_ref() else {
        return Err(AttentionWriteResolution::InvalidScope(
            "attention facts require durable binding history; legacy enrollments \
             cannot drive automatic chains"
                .to_string(),
        ));
    };
    if &history.current != expected_binding {
        return Err(AttentionWriteResolution::Conflict {
            current_revision: Some(saved.policy_revision),
        });
    }
    if !saved.enabled && saved.terminated_at.is_none() {
        return Err(AttentionWriteResolution::Conflict {
            current_revision: Some(saved.policy_revision),
        });
    }
    Ok(())
}

/// C5 Task 1：`automation-plan-intent.json` 的版本化读侧。旧格式文件
///（仅含已废弃的 `logical_repository_id`、无 `target`）读入为
/// `LegacyUnbound`（旧代无绑定，可诊断 fail-closed），不把 serde 缺字段
/// 错误泄漏为普通 JSON 读取失败；新格式完整校验后返回。
// 读侧两态同域(旧代诊断/新格式),变体互斥且一次只存活一个;Box 会加解引用噪声。
#[allow(clippy::large_enum_variant)]
pub(crate) enum PreparedPlanIntentRead {
    Current(PreparedPlanIntent),
    /// 旧代意图：无 target。携带身份字段供诊断；绝不驱动自动链。
    // 字段当前无读取点；删除会改变枚举形状并触发 large_enum_variant（禁碰类别），故保留并放行。
    #[allow(dead_code)]
    LegacyUnbound {
        enrollment_id: String,
        prepare_intent_id: String,
    },
}

pub(crate) fn read_prepared_plan_intent(
    path: &Path,
) -> Result<PreparedPlanIntentRead, ProductStoreError> {
    let value: serde_json::Value = read_json(path)?;
    if value
        .get("target")
        .map(serde_json::Value::is_null)
        .unwrap_or(true)
    {
        return Ok(PreparedPlanIntentRead::LegacyUnbound {
            enrollment_id: value
                .get("enrollment_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            prepare_intent_id: value
                .get("prepare_intent_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }
    let intent: PreparedPlanIntent = serde_json::from_value(value).map_err(|error| {
        ProductStoreError::Json(format!("prepared plan intent parse failed: {error}"))
    })?;
    Ok(PreparedPlanIntentRead::Current(intent))
}

/// C5 Task 1：`automation-generation-intent.json` 的版本化读侧（语义同
/// `PreparedPlanIntentRead`：旧格式无 target → 旧代无绑定 fail-closed）。
pub enum PlanGenerationIntentRead {
    Current(PlanGenerationIntent),
    /// 旧代意图：无 target。携带身份字段供诊断；绝不驱动自动链。
    // 字段当前无读取点；删除会改变枚举形状，故保留并放行。
    #[allow(dead_code)]
    LegacyUnbound {
        enrollment_id: String,
        plan_id: String,
    },
}

pub(crate) fn read_plan_generation_intent(
    path: &Path,
) -> Result<PlanGenerationIntentRead, ProductStoreError> {
    let value: serde_json::Value = read_json(path)?;
    if value
        .get("target")
        .map(serde_json::Value::is_null)
        .unwrap_or(true)
    {
        return Ok(PlanGenerationIntentRead::LegacyUnbound {
            enrollment_id: value
                .get("enrollment_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            plan_id: value
                .get("plan_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }
    let intent: PlanGenerationIntent = serde_json::from_value(value).map_err(|error| {
        ProductStoreError::Json(format!("plan generation intent parse failed: {error}"))
    })?;
    Ok(PlanGenerationIntentRead::Current(intent))
}

/// `terminate_plan` 锁内判定；锁外映射为 `EnrollmentTerminationResult`。
enum TerminationResolution {
    Accepted {
        command_id: String,
        terminated_at: String,
        enrollment: IssueAutomationEnrollment,
    },
    Replayed {
        command_id: String,
        terminated_at: String,
        enrollment: IssueAutomationEnrollment,
    },
    Conflict {
        current_revision: Option<u64>,
    },
    Missing,
}

fn resolve_termination(
    resolution: TerminationResolution,
) -> Result<EnrollmentTerminationResult, EnrollmentError> {
    match resolution {
        TerminationResolution::Accepted {
            command_id,
            terminated_at,
            enrollment,
        } => Ok(EnrollmentTerminationResult {
            command_id,
            state: OperationState::Accepted,
            terminated_at,
            enrollment,
        }),
        TerminationResolution::Replayed {
            command_id,
            terminated_at,
            enrollment,
        } => Ok(EnrollmentTerminationResult {
            command_id,
            state: OperationState::Replayed,
            terminated_at,
            enrollment,
        }),
        TerminationResolution::Conflict { current_revision } => {
            Err(EnrollmentError::Conflict { current_revision })
        }
        TerminationResolution::Missing => Err(EnrollmentError::NotFound),
    }
}

/// `rebind` 锁内判定；锁外映射为 `EnrollmentRebindResult`/`EnrollmentError`。
enum RebindResolution {
    Accepted {
        command_id: String,
        enrollment: IssueAutomationEnrollment,
    },
    Replayed {
        command_id: String,
        enrollment: IssueAutomationEnrollment,
    },
    Conflict {
        current_revision: Option<u64>,
    },
    InvalidScope(String),
    Missing,
}

fn resolve_rebind(resolution: RebindResolution) -> Result<EnrollmentRebindResult, EnrollmentError> {
    match resolution {
        RebindResolution::Accepted {
            command_id,
            enrollment,
        } => Ok(EnrollmentRebindResult {
            command_id,
            state: OperationState::Accepted,
            enrollment,
        }),
        RebindResolution::Replayed {
            command_id,
            enrollment,
        } => Ok(EnrollmentRebindResult {
            command_id,
            state: OperationState::Replayed,
            enrollment,
        }),
        RebindResolution::Conflict { current_revision } => {
            Err(EnrollmentError::Conflict { current_revision })
        }
        RebindResolution::InvalidScope(reason) => Err(EnrollmentError::InvalidScope(reason)),
        RebindResolution::Missing => Err(EnrollmentError::NotFound),
    }
}

/// 将 durable 归属注入 SessionState 帧（所有 manager 对外出口统一调用）。
/// 读取失败由调用方决定暴露方式：HTTP 明确报错；WS 帧置 None 保持未知。
pub fn project_session_automation(
    frame: &mut crate::web::workspace_ws_types::WsOutMessage,
    record: &crate::product::models::WorkspaceSessionRecord,
    store: &IssueAutomationStore,
) -> Result<(), ProductStoreError> {
    if let crate::web::workspace_ws_types::WsOutMessage::SessionState { automation, .. } = frame {
        *automation = Some(store.ownership_for_session(record)?);
    }
    Ok(())
}

fn read_optional_enrollment(
    path: &Path,
) -> Result<Option<IssueAutomationEnrollment>, ProductStoreError> {
    if path.metadata().is_err() {
        return Ok(None);
    }
    let saved: IssueAutomationEnrollment = read_json(path)?;
    Ok(Some(saved))
}

fn apply_revision_and_write(
    path: &Path,
    existing: Option<IssueAutomationEnrollment>,
    command: EnrollmentWriteCommand,
    expected_revision: Option<u64>,
) -> Result<CasResolution, ProductStoreError> {
    let now = now_rfc3339();
    let next = match (existing, command) {
        (
            None,
            EnrollmentWriteCommand::Enable {
                command_id,
                selection_key,
                source,
                options,
                target,
            },
        ) => {
            let enrollment_id = Uuid::new_v4().to_string();
            // prepare_intent_id 与 enrollment 同源；P0 不消费该意图。
            let prepare_intent_id = enrollment_id.clone();
            // C5 Task 1：target 必填——Enable 即建立 durable binding v1；
            // plan/session 尚未绑定（绑定写入时补全 current 身份）。
            let binding_history =
                initial_binding_history(&enrollment_id, None, None, &source, &target, &options);
            // REQ-WIGA-09 修订：显式 command_id 的 Enable 进命令账本
            //（人工动作留痕＋同命令重放面）；缺省不进（旧客户端行为）。
            let command_ledger = enable_ledger_entry(
                command_id.as_deref(),
                &selection_key,
                &source,
                &options,
                &target,
                expected_revision,
                1,
            )
            .into_iter()
            .collect();
            IssueAutomationEnrollment {
                enrollment_id,
                selection_key,
                project_id: path_project_id(path),
                issue_id: path_issue_id(path),
                enabled: true,
                policy_revision: 1,
                source,
                options,
                prepare_intent_id,
                plan_id: None,
                session_id: None,
                created_at: now.clone(),
                updated_at: now,
                target: Some(target),
                binding_history: Some(binding_history),
                command_ledger,
                // 批 2 N0：首次 Enable 无终止、无待处理事实；terminated_at
                // 写入归 T1 终止 CAS。
                terminated_at: None,
                attention_facts: Vec::new(),
            }
        }
        // 重开（disabled 后换 payload 的启用）：保留 enrollment 身份与既有
        // 绑定，仅 revision+1。C1 Task 1：target 是授权域冻结事实——已声明
        // target 的漂移一律 fail-closed（换 target 只能走显式 rebind）。
        (
            Some(mut saved),
            EnrollmentWriteCommand::Enable {
                command_id,
                selection_key,
                source,
                options,
                target,
            },
        ) => {
            // 旧 enrollment（无声明）重开时允许显式升级 target；已声明
            // target 的漂移：fail-closed（走显式 rebind）。
            let target_drifted = saved
                .target
                .as_ref()
                .is_some_and(|declared| *declared != target);
            if target_drifted {
                return Ok(CasResolution::Conflict {
                    current_revision: Some(saved.policy_revision),
                });
            }
            // 旧 enrollment（无声明）重开时显式升级 target → 初始化 v1。
            if saved.binding_history.is_none() {
                saved.binding_history = Some(initial_binding_history(
                    &saved.enrollment_id,
                    saved.plan_id.as_deref(),
                    saved.session_id.as_deref(),
                    &source,
                    &target,
                    &options,
                ));
            }
            // REQ-WIGA-09 修订：显式 command_id 的重开同样进命令账本
            //（binding_version 沿当前代，重开不换代）。
            if let Some(entry) = enable_ledger_entry(
                command_id.as_deref(),
                &selection_key,
                &source,
                &options,
                &target,
                expected_revision,
                saved
                    .binding_history
                    .as_ref()
                    .map(|history| history.current.binding_version)
                    .unwrap_or_default(),
            ) {
                saved.command_ledger.push(entry);
            }
            saved.selection_key = selection_key;
            saved.source = source;
            saved.options = options;
            saved.target = Some(target);
            saved.enabled = true;
            saved.policy_revision += 1;
            saved.updated_at = now;
            saved
        }
        (Some(mut saved), EnrollmentWriteCommand::Disable) => {
            saved.enabled = false;
            saved.policy_revision += 1;
            saved.updated_at = now;
            saved
        }
        (None, EnrollmentWriteCommand::Disable) => return Ok(CasResolution::Missing),
    };
    write_json(path, &next)?;
    Ok(CasResolution::Applied(next))
}

/// C1 Task 1：从 durable enrollment 事实构造初始 binding v1（enable 显式
/// 声明 target 时）。plan/session 未绑定时以空串占位，绑定写入时补全。
fn initial_binding_history(
    enrollment_id: &str,
    plan_id: Option<&str>,
    session_id: Option<&str>,
    source: &crate::product::models::automation::EnrollmentSource,
    target: &crate::product::logical_codebase::EnrollmentTarget,
    options: &crate::product::models::automation::EnrollmentOptions,
) -> EnrollmentBindingHistory {
    EnrollmentBindingHistory {
        current: EnrollmentBindingIdentity {
            binding_version: 1,
            enrollment_id: enrollment_id.to_string(),
            plan_id: plan_id.unwrap_or_default().to_string(),
            session_id: session_id.unwrap_or_default().to_string(),
            source: source.clone(),
            target: target.clone(),
            author_provider: options.author_provider.clone(),
            reviewer_provider: options.reviewer_provider.clone(),
        },
        previous: Vec::new(),
    }
}

/// REQ-WIGA-09 修订：Enable 命令（人工入口）的稳定 payload 摘要——
/// `command_id`＋selection_key＋source＋options＋target＋expected_revision
/// 全部参与；同一命令的异 payload/异 revision 必然产生不同 digest
///（命令账本按此 fail-closed）。取代与常规 Enable 留痕共用同一口径。
fn enable_command_payload_digest(
    command_id: &str,
    selection_key: &str,
    source: &EnrollmentSource,
    options: &EnrollmentOptions,
    target: &EnrollmentTarget,
    expected_revision: Option<u64>,
) -> String {
    let payload = serde_json::json!({
        "command_id": command_id,
        "selection_key": selection_key,
        "source": source,
        "options": options,
        "target": target,
        "expected_revision": expected_revision,
    });
    format!(
        "sha256:{:x}",
        Sha256::digest(payload.to_string().as_bytes())
    )
}

/// REQ-WIGA-09 修订：显式 `command_id` 的 Enable 账目（Accepted＋提交时
/// 的 binding_version）；缺省 `None` 返回 `None`（旧客户端不进账本）。
fn enable_ledger_entry(
    command_id: Option<&str>,
    selection_key: &str,
    source: &EnrollmentSource,
    options: &EnrollmentOptions,
    target: &EnrollmentTarget,
    expected_revision: Option<u64>,
    binding_version: u64,
) -> Option<EnrollmentCommandResult> {
    let command_id = command_id?;
    Some(EnrollmentCommandResult {
        command_id: command_id.to_string(),
        payload_digest: enable_command_payload_digest(
            command_id,
            selection_key,
            source,
            options,
            target,
            expected_revision,
        ),
        state: OperationState::Accepted,
        binding_version,
    })
}

/// terminated 人工取代的 Enable 载荷（锁内传参分组）。`original_source_dangling`
/// 由调用方核实「终止记录的原 source 引用实体已删」后置真（悬空源重授权）。
struct SupersedeEnableRequest<'a> {
    command_id: Option<&'a str>,
    selection_key: &'a str,
    source: &'a EnrollmentSource,
    options: &'a EnrollmentOptions,
    target: &'a EnrollmentTarget,
    expected_revision: Option<u64>,
    original_source_dangling: bool,
}

/// REQ-WIGA-09 修订：terminated 的人工显式 Enable 取代（compare_and_set
/// 锁内）。判定顺序：命令账本先判（同 command 同 payload 幂等重放首次
/// durable 结果的当前投影、异 payload fail-closed）→ 稳定 command_id
///（缺 → InvalidScope，人工重新开启必须可留痕、可重放）→ revision 线性化
///（必须显式等于当前 policy_revision）→ 授权不改写（source/options/target
/// 与终止记录逐字段相等；selection_key 是新会话身份不比对；旧代无
/// target 声明时按本次提交显式升级）。取代落盘：同 enrollment_id＋
/// revision+1＋terminated_at 清空＋enabled=true＋plan/session 解绑＋
/// prepare_intent_id 轮换（plan/session id 由它派生，沿用会让 ensure
/// 幂等复用被终止的旧 plan 实体）＋binding 换代（旧代 current 进
/// previous、binding_version+1、plan/session 空串占位）＋账本 Accepted
/// 留痕＋attention 事实随旧授权退役＋旧代伴生意图/生成检查点删除（新链
/// 由 ensure 重新派生，身份比对不再 fail-closed 卡死）。自动路径无从
/// 构造本入口（生产唯一 Enable 调用方是人工 REST PUT）。
fn supersede_terminated_enrollment(
    path: &Path,
    saved: &IssueAutomationEnrollment,
    request: SupersedeEnableRequest<'_>,
) -> Result<CasResolution, ProductStoreError> {
    let SupersedeEnableRequest {
        command_id,
        selection_key,
        source,
        options,
        target,
        expected_revision,
        original_source_dangling,
    } = request;
    let Some(command_id) = command_id else {
        return Ok(CasResolution::InvalidScope(
            "re-enabling a terminated enrollment requires a stable command_id".to_string(),
        ));
    };
    let digest = enable_command_payload_digest(
        command_id,
        selection_key,
        source,
        options,
        target,
        expected_revision,
    );
    if let Some(ledger) = saved
        .command_ledger
        .iter()
        .find(|entry| entry.command_id == command_id)
    {
        return Ok(if ledger.payload_digest == digest {
            CasResolution::Unchanged(saved.clone())
        } else {
            CasResolution::Conflict {
                current_revision: Some(saved.policy_revision),
            }
        });
    }
    if expected_revision != Some(saved.policy_revision) {
        return Ok(CasResolution::Conflict {
            current_revision: Some(saved.policy_revision),
        });
    }
    // 授权不改写：options/target 与终止记录逐字段相等；source 漂移仅在
    // 调用方已核实原 source 悬空（级联删除后引用实体已不存在、原授权
    // 失效）时放行——悬空源重授权＝人工显式按当前已确认 source 重新授权，
    // 留痕由取代落盘的 binding 换代承担（旧代 current 连同悬空 source
    // 只读进 previous、binding_version+1）。旧代无声明（legacy）按本次
    // 提交显式升级（与既有"旧 enrollment 重开时显式升级 target"语义连续）。
    let options_or_target_drifted = &saved.options != options
        || saved
            .target
            .as_ref()
            .is_some_and(|declared| *declared != *target);
    let source_drifted = &saved.source != source;
    if options_or_target_drifted || (source_drifted && !original_source_dangling) {
        return Ok(CasResolution::Conflict {
            current_revision: Some(saved.policy_revision),
        });
    }

    let now = now_rfc3339();
    let mut next = saved.clone();
    // 新链身份：prepare_intent_id 轮换（ensure 派生 plan/session 的唯一
    // 源）；enrollment 身份与 created_at 保留（同记录取代）。
    next.prepare_intent_id = Uuid::new_v4().to_string();
    next.selection_key = selection_key.to_string();
    next.source = source.clone();
    next.options = options.clone();
    next.target = Some(target.clone());
    next.enabled = true;
    next.terminated_at = None;
    next.policy_revision += 1;
    next.updated_at = now;
    next.plan_id = None;
    next.session_id = None;
    next.attention_facts = Vec::new();
    // binding 换代：旧代 current 追加 previous（只读可追溯）；新代以本次
    // 授权重建（plan/session 空串占位，绑定写入时补全）。
    let mut previous = saved
        .binding_history
        .as_ref()
        .map(|history| history.previous.clone())
        .unwrap_or_default();
    let binding_version = saved
        .binding_history
        .as_ref()
        .map(|history| history.current.binding_version + 1)
        .unwrap_or(1);
    if let Some(history) = saved.binding_history.as_ref() {
        previous.push(history.current.clone());
    }
    next.binding_history = Some(EnrollmentBindingHistory {
        current: EnrollmentBindingIdentity {
            binding_version,
            enrollment_id: saved.enrollment_id.clone(),
            plan_id: String::new(),
            session_id: String::new(),
            source: source.clone(),
            target: target.clone(),
            author_provider: options.author_provider.clone(),
            reviewer_provider: options.reviewer_provider.clone(),
        },
        previous,
    });
    next.command_ledger.push(EnrollmentCommandResult {
        command_id: command_id.to_string(),
        payload_digest: digest,
        state: OperationState::Accepted,
        binding_version,
    });
    // 旧代伴生意图/生成检查点随旧授权退役：先删后写（崩溃窗口最多留下
    // "无意图文件的 terminated 记录"，重试取代幂等收敛；反向窗口被
    // write_json 的原子 rename 消除）。
    for stale in [
        path.with_file_name("automation-plan-intent.json"),
        path.with_file_name("automation-generation-intent.json"),
    ] {
        if stale.metadata().is_ok() {
            std::fs::remove_file(&stale).map_err(|error| {
                ProductStoreError::Io(format!(
                    "remove superseded automation intent {}: {error}",
                    stale.display()
                ))
            })?;
        }
    }
    write_json(path, &next)?;
    Ok(CasResolution::Applied(next))
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn path_project_id(path: &Path) -> String {
    path.ancestors()
        .nth(3)
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn path_issue_id(path: &Path) -> String {
    path.parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}
