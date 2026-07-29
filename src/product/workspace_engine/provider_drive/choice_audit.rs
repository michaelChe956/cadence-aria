use super::*;

pub(super) struct ChoiceResponseAuditInput<'a> {
    pub(super) request: Option<&'a ChoiceRequestData>,
    pub(super) choice_id: &'a str,
    pub(super) selected_option_ids: &'a [String],
    pub(super) free_text: Option<&'a str>,
    pub(super) answers: &'a [ChoiceAnswerData],
    pub(super) node_id: Option<&'a str>,
    pub(super) agent: Option<&'a ProviderName>,
    pub(super) role: &'a ProviderConversationRole,
}

impl WorkspaceEngine {
    pub(super) fn record_choice_response_audit(&mut self, input: ChoiceResponseAuditInput<'_>) {
        let content = build_choice_response_audit_message(&input);
        let msg_id = format!("msg_{:03}", self.session.messages.len() + 1);
        let now = chrono::Utc::now().to_rfc3339();
        self.session.messages.push(SessionMessage {
            id: msg_id,
            role: "system".to_string(),
            content: content.clone(),
            checkpoint_id: None,
            created_at: now,
        });
        if let Some(store) = &self.lifecycle_store {
            let _ = store.append_workspace_message(
                &self.session.session_id,
                "system".to_string(),
                content,
            );
        }
    }
}

fn build_choice_response_audit_message(input: &ChoiceResponseAuditInput<'_>) -> String {
    let source = input
        .request
        .map(|request| request.source.as_str())
        .unwrap_or("unknown");
    let mut message = String::new();
    message.push_str("结构化交互审计记录（daemon 捕获）\n");
    message.push_str("- audit_kind: provider_choice_response\n");
    message.push_str(&format!("- choice_id: {}\n", input.choice_id));
    message.push_str(&format!("- source: {source}\n"));
    message.push_str(&format!("- provider_role: {}\n", role_label(input.role)));
    if let Some(agent) = input.agent {
        message.push_str(&format!("- agent: {agent:?}\n"));
    }
    if let Some(node_id) = input.node_id {
        message.push_str(&format!("- node_id: {node_id}\n"));
    }
    if let Some(request) = input.request {
        message.push_str(&format!(
            "- request_prompt: {}\n",
            audit_one_line(&request.prompt)
        ));
    }
    message.push_str("- answers:\n");
    append_choice_answers_audit(
        &mut message,
        input.request,
        input.selected_option_ids,
        input.free_text,
        input.answers,
    );
    message.push_str(
        "\n说明：以上记录由 daemon 在 ProviderEvent::ChoiceRequest 与 \
         ProviderCommand::ChoiceResponse 之间捕获，是后续 reviewer 核对 artifact 中 \
         author-decision/source claims 的可审计来源。\n",
    );
    message
}

fn append_choice_answers_audit(
    message: &mut String,
    request: Option<&ChoiceRequestData>,
    selected_option_ids: &[String],
    free_text: Option<&str>,
    answers: &[ChoiceAnswerData],
) {
    let questions = request
        .map(|request| request.effective_questions())
        .unwrap_or_default();
    let fallback_options = request
        .map(|request| request.options.as_slice())
        .unwrap_or_default();
    let fallback_free_text = free_text.and_then(trimmed_non_empty).map(str::to_string);
    let normalized_answers = if answers.is_empty() {
        if selected_option_ids.is_empty() && fallback_free_text.is_none() {
            Vec::new()
        } else {
            vec![ChoiceAnswerData {
                question_id: "default".to_string(),
                selected_option_ids: selected_option_ids.to_vec(),
                free_text: fallback_free_text,
            }]
        }
    } else {
        answers.to_vec()
    };

    if normalized_answers.is_empty() {
        message.push_str("  - 未捕获到显式答案。\n");
        return;
    }

    for answer in normalized_answers {
        let question = questions
            .iter()
            .find(|question| question.id == answer.question_id);
        let options = question
            .map(|question| question.options.as_slice())
            .unwrap_or(fallback_options);
        message.push_str(&format!("  - question_id: {}\n", answer.question_id));
        if let Some(question) = question {
            message.push_str(&format!(
                "    question: {}\n",
                audit_one_line(&question.prompt)
            ));
        }
        if !answer.selected_option_ids.is_empty() {
            let labels = answer
                .selected_option_ids
                .iter()
                .map(|option_id| choice_option_label(option_id, options))
                .collect::<Vec<_>>()
                .join("; ");
            message.push_str(&format!("    selected: {labels}\n"));
        }
        if let Some(free_text) = answer.free_text.as_deref().and_then(trimmed_non_empty) {
            message.push_str(&format!("    free_text: {}\n", audit_one_line(free_text)));
        }
    }
}

fn choice_option_label(option_id: &str, options: &[ChoiceOptionData]) -> String {
    match options.iter().find(|option| option.id == option_id) {
        Some(option) if option.label.trim().is_empty() => option_id.to_string(),
        Some(option) => format!("{option_id} = {}", audit_one_line(&option.label)),
        None => format!("{option_id} = <unknown option>"),
    }
}

fn audit_one_line(value: &str) -> String {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn trimmed_non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn role_label(role: &ProviderConversationRole) -> &'static str {
    match role {
        ProviderConversationRole::Author => "author",
        ProviderConversationRole::Reviewer => "reviewer",
        ProviderConversationRole::Coder => "coder",
        ProviderConversationRole::Analyst => "analyst",
        ProviderConversationRole::CodeReviewer => "code_reviewer",
        ProviderConversationRole::InternalReviewer => "internal_reviewer",
    }
}
