use std::path::PathBuf;

use super::WorkItemRevisionStore;

impl WorkItemRevisionStore {
    pub(super) fn plan_root(&self, project_id: &str, issue_id: &str, plan_id: &str) -> PathBuf {
        self.paths
            .issue_root(project_id, issue_id)
            .join("work-item-revisions")
            .join(plan_id)
    }

    pub(super) fn plan_lineage_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> PathBuf {
        self.plan_root(project_id, issue_id, plan_id)
            .join("lineage.json")
    }

    pub(super) fn initial_plan_publication_journal_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        compile_id: &str,
    ) -> PathBuf {
        self.paths
            .issue_root(project_id, issue_id)
            .join("work-item-revision-publications")
            .join(plan_id)
            .join(format!("{compile_id}.json"))
    }

    pub(super) fn plan_revision_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        revision_id: &str,
    ) -> PathBuf {
        self.plan_root(project_id, issue_id, plan_id)
            .join("plan-revisions")
            .join(format!("{revision_id}.json"))
    }

    pub(super) fn logical_work_items_root(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> PathBuf {
        self.plan_root(project_id, issue_id, plan_id)
            .join("logical-work-items")
    }

    pub(super) fn logical_work_item_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        logical_work_item_id: &str,
    ) -> PathBuf {
        self.logical_work_items_root(project_id, issue_id, plan_id)
            .join(logical_work_item_id)
            .join("logical-work-item.json")
    }

    pub(super) fn draft_revision_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        draft_revision_id: &str,
    ) -> PathBuf {
        self.plan_root(project_id, issue_id, plan_id)
            .join("draft-revisions")
            .join(format!("{draft_revision_id}.json"))
    }

    pub(super) fn draft_revision_state_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        draft_revision_id: &str,
    ) -> PathBuf {
        self.plan_root(project_id, issue_id, plan_id)
            .join("draft-revision-states")
            .join(format!("{draft_revision_id}.json"))
    }

    pub(super) fn work_item_revision_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        logical_work_item_id: &str,
        revision_id: &str,
    ) -> PathBuf {
        self.logical_work_items_root(project_id, issue_id, plan_id)
            .join(logical_work_item_id)
            .join("revisions")
            .join(format!("{revision_id}.json"))
    }

    pub(super) fn verification_plan_revision_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        revision_id: &str,
    ) -> PathBuf {
        self.plan_root(project_id, issue_id, plan_id)
            .join("verification-plan-revisions")
            .join(format!("{revision_id}.json"))
    }

    pub(super) fn plan_validation_report_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        report_id: &str,
    ) -> PathBuf {
        self.plan_root(project_id, issue_id, plan_id)
            .join("validation-reports")
            .join(format!("{report_id}.json"))
    }

    pub(super) fn work_item_projection_bundle_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        bundle_id: &str,
    ) -> PathBuf {
        self.plan_root(project_id, issue_id, plan_id)
            .join("work-item-projection-bundles")
            .join(format!("{bundle_id}.json"))
    }

    pub(super) fn plan_projection_bundle_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        bundle_id: &str,
    ) -> PathBuf {
        self.plan_root(project_id, issue_id, plan_id)
            .join("plan-projection-bundles")
            .join(format!("{bundle_id}.json"))
    }

    pub(super) fn human_presentation_revisions_root(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> PathBuf {
        self.plan_root(project_id, issue_id, plan_id)
            .join("human-presentation-revisions")
    }

    pub(super) fn human_presentation_revision_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        revision_id: &str,
    ) -> PathBuf {
        self.human_presentation_revisions_root(project_id, issue_id, plan_id)
            .join(format!("{revision_id}.json"))
    }

    pub(super) fn dependency_graph_revision_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        revision_id: &str,
    ) -> PathBuf {
        self.plan_root(project_id, issue_id, plan_id)
            .join("dependency-graph-revisions")
            .join(format!("{revision_id}.json"))
    }

    pub(super) fn handoff_revision_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        logical_work_item_id: &str,
        handoff_revision_id: &str,
    ) -> PathBuf {
        self.logical_work_items_root(project_id, issue_id, plan_id)
            .join(logical_work_item_id)
            .join("handoff-revisions")
            .join(format!("{handoff_revision_id}.json"))
    }

    pub(super) fn repair_requests_root(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> PathBuf {
        self.plan_root(project_id, issue_id, plan_id)
            .join("repair-requests")
    }

    pub(super) fn repair_request_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        request_id: &str,
    ) -> PathBuf {
        self.repair_requests_root(project_id, issue_id, plan_id)
            .join(format!("{request_id}.json"))
    }

    pub(super) fn amendment_manifest_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        amendment_id: &str,
    ) -> PathBuf {
        self.plan_root(project_id, issue_id, plan_id)
            .join("amendment-manifests")
            .join(format!("{amendment_id}.json"))
    }

    pub(super) fn amendment_publication_journal_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        journal_id: &str,
    ) -> PathBuf {
        self.plan_root(project_id, issue_id, plan_id)
            .join("amendment-publication-journals")
            .join(format!("{journal_id}.json"))
    }
}
