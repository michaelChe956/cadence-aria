use crate::protocol::constraints::OpenSpecConstraintBundle;
use chrono::{SecondsFormat, Utc};

pub fn provider_run_id(task_id: &str, node_id: &str) -> String {
    format!("prun_{}_{}", task_id, node_id.to_lowercase())
}

pub fn proposal_constraint_summary(bundle: &OpenSpecConstraintBundle) -> String {
    format!(
        "proposal business_intent={} scope={}",
        bundle.proposal_constraints.business_intent.join(" | "),
        bundle.proposal_constraints.scope.join(" | ")
    )
}

pub fn requirement_constraint_summary(bundle: &OpenSpecConstraintBundle) -> String {
    format!(
        "requirement_ids={}",
        bundle.requirement_constraints.requirement_ids.join(",")
    )
}

pub(crate) fn openspec_id(value: &str) -> String {
    value.to_ascii_uppercase()
}

pub(crate) fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn ids_from_markdown(markdown: &str, prefix: &str) -> Vec<String> {
    let normalized = markdown
        .replace(['[', ']', '(', ')', ',', ';', ':', '.', '`'], " ")
        .replace('\t', " ");
    let mut ids = Vec::new();
    for token in normalized.split_whitespace() {
        let trimmed = token
            .trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '-');
        if trimmed.starts_with(prefix) && !ids.iter().any(|existing| existing == trimmed) {
            ids.push(trimmed.to_string());
        }
    }
    ids
}

pub(crate) fn now_iso8601() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}
