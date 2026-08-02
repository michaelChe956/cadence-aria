use super::{
    ProjectionRenderError, ProviderProjectionRenderer, ProviderRenderProfile,
    render_coder_with_profile, render_reviewer_with_profile,
};
use crate::product::work_item_projection::{
    CoderExecutionEnvelope, CoderWorkItemProjection, RenderedExecutionContext,
    ReviewerExecutionEnvelope, ReviewerWorkItemProjection,
};

const PROFILE: ProviderRenderProfile = ProviderRenderProfile {
    provider_label: "Pi",
    renderer_version: "pi-provider-projection-renderer-v1",
    permission_and_tool_hint: "Use Pi tools within repository permissions; Pi runs in Auto mode without per-tool approval.",
    structured_output_wrapper: "Return the requested Pi result without altering the normative projection sections.",
};

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct PiProjectionRenderer;

impl ProviderProjectionRenderer for PiProjectionRenderer {
    fn renderer_version(&self) -> &'static str {
        PROFILE.renderer_version
    }

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
