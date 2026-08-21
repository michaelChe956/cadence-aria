use super::*;
use crate::cross_cutting::structured_output::StructuredOutputContract;
use crate::product::workspace_engine::review::ReviewCompletionError;

impl WorkspaceEngine {
    pub(crate) fn build_review_repair_input(
        &self,
        base_input: &StreamingProviderInput,
        completion: &ProviderCompletion,
        error: &ReviewCompletionError,
        provider_session_id: Option<String>,
    ) -> Result<StreamingProviderInput, String> {
        let provider_session_id =
            provider_session_id.filter(|session_id| !session_id.trim().is_empty());
        let nonce = structured_output_nonce();
        let recoverable_value = error
            .recoverable_value()
            .ok_or_else(|| "structured output repair requires recoverable JSON".to_string())?;
        let recoverable_json = serde_json::to_string_pretty(recoverable_value)
            .map_err(|error| format!("serialize recoverable review JSON failed: {error}"))?;
        let schema_name = base_input
            .structured_output_contract
            .as_ref()
            .map(|contract| contract.schema_name.clone())
            .unwrap_or_else(|| "workspace_review".to_string());
        let prompt = format!(
            "上一轮审核业务内容已经完成，但结构化输出格式无效。\n\
             只能修复 JSON 与 ARIA_STRUCTURED_OUTPUT 封装；不得重新审核，不得改变 verdict、summary、findings、review_scope、affects_items 或其他业务字段。\n\
             schema_name: {}\n\
             error_code: {}\n\
             已恢复的原业务 JSON（必须逐字段保持语义一致）：\n{}\n\
             原始输出：\n{}\n\
             请只返回以下 nonce block，不要输出其他说明：\n\
             <ARIA_STRUCTURED_OUTPUT nonce=\"{}\">\n{{\"nonce\":\"{}\",修复后的原业务 JSON}}\n</ARIA_STRUCTURED_OUTPUT>\n",
            schema_name,
            error.code(),
            recoverable_json,
            completion.full_output,
            nonce,
            nonce,
        );
        Ok(StreamingProviderInput {
            provider_type: base_input.provider_type.clone(),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir: base_input.working_dir.clone(),
            workspace_session_id: base_input.workspace_session_id.clone(),
            resume_provider_session_id: provider_session_id,
            permission_mode: permission_mode_for_provider_type(
                &base_input.provider_type,
                self.session.permission_modes.reviewer.clone(),
            ),
            structured_output_contract: Some(StructuredOutputContract { nonce, schema_name }),
            env_vars: base_input.env_vars.clone(),
            timeout_secs: base_input.timeout_secs,
        })
    }
}
