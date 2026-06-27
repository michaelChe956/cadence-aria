use crate::product::coding_models::{
    CodeReviewReport, InternalPrReview, ReviewRequest, TestingReport,
};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

impl super::CodingAttemptStore {
    pub fn save_test_plan(
        &self,
        plan: &crate::product::coding_models::TestPlan,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&plan.id)?;
        let attempt = self.find_attempt_by_id(&plan.attempt_id)?;
        write_json(
            &self
                .test_plans_root(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .join(format!("{}.json", plan.id)),
            plan,
        )
    }

    pub fn list_test_plans(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<crate::product::coding_models::TestPlan>, ProductStoreError> {
        super::list_json_records(&self.test_plans_root(project_id, issue_id, attempt_id))
    }

    pub fn save_testing_report(&self, report: &TestingReport) -> Result<(), ProductStoreError> {
        let attempt = self.find_attempt_by_id(&report.attempt_id)?;
        write_json(
            &self
                .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .join("testing-reports")
                .join(format!("{}.json", report.id)),
            report,
        )
    }

    pub fn get_testing_report(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        report_id: &str,
    ) -> Result<TestingReport, ProductStoreError> {
        validate_relative_id(report_id)?;
        read_json(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("testing-reports")
                .join(format!("{report_id}.json")),
        )
    }

    pub fn list_testing_reports(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<TestingReport>, ProductStoreError> {
        super::list_json_records(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("testing-reports"),
        )
    }

    pub fn save_code_review_report(
        &self,
        report: &CodeReviewReport,
    ) -> Result<(), ProductStoreError> {
        let attempt = self.find_attempt_by_id(&report.attempt_id)?;
        write_json(
            &self
                .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .join("code-reviews")
                .join(format!("{}.json", report.id)),
            report,
        )
    }

    pub fn list_code_review_reports(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodeReviewReport>, ProductStoreError> {
        super::list_json_records(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("code-reviews"),
        )
    }

    pub fn save_review_request(&self, request: &ReviewRequest) -> Result<(), ProductStoreError> {
        let attempt = self.find_attempt_by_id(&request.attempt_id)?;
        write_json(
            &self
                .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .join("review-requests")
                .join(format!("{}.json", request.id)),
            request,
        )
    }

    pub fn get_review_request(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        request_id: &str,
    ) -> Result<ReviewRequest, ProductStoreError> {
        validate_relative_id(request_id)?;
        read_json(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("review-requests")
                .join(format!("{request_id}.json")),
        )
    }

    pub fn list_review_requests(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<ReviewRequest>, ProductStoreError> {
        super::list_json_records(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("review-requests"),
        )
    }

    pub fn save_internal_pr_review(
        &self,
        review: &InternalPrReview,
    ) -> Result<(), ProductStoreError> {
        let attempt = self.find_attempt_by_id(&review.attempt_id)?;
        write_json(
            &self
                .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .join("internal-reviews")
                .join(format!("{}.json", review.id)),
            review,
        )
    }

    pub fn get_internal_pr_review(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        review_id: &str,
    ) -> Result<InternalPrReview, ProductStoreError> {
        validate_relative_id(review_id)?;
        read_json(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("internal-reviews")
                .join(format!("{review_id}.json")),
        )
    }

    pub fn list_internal_pr_reviews(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<InternalPrReview>, ProductStoreError> {
        super::list_json_records(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("internal-reviews"),
        )
    }
}
