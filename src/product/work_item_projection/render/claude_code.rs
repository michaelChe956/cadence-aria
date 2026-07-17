use super::{
    ProjectionRenderError, ProviderProjectionRenderer, ProviderRenderProfile,
    render_coder_with_profile, render_reviewer_with_profile,
};
use crate::product::work_item_projection::{
    CoderExecutionEnvelope, CoderWorkItemProjection, RenderedExecutionContext,
    ReviewerExecutionEnvelope, ReviewerWorkItemProjection,
};

const PROFILE: ProviderRenderProfile = ProviderRenderProfile {
    provider_label: "Claude Code",
    renderer_version: "claude-code-provider-projection-renderer-v1",
    permission_and_tool_hint: "Use Claude Code tools within repository permissions and pause for approval when required.",
    structured_output_wrapper: "Return the requested Claude Code result without altering the normative projection sections.",
};

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ClaudeCodeProjectionRenderer;

impl ProviderProjectionRenderer for ClaudeCodeProjectionRenderer {
    fn render_coder(
        &self,
        projection: &CoderWorkItemProjection,
        envelope: &CoderExecutionEnvelope,
    ) -> Result<RenderedExecutionContext, ProjectionRenderError> {
        render_coder_with_profile(PROFILE, projection, envelope)
    }

    fn render_reviewer(
        &self,
        projection: &ReviewerWorkItemProjection,
        envelope: &ReviewerExecutionEnvelope,
    ) -> Result<RenderedExecutionContext, ProjectionRenderError> {
        render_reviewer_with_profile(PROFILE, projection, envelope)
    }
}
