use std::collections::BTreeMap;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderPermissionMode, ProviderSession, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::cross_cutting::structured_output::{StructuredOutputContract, StructuredOutputState};
use crate::protocol::contracts::AdapterRole;

use super::models::{ImageCreateError, SessionRecord, TemplateChoice};
use super::templates::{build_iteration_prompt, resolve_guidance};

const ITERATION_TIMEOUT_SECS: u64 = 300;
const SUGGESTED_PROMPT_SCHEMA: &str = "image_create_suggested_prompt";

pub struct PromptIterationEngine {
    registry: Arc<ProviderRegistry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IterationOutcome {
    pub readable_text: String,
    pub suggested_prompt: Option<String>,
    /// 结构化块解析成功但 suggested_prompt 为空/缺失 ⇒ 模型明确未产出（拒答或主动不给）；
    /// false = 正常产出，或结构化输出根本没解析出来（格式不符/模型无视契约）。
    pub prompt_declined: bool,
    pub provider_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IterationHistory {
    pub template_choice: TemplateChoice,
    pub guidance: String,
    pub past_user_messages: Vec<String>,
    pub last_suggested_prompt: Option<String>,
}

impl PromptIterationEngine {
    pub fn new(registry: Arc<ProviderRegistry>) -> Self {
        Self { registry }
    }

    pub async fn iterate(
        &self,
        record: &SessionRecord,
        user_message: &str,
        paths: &AriaStatePaths,
        cancel: CancellationToken,
    ) -> Result<IterationOutcome, ImageCreateError> {
        let session = &record.session;
        let guidance = resolve_guidance(&session.template)?;
        let history = IterationHistory {
            template_choice: session.template.clone(),
            guidance: guidance.clone(),
            past_user_messages: record
                .messages
                .iter()
                .filter(|message| message.role == "user")
                .map(|message| message.content.clone())
                .collect(),
            last_suggested_prompt: session.current_prompt.clone(),
        };
        let working_dir = paths.image_create_session_scratch_dir(&session.id);
        tokio::fs::create_dir_all(&working_dir)
            .await
            .map_err(|error| {
                ImageCreateError::Iteration(format!(
                    "failed to create image iteration working directory: {error}"
                ))
            })?;
        let provider = self.registry.get(&session.provider_name).ok_or_else(|| {
            ImageCreateError::Iteration(format!(
                "image iteration provider {:?} is not available",
                session.provider_name
            ))
        })?;

        let initial_input = iteration_input(
            session.provider_name.clone().into(),
            session.id.clone(),
            build_iteration_prompt(&guidance, user_message),
            working_dir.clone(),
            session.last_provider_session_id.clone(),
        );
        match run_iteration(provider.clone(), initial_input, cancel.clone()).await {
            Ok(outcome) => Ok(outcome),
            Err(error)
                if session.last_provider_session_id.is_some()
                    && is_invalid_resume_error(&error) =>
            {
                let fallback_input = iteration_input(
                    session.provider_name.clone().into(),
                    session.id.clone(),
                    build_iteration_prompt(
                        &guidance,
                        &fallback_user_message(&history, user_message),
                    ),
                    working_dir,
                    None,
                );
                run_iteration(provider, fallback_input, cancel)
                    .await
                    .map_err(IterationRunError::into_image_create_error)
            }
            Err(error) => Err(error.into_image_create_error()),
        }
    }
}

fn iteration_input(
    provider_type: crate::protocol::contracts::ProviderType,
    workspace_session_id: String,
    prompt: String,
    working_dir: std::path::PathBuf,
    resume_provider_session_id: Option<String>,
) -> StreamingProviderInput {
    let nonce = make_nonce();
    let prompt = format!(
        "{prompt}\n\n请严格按以下格式输出：\n<ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">\n{{\"nonce\":\"{nonce}\",\"suggested_prompt\":\"用中文撰写的最终图片生成 prompt\"}}\n</ARIA_STRUCTURED_OUTPUT>\n\n除上面的 JSON 块外不要输出其他内容。\n若你无法或不宜给出图片 prompt（例如内容涉及真实在世人物的私生活传闻、或你判断无法完成），\n仍然必须输出上面的 JSON 块，但把 suggested_prompt 置为空字符串 \"\"，\n并可在 JSON 块之外用一段文字说明原因。\n要求：suggested_prompt 的内容必须用中文撰写（描述主体、场景、风格、构图等），不要用英文。\n"
    );
    StreamingProviderInput {
        working_directory: None,
        baseline_tree: None,
        tool_policy: None,
        audit_sink: None,
        provider_type,
        role: AdapterRole::Executor,
        prompt,
        working_dir,
        workspace_session_id: Some(workspace_session_id),
        resume_provider_session_id,
        permission_mode: ProviderPermissionMode::Auto,
        structured_output_contract: Some(StructuredOutputContract {
            nonce,
            schema_name: SUGGESTED_PROMPT_SCHEMA.to_string(),
        }),
        env_vars: BTreeMap::new(),
        timeout_secs: ITERATION_TIMEOUT_SECS,
    }
}

async fn run_iteration(
    provider: Arc<dyn StreamingProviderAdapter>,
    input: StreamingProviderInput,
    cancel: CancellationToken,
) -> Result<IterationOutcome, IterationRunError> {
    let start = provider.start(input, cancel.clone());
    tokio::pin!(start);
    let mut session = tokio::select! {
        _ = cancel.cancelled() => return Err(IterationRunError::Runtime("image iteration cancelled".to_string())),
        result = &mut start => result.map_err(IterationRunError::Start)?,
    };
    consume_iteration_events(&mut session, cancel).await
}

async fn consume_iteration_events(
    session: &mut ProviderSession,
    cancel: CancellationToken,
) -> Result<IterationOutcome, IterationRunError> {
    let mut text_deltas = String::new();
    loop {
        let event = tokio::select! {
            _ = cancel.cancelled() => return Err(IterationRunError::Runtime("image iteration cancelled".to_string())),
            event = session.events.recv() => event,
        };
        match event {
            Some(ProviderEvent::TextDelta { content }) => text_deltas.push_str(&content),
            Some(ProviderEvent::Completed(completion)) => {
                // 无论 parse 成功/失败，readable_output 或 text_deltas 都可能残留 sentinel 块，
                // 统一剥离，确保聊天框不显示 <ARIA_STRUCTURED_OUTPUT> 标签。
                let raw_readable = if !completion.readable_output.trim().is_empty() {
                    completion.readable_output.clone()
                } else {
                    text_deltas
                };
                let readable_text = strip_structured_output_block(&raw_readable);
                // 双出口契约：结构化块解析成功但没有非空 suggested_prompt ⇒ 模型明确未产出
                // （拒答或主动不给）；与「格式不符导致解析失败」（declined 保持 false）区分开。
                let mut prompt_declined = false;
                let suggested_prompt = match completion.structured_output {
                    StructuredOutputState::Parsed(value) => {
                        let prompt = value
                            .get("suggested_prompt")
                            .and_then(serde_json::Value::as_str)
                            .map(str::trim)
                            .filter(|prompt| !prompt.is_empty())
                            .map(str::to_string);
                        if prompt.is_none() {
                            prompt_declined = true;
                        }
                        prompt
                    }
                    StructuredOutputState::NotRequested | StructuredOutputState::Failed(_) => None,
                };
                return Ok(IterationOutcome {
                    readable_text,
                    suggested_prompt,
                    prompt_declined,
                    provider_session_id: completion.provider_session_id,
                });
            }
            Some(ProviderEvent::Failed { message }) => {
                return Err(IterationRunError::Runtime(message));
            }
            Some(ProviderEvent::ProtocolError { code, message, .. }) => {
                return Err(IterationRunError::Runtime(format!("{code}: {message}")));
            }
            Some(ProviderEvent::PermissionTimeout { permission_id }) => {
                return Err(IterationRunError::Runtime(format!(
                    "permission request {permission_id} timed out"
                )));
            }
            Some(ProviderEvent::PermissionRequest(_)) | Some(ProviderEvent::ChoiceRequest(_)) => {
                return Err(IterationRunError::Runtime(
                    "image iteration provider requested unsupported interaction".to_string(),
                ));
            }
            Some(ProviderEvent::StatusChanged(_))
            | Some(ProviderEvent::Execution(_))
            | Some(ProviderEvent::ToolCall(_))
            | Some(ProviderEvent::ToolResult(_))
            | Some(ProviderEvent::UsageReport(_))
            | Some(ProviderEvent::ToolPolicyDecision(_))
            | Some(ProviderEvent::ToolPolicyWarning(_))
            | Some(ProviderEvent::ToolPolicyTerminated(_))
            | Some(ProviderEvent::TerminalActivity(_)) => {}
            None => {
                return Err(IterationRunError::Runtime(
                    "image iteration provider stream closed before completion".to_string(),
                ));
            }
        }
    }
}

fn fallback_user_message(history: &IterationHistory, user_message: &str) -> String {
    let mut sections = Vec::new();
    if !history.past_user_messages.is_empty() {
        sections.push(format!(
            "历史用户输入：\n{}",
            history.past_user_messages.join("\n")
        ));
    }
    if let Some(prompt) = history.last_suggested_prompt.as_deref() {
        sections.push(format!("上一轮 suggested prompt：\n{prompt}"));
    }
    sections.push(format!("当前用户输入：\n{user_message}"));
    sections.join("\n\n")
}

/// 剥离流式文本里的结构化输出块（`<ARIA_STRUCTURED_OUTPUT ...>...</ARIA_STRUCTURED_OUTPUT ...>`）。
/// 循环剥离所有完整配对：拒答场景模型可能在块外说明文字里逐字复读多处模板，
/// 从第二处起残留的标签原文会随 text 事件推给前端并落库。
/// 当 provider 把整个输出都包在结构化块里时，`readable_output` 为空，需对 fallback 的 text_deltas 剥离。
fn strip_structured_output_block(text: &str) -> String {
    const START: &str = "<ARIA_STRUCTURED_OUTPUT";
    const END: &str = "</ARIA_STRUCTURED_OUTPUT";
    let mut stripped = text.to_string();
    while let Some(start) = stripped.find(START) {
        // 从 END 开始找其后的 '>'（结束标签形如 </ARIA_STRUCTURED_OUTPUT>）
        let Some(end_tag_start) = stripped[start..].find(END) else {
            break;
        };
        let end_tag_start = start + end_tag_start;
        let after_end_tag = &stripped[end_tag_start + END.len()..];
        let end_close = after_end_tag
            .find('>')
            .map(|i| end_tag_start + END.len() + i + 1);
        let Some(end_close) = end_close else {
            break;
        };
        stripped.drain(start..end_close);
    }
    stripped.trim().to_string()
}

fn make_nonce() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

fn is_invalid_resume_error(error: &IterationRunError) -> bool {
    let message = error.searchable_message().to_ascii_lowercase();
    let has_session_context = message.contains("session");
    let explicit_resume_failure = message.contains("resume") && has_session_context;
    let missing_or_expired_session = has_session_context
        && ["not found", "missing", "expired", "invalid", "unknown"]
            .iter()
            .any(|marker| message.contains(marker));
    explicit_resume_failure || missing_or_expired_session
}

enum IterationRunError {
    Start(ProviderAdapterError),
    Runtime(String),
}

impl IterationRunError {
    fn searchable_message(&self) -> String {
        match self {
            Self::Start(error) => format!("{} {} {}", error.details, error.stdout, error.stderr),
            Self::Runtime(message) => message.clone(),
        }
    }

    fn into_image_create_error(self) -> ImageCreateError {
        ImageCreateError::Iteration(match self {
            Self::Start(error) => {
                let message = if !error.stderr.trim().is_empty() {
                    error.stderr
                } else if !error.stdout.trim().is_empty() {
                    error.stdout
                } else {
                    error.to_string()
                };
                format!("provider failed to start image iteration: {message}")
            }
            Self::Runtime(message) => format!("provider image iteration failed: {message}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use async_trait::async_trait;
    use chrono::Utc;
    use serde_json::{Value, json};
    use tempfile::tempdir;
    use tokio::sync::{Mutex, mpsc};

    use super::*;
    use crate::cross_cutting::streaming_provider::{
        ProviderCommand, ProviderCompletion, ProviderSession,
    };
    use crate::product::image_create::models::{
        ChatMessage, ImageCreateSession, PresetTemplate, SessionStatus,
    };
    use crate::product::models::ProviderName;

    enum Script {
        StartFails(String),
        Complete {
            text: String,
            structured: Option<Value>,
            session: Option<String>,
        },
        /// 块内 JSON 原文直出（`{NONCE}` 占位符在构造时替换为真实 nonce）：
        /// 用于表达「结构化块里没有 suggested_prompt 键」这类经 Value 封装
        /// 会改写形状（缺键会被补成 null）的输入。
        CompleteRawBlock {
            text: String,
            block_json: String,
            session: Option<String>,
        },
    }

    struct ScriptedProvider {
        scripts: Arc<Mutex<VecDeque<Script>>>,
        captured: Arc<Mutex<Vec<StreamingProviderInput>>>,
    }

    impl ScriptedProvider {
        fn new(scripts: impl IntoIterator<Item = Script>) -> Self {
            Self {
                scripts: Arc::new(Mutex::new(scripts.into_iter().collect())),
                captured: Arc::new(Mutex::new(Vec::new())),
            }
        }

        async fn captured(&self) -> Vec<StreamingProviderInput> {
            self.captured.lock().await.clone()
        }
    }

    #[async_trait]
    impl StreamingProviderAdapter for ScriptedProvider {
        async fn start(
            &self,
            input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            self.captured.lock().await.push(input);
            let contract = self
                .captured
                .lock()
                .await
                .last()
                .and_then(|input| input.structured_output_contract.clone());
            match self.scripts.lock().await.pop_front() {
                Some(Script::StartFails(message)) => Err(ProviderAdapterError::execution_failed(
                    None,
                    String::new(),
                    message,
                    0,
                )),
                Some(Script::Complete {
                    text,
                    structured,
                    session,
                }) => {
                    let full_output = match (contract.as_ref(), structured) {
                        (Some(contract), Some(value)) => format!(
                            "{text}\n<ARIA_STRUCTURED_OUTPUT nonce=\"{}\">\n{}\n</ARIA_STRUCTURED_OUTPUT>",
                            contract.nonce,
                            serde_json::json!({"nonce": contract.nonce, "suggested_prompt": value["suggested_prompt"]})
                        ),
                        _ => text.clone(),
                    };
                    scripted_completion_session(full_output, contract.as_ref(), session)
                }
                Some(Script::CompleteRawBlock {
                    text,
                    block_json,
                    session,
                }) => {
                    let full_output = match contract.as_ref() {
                        Some(contract) => format!(
                            "{text}\n<ARIA_STRUCTURED_OUTPUT nonce=\"{}\">\n{}\n</ARIA_STRUCTURED_OUTPUT>",
                            contract.nonce,
                            block_json.replace("{NONCE}", &contract.nonce)
                        ),
                        None => text.clone(),
                    };
                    scripted_completion_session(full_output, contract.as_ref(), session)
                }
                None => Err(ProviderAdapterError::execution_failed(
                    None,
                    String::new(),
                    "script exhausted",
                    0,
                )),
            }
        }
    }

    /// 由完整输出构造流式会话：先推 TextDelta 再推 Completed，对齐真实 provider 的流式顺序。
    fn scripted_completion_session(
        full_output: String,
        contract: Option<&StructuredOutputContract>,
        session: Option<String>,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let completion = ProviderCompletion::from_output(full_output.clone(), contract, session);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(1);
        tokio::spawn(async move {
            if event_tx
                .send(ProviderEvent::TextDelta {
                    content: full_output,
                })
                .await
                .is_err()
            {
                return;
            }
            let _ = event_tx.send(ProviderEvent::Completed(completion)).await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    fn record() -> SessionRecord {
        SessionRecord {
            session: ImageCreateSession {
                id: "image-session-1".to_string(),
                provider_name: ProviderName::Fake,
                template: TemplateChoice {
                    preset: Some(PresetTemplate::PptBusinessIllustration),
                    custom: None,
                },
                last_provider_session_id: Some("stale-provider-session".to_string()),
                current_prompt: Some("上一轮建议：蓝色等距产品插画".to_string()),
                status: SessionStatus::Active,
                created_at: Utc::now(),
            },
            messages: vec![
                ChatMessage {
                    role: "assistant".to_string(),
                    content: "请描述画面".to_string(),
                    ts: Utc::now(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: "过去需求：画云端部署场景".to_string(),
                    ts: Utc::now(),
                },
            ],
            prompt_blocks: vec![],
            generation_results: vec![],
            events: vec![],
            generation: 0,
        }
    }

    fn engine(provider: Arc<ScriptedProvider>) -> PromptIterationEngine {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::Fake, provider);
        PromptIterationEngine::new(Arc::new(registry))
    }

    #[test]
    fn strips_every_structured_output_block_pair_not_only_the_first() {
        // 拒答场景：模型在块外说明文字里逐字复读模板，出现两处完整标签对 ⇒
        // 必须全部剥离；从第二处起残留的标签原文会随 text 事件推给前端并落库。
        let text = "无法产出。\n<ARIA_STRUCTURED_OUTPUT nonce=\"abcd1234\">{\"suggested_prompt\":\"\"}</ARIA_STRUCTURED_OUTPUT>\n原因是隐私政策。\n<ARIA_STRUCTURED_OUTPUT nonce=\"abcd1234\">{\"suggested_prompt\":\"\"}</ARIA_STRUCTURED_OUTPUT>\n以上。";

        let stripped = strip_structured_output_block(text);

        assert!(
            !stripped.contains("<ARIA_STRUCTURED_OUTPUT"),
            "第二处开始标签残留：{stripped}"
        );
        assert!(
            !stripped.contains("</ARIA_STRUCTURED_OUTPUT"),
            "第二处结束标签残留：{stripped}"
        );
        assert!(stripped.contains("无法产出。"));
        assert!(stripped.contains("原因是隐私政策。"));
        assert!(stripped.contains("以上。"));
    }

    #[test]
    fn strips_single_structured_output_block_pair_as_before() {
        // 回归：只有一个配对时行为与原实现一致（剥离一次 + 首尾 trim）。
        let text = "  说明文字\n<ARIA_STRUCTURED_OUTPUT nonce=\"abcd1234\">{\"a\":1}</ARIA_STRUCTURED_OUTPUT>\n收尾  ";

        let stripped = strip_structured_output_block(text);

        assert_eq!(stripped, "说明文字\n\n收尾");
    }

    #[tokio::test]
    async fn normal_parses_suggested_prompt_and_uses_eight_character_nonce() {
        let provider = Arc::new(ScriptedProvider::new([Script::Complete {
            text: "我已优化画面构图。".to_string(),
            structured: Some(json!({"suggested_prompt": "专业蓝色等距云端部署插画"})),
            session: Some("provider-session-2".to_string()),
        }]));
        let engine = engine(provider.clone());
        let root = tempdir().unwrap();
        let paths = AriaStatePaths::from_workspace_root(root.path());

        let outcome = engine
            .iterate(
                &record(),
                "增加安全盾牌元素",
                &paths,
                CancellationToken::new(),
            )
            .await
            .unwrap();

        assert_eq!(outcome.readable_text, "我已优化画面构图。");
        assert!(!outcome.readable_text.contains("<ARIA_STRUCTURED_OUTPUT"));
        assert_eq!(
            outcome.suggested_prompt.as_deref(),
            Some("专业蓝色等距云端部署插画")
        );
        assert_eq!(
            outcome.provider_session_id.as_deref(),
            Some("provider-session-2")
        );
        let inputs = provider.captured().await;
        let contract = inputs[0].structured_output_contract.as_ref().unwrap();
        assert_eq!(contract.nonce.len(), 8);
        assert!(
            contract
                .nonce
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric())
        );
        assert_eq!(contract.schema_name, SUGGESTED_PROMPT_SCHEMA);
        let start_tag = format!("<ARIA_STRUCTURED_OUTPUT nonce=\"{}\">", contract.nonce);
        let end_tag = "</ARIA_STRUCTURED_OUTPUT>";
        assert!(inputs[0].prompt.contains(&start_tag));
        assert!(inputs[0].prompt.contains(&format!(
            "{{\"nonce\":\"{}\",\"suggested_prompt\"",
            contract.nonce
        )));
        assert!(inputs[0].prompt.contains(end_tag));
        assert!(inputs[0].working_dir.is_dir());
        assert!(
            inputs[0]
                .prompt
                .contains("除上面的 JSON 块外不要输出其他内容")
        );
        assert!(!outcome.prompt_declined);
        assert!(
            inputs[0].prompt.contains("仍然必须输出上面的 JSON 块")
                && inputs[0]
                    .prompt
                    .contains("并可在 JSON 块之外用一段文字说明原因")
        );
    }

    #[tokio::test]
    async fn parse_fallback_keeps_readable_text_without_structured_prompt() {
        let provider = Arc::new(ScriptedProvider::new([Script::Complete {
            text: "仍然可以展示这段可读回复。".to_string(),
            structured: None,
            session: None,
        }]));
        let engine = engine(provider);
        let root = tempdir().unwrap();
        let paths = AriaStatePaths::from_workspace_root(root.path());

        let outcome = engine
            .iterate(&record(), "换成暖色", &paths, CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(outcome.readable_text, "仍然可以展示这段可读回复。");
        assert_eq!(outcome.suggested_prompt, None);
        // 结构化输出根本没解析出来（格式不符/无视契约）⇒ declined 必须保持 false，
        // 与「模型明确未产出」严格区分（DB-03 分界）。
        assert!(!outcome.prompt_declined);
    }

    #[tokio::test]
    async fn declined_empty_suggested_prompt_marks_prompt_declined() {
        let provider = Arc::new(ScriptedProvider::new([Script::Complete {
            text: "涉及真实在世人物的私生活传闻，本轮无法产出图片 prompt。".to_string(),
            structured: Some(json!({"suggested_prompt": ""})),
            session: Some("provider-session-3".to_string()),
        }]));
        let engine = engine(provider);
        let root = tempdir().unwrap();
        let paths = AriaStatePaths::from_workspace_root(root.path());

        let outcome = engine
            .iterate(
                &record(),
                "画某在世人物私生活场景",
                &paths,
                CancellationToken::new(),
            )
            .await
            .unwrap();

        // 结构化块解析成功但 suggested_prompt 为空串 ⇒ 模型明确拒答，而不是解析失败。
        assert_eq!(outcome.suggested_prompt, None);
        assert!(outcome.prompt_declined);
        assert_eq!(
            outcome.readable_text,
            "涉及真实在世人物的私生活传闻，本轮无法产出图片 prompt。"
        );
    }

    #[tokio::test]
    async fn declined_missing_suggested_prompt_key_marks_prompt_declined() {
        // DB-06①：结构化块解析成功、但 JSON 里根本没有 suggested_prompt 键 ⇒
        // 模型明确未产出（区别于格式不符的解析失败）：suggested_prompt=None 且 declined=true。
        let provider = Arc::new(ScriptedProvider::new([Script::CompleteRawBlock {
            text: "本轮不便给出图片 prompt。".to_string(),
            block_json: r#"{"nonce":"{NONCE}"}"#.to_string(),
            session: Some("provider-session-4".to_string()),
        }]));
        let engine = engine(provider);
        let root = tempdir().unwrap();
        let paths = AriaStatePaths::from_workspace_root(root.path());

        let outcome = engine
            .iterate(&record(), "画一张图", &paths, CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(outcome.suggested_prompt, None);
        assert!(
            outcome.prompt_declined,
            "JSON 块缺 suggested_prompt 键 ⇒ 模型明确未产出，declined 必须为 true"
        );
    }

    #[tokio::test]
    async fn declined_whitespace_only_suggested_prompt_marks_prompt_declined() {
        // DB-06②：suggested_prompt 的值只含空白字符 ⇒ trim 后为空 ⇒ 同样视作明确未产出。
        let provider = Arc::new(ScriptedProvider::new([Script::Complete {
            text: "涉及隐私传闻，本轮无法产出图片 prompt。".to_string(),
            structured: Some(json!({"suggested_prompt": "   "})),
            session: Some("provider-session-5".to_string()),
        }]));
        let engine = engine(provider);
        let root = tempdir().unwrap();
        let paths = AriaStatePaths::from_workspace_root(root.path());

        let outcome = engine
            .iterate(&record(), "画一张图", &paths, CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(outcome.suggested_prompt, None);
        assert!(
            outcome.prompt_declined,
            "suggested_prompt 只含空白 ⇒ 模型明确未产出，declined 必须为 true"
        );
    }

    /// DB-08 判据：迭代 prompt 的首句指令必须是新措辞，旧措辞不得以任何独立形式出现。
    /// 新句「除上面的 JSON 块外不要输出其他内容。」本身包含子串「不要输出其他内容」，
    /// 简单 `!contains` 无法区分新旧 ⇒ 判据取：该子串的**每一次**出现都必须紧跟新句前缀
    /// 「除上面的 JSON 块外」之后。旧句「（不要输出其他内容）」与任何裸句式都不满足
    /// 前缀条件，必被抓住；同时要求至少出现一次，钉死新句存在。
    fn assert_no_standalone_old_wording(prompt: &str) {
        const OLD_PHRASE: &str = "不要输出其他内容";
        const NEW_PREFIX: &str = "除上面的 JSON 块外";
        let mut occurrences = 0;
        for index in prompt.match_indices(OLD_PHRASE).map(|(index, _)| index) {
            assert!(
                prompt[..index].ends_with(NEW_PREFIX),
                "旧措辞「{OLD_PHRASE}」不得脱离新句前缀「{NEW_PREFIX}」独立出现：{prompt}"
            );
            occurrences += 1;
        }
        assert!(
            occurrences >= 1,
            "新措辞首句「{NEW_PREFIX}{OLD_PHRASE}。」必须存在：{prompt}"
        );
    }

    #[test]
    fn db08_criterion_accepts_new_wording() {
        // 判据自证（放行方向）：新措辞连同尾句号、上下文必须原样通过。
        let prompt = "请严格按以下格式输出：\n</ARIA_STRUCTURED_OUTPUT>\n\n除上面的 JSON 块外不要输出其他内容。\n若你无法或不宜给出图片 prompt，\n";
        assert_no_standalone_old_wording(prompt);
    }

    #[test]
    #[should_panic(expected = "旧措辞")]
    fn db08_criterion_rejects_historical_parenthesized_old_wording() {
        // 判据自证（拒绝方向一）：256ee427 之前的迭代 prompt 原文含旧句
        // 「（不要输出其他内容）」（见 git 49a39400），必须被抓住。
        let prompt = "请严格按以下格式输出（不要输出其他内容）：\n<ARIA_STRUCTURED_OUTPUT nonce=\"a1b2c3d4\">\n{}\n</ARIA_STRUCTURED_OUTPUT>\n";
        assert_no_standalone_old_wording(prompt);
    }

    #[test]
    #[should_panic(expected = "旧措辞")]
    fn db08_criterion_rejects_bare_old_wording() {
        // 判据自证（拒绝方向二）：即使新句存在，脱离新句前缀的裸旧句式
        // （如「要求只输出 JSON，不要输出其他内容」）同样必须被抓住。
        let prompt = "要求只输出 JSON，不要输出其他内容。\n除上面的 JSON 块外不要输出其他内容。\n";
        assert_no_standalone_old_wording(prompt);
    }

    #[tokio::test]
    async fn iteration_prompt_first_sentence_is_new_wording_not_old() {
        // DB-08：迭代 prompt 首句为新措辞「除上面的 JSON 块外不要输出其他内容。」；
        // 方案 7.7 的字面旧措辞「（不要输出其他内容）」不得残留，
        // 且任何裸旧句式都不存在（assert_no_standalone_old_wording 强判据）。
        let provider = Arc::new(ScriptedProvider::new([Script::Complete {
            text: "已按要求优化。".to_string(),
            structured: Some(json!({"suggested_prompt": "优化后的建议"})),
            session: Some("provider-session-6".to_string()),
        }]));
        let engine = engine(provider.clone());
        let root = tempdir().unwrap();
        let paths = AriaStatePaths::from_workspace_root(root.path());

        engine
            .iterate(&record(), "调整构图", &paths, CancellationToken::new())
            .await
            .unwrap();

        let prompt = provider.captured().await[0].prompt.clone();
        assert!(prompt.contains("除上面的 JSON 块外不要输出其他内容。"));
        assert!(
            !prompt.contains("（不要输出其他内容）"),
            "旧措辞（带括号形态）不得残留"
        );
        assert_no_standalone_old_wording(&prompt);
    }

    #[tokio::test]
    async fn ordinary_invalid_request_does_not_trigger_fresh_session_fallback() {
        let provider = Arc::new(ScriptedProvider::new([
            Script::StartFails("invalid request: prompt is too long".to_string()),
            Script::Complete {
                text: "must not be reached".to_string(),
                structured: None,
                session: None,
            },
        ]));
        let engine = engine(provider.clone());
        let root = tempdir().unwrap();
        let paths = AriaStatePaths::from_workspace_root(root.path());

        let error = engine
            .iterate(
                &record(),
                "当前需求：增加安全盾牌",
                &paths,
                CancellationToken::new(),
            )
            .await
            .expect_err("ordinary provider errors must be returned directly");

        assert!(
            error
                .to_string()
                .contains("invalid request: prompt is too long")
        );
        assert_eq!(provider.captured().await.len(), 1);
    }

    #[tokio::test]
    async fn resume_fallback_retries_fresh_with_history_in_prompt() {
        let provider = Arc::new(ScriptedProvider::new([
            Script::StartFails("provider session is invalid".to_string()),
            Script::Complete {
                text: "已从历史恢复并优化。".to_string(),
                structured: Some(json!({"suggested_prompt": "恢复后的建议"})),
                session: Some("fresh-provider-session".to_string()),
            },
        ]));
        let engine = engine(provider.clone());
        let root = tempdir().unwrap();
        let paths = AriaStatePaths::from_workspace_root(root.path());

        let outcome = engine
            .iterate(
                &record(),
                "当前需求：增加安全盾牌",
                &paths,
                CancellationToken::new(),
            )
            .await
            .unwrap();

        assert_eq!(outcome.suggested_prompt.as_deref(), Some("恢复后的建议"));
        let inputs = provider.captured().await;
        assert_eq!(inputs.len(), 2);
        assert_eq!(
            inputs[0].resume_provider_session_id.as_deref(),
            Some("stale-provider-session")
        );
        assert_eq!(inputs[1].resume_provider_session_id, None);
        assert!(inputs[1].prompt.contains("过去需求：画云端部署场景"));
        assert!(inputs[1].prompt.contains("上一轮建议：蓝色等距产品插画"));
        assert!(inputs[1].prompt.contains("当前需求：增加安全盾牌"));
        for input in &inputs {
            let contract = input.structured_output_contract.as_ref().unwrap();
            assert_eq!(contract.nonce.len(), 8);
            assert_ne!(contract.nonce, "00000000");
            assert!(input.prompt.contains(&format!(
                "<ARIA_STRUCTURED_OUTPUT nonce=\"{}\">",
                contract.nonce
            )));
            assert!(
                input
                    .prompt
                    .contains(&"</ARIA_STRUCTURED_OUTPUT>".to_string())
            );
        }
    }
}
