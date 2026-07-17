use super::{
    ProjectionRenderError, ProviderProjectionRenderer, ProviderRenderProfile,
    render_coder_with_profile, render_reviewer_with_profile,
};
use crate::product::work_item_projection::{
    CoderExecutionEnvelope, CoderWorkItemProjection, RenderedExecutionContext,
    ReviewerExecutionEnvelope, ReviewerWorkItemProjection,
};

const PROFILE: ProviderRenderProfile = ProviderRenderProfile {
    provider_label: "Codex",
    renderer_version: "codex-provider-projection-renderer-v1",
    permission_and_tool_hint: "Use Codex tools within repository permissions and request approval when required.",
    structured_output_wrapper: "Return the requested Codex result without altering the normative projection sections.",
};

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct CodexProjectionRenderer;

impl ProviderProjectionRenderer for CodexProjectionRenderer {
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
