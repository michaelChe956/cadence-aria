use crate::product::coding_models::{
    CodeReviewReport, CodingExecutionAttempt, InternalPrReview, ReviewRequest,
};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

impl super::CodingAttemptStore {
    pub fn save_code_review_report(
        &self,
        attempt: &CodingExecutionAttempt,
        report: &CodeReviewReport,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&report.id)?;
        self.validate_scoped_attempt_record(
            attempt,
            &report.attempt_id,
            "code_review_report",
            &report.id,
        )?;
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

    pub fn save_review_request(
        &self,
        attempt: &CodingExecutionAttempt,
        request: &ReviewRequest,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&request.id)?;
        self.validate_scoped_attempt_record(
            attempt,
            &request.attempt_id,
            "review_request",
            &request.id,
        )?;
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

    /// PointerPublication 来源的 ReviewRequest 独立分区落盘：
    /// `pointer-publications/{publication_id}/review-requests/{request_id}.json`。
    /// 不塞进 attempt_dir（attempt_dir 有 scoped 校验，强绑定 issue/attempt）。
    /// request_id 建议 `rr-{publication_id}-{member_repo_id}`（每仓一条，可覆盖更新）。
    pub fn save_pointer_review_request(
        &self,
        project_id: &str,
        publication_id: &str,
        request: &ReviewRequest,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&request.id)?;
        write_json(
            &self.pointer_review_request_path(project_id, publication_id, &request.id),
            request,
        )
    }

    pub fn list_pointer_review_requests(
        &self,
        project_id: &str,
        publication_id: &str,
    ) -> Result<Vec<ReviewRequest>, ProductStoreError> {
        super::list_json_records(&self.pointer_review_requests_root(project_id, publication_id))
    }

    pub fn save_internal_pr_review(
        &self,
        attempt: &CodingExecutionAttempt,
        review: &InternalPrReview,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&review.id)?;
        self.validate_scoped_attempt_record(
            attempt,
            &review.attempt_id,
            "internal_pr_review",
            &review.id,
        )?;
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
