use super::*;

pub(crate) fn parse_ask_user_question_from_input(
    input: &Value,
    request_id: &str,
) -> ChoiceRequestData {
    let questions = input
        .get("questions")
        .and_then(Value::as_array)
        .map(|questions| {
            questions
                .iter()
                .enumerate()
                .map(parse_choice_question)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let first_question = questions.first();
    let prompt = if questions.len() > 1 {
        format!("请确认 {} 个问题", questions.len())
    } else {
        first_question
            .map(|question| question.prompt.clone())
            .unwrap_or_else(|| "请选择".to_string())
    };
    let options = first_question
        .map(|question| question.options.clone())
        .unwrap_or_default();
    let allow_multiple = first_question
        .map(|question| question.allow_multiple)
        .unwrap_or(false);

    ChoiceRequestData {
        id: request_id.to_string(),
        prompt,
        options,
        allow_multiple,
        allow_free_text: true,
        questions,
        source: ChoiceRequestSource::AskUserQuestion,
    }
}

pub(crate) fn ask_user_question_answers_from_decision(
    input: &Value,
    decision: &ChoiceDecision,
) -> serde_json::Map<String, Value> {
    let mut answers = serde_json::Map::new();
    let Some(questions) = input.get("questions").and_then(Value::as_array) else {
        return answers;
    };

    if !decision.answers.is_empty() {
        for answer in &decision.answers {
            let Some((_, question)) = questions
                .iter()
                .enumerate()
                .find(|(idx, question)| question_id(question, *idx) == answer.question_id)
            else {
                continue;
            };
            if let Some(rendered_answer) = answer_text(question, answer) {
                let question_text = question
                    .get("question")
                    .and_then(Value::as_str)
                    .unwrap_or("question");
                answers.insert(question_text.to_string(), Value::String(rendered_answer));
            }
        }
        return answers;
    }

    let Some(first_question) = questions.first() else {
        return answers;
    };

    let question_text = first_question
        .get("question")
        .and_then(Value::as_str)
        .unwrap_or("question");
    let answer = if let Some(text) = decision
        .free_text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        text.to_string()
    } else if !decision.selected_option_ids.is_empty() {
        selected_option_labels(first_question, &decision.selected_option_ids).join(", ")
    } else {
        String::new()
    };

    if !answer.is_empty() {
        answers.insert(question_text.to_string(), Value::String(answer));
    }
    answers
}

fn parse_choice_question((idx, question): (usize, &Value)) -> ChoiceQuestionData {
    ChoiceQuestionData {
        id: question_id(question, idx),
        prompt: question
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or("请选择")
            .to_string(),
        options: question_options(question),
        allow_multiple: question
            .get("multiSelect")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        allow_free_text: true,
    }
}

fn question_id(question: &Value, idx: usize) -> String {
    question
        .get("id")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("q_{idx}"))
}

fn question_options(question: &Value) -> Vec<ChoiceOptionData> {
    question
        .get("options")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .enumerate()
                .filter_map(|(idx, opt)| {
                    let label = opt.get("label")?.as_str()?.to_string();
                    let description = opt
                        .get("description")
                        .and_then(Value::as_str)
                        .map(String::from);
                    Some(ChoiceOptionData {
                        id: format!("opt_{idx}"),
                        label,
                        description,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn answer_text(question: &Value, answer: &ChoiceAnswerData) -> Option<String> {
    if let Some(text) = answer
        .free_text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return Some(text.to_string());
    }
    if answer.selected_option_ids.is_empty() {
        return None;
    }
    Some(selected_option_labels(question, &answer.selected_option_ids).join(", "))
}

pub(crate) fn selected_option_labels(
    question: &Value,
    selected_option_ids: &[String],
) -> Vec<String> {
    let options = question.get("options").and_then(Value::as_array);
    selected_option_ids
        .iter()
        .map(|id| {
            options
                .and_then(|opts| {
                    let idx = id.strip_prefix("opt_")?.parse::<usize>().ok()?;
                    opts.get(idx)?.get("label")?.as_str().map(String::from)
                })
                .unwrap_or_else(|| id.clone())
        })
        .collect()
}

pub(crate) fn ask_user_question_tool_result_content(
    input: &Value,
    answers: &serde_json::Map<String, Value>,
) -> String {
    let ordered_questions = input
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|question| question.get("question").and_then(Value::as_str));
    let mut rendered_answers = Vec::new();
    for question in ordered_questions {
        if let Some(answer) = answers.get(question) {
            rendered_answers.push(format!(
                "\"{question}\"=\"{}\"",
                render_answer_value(answer)
            ));
        }
    }
    for (question, answer) in answers {
        if !rendered_answers
            .iter()
            .any(|rendered| rendered.starts_with(&format!("\"{question}\"=")))
        {
            rendered_answers.push(format!(
                "\"{question}\"=\"{}\"",
                render_answer_value(answer)
            ));
        }
    }

    if rendered_answers.is_empty() {
        return "Your questions have been answered: no answer was provided. You can now continue with these answers in mind.".to_string();
    }

    format!(
        "Your questions have been answered: {}. You can now continue with these answers in mind.",
        rendered_answers.join(", ")
    )
}

pub(crate) fn render_answer_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    }
}
