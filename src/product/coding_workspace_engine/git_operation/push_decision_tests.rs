use super::{ReviewPushDecision, review_push_decision};

#[test]
fn remote_commit_match_is_pushed() {
    assert_eq!(
        review_push_decision("commit_0001", Ok::<_, ()>(Some("commit_0001"))),
        ReviewPushDecision::Pushed
    );
}

#[test]
fn remote_commit_difference_or_absence_is_verified_failed() {
    assert_eq!(
        review_push_decision("commit_0001", Ok::<_, ()>(Some("commit_other"))),
        ReviewPushDecision::Failed
    );
    assert_eq!(
        review_push_decision("commit_0001", Ok::<_, ()>(None)),
        ReviewPushDecision::Failed
    );
}

#[test]
fn remote_query_error_is_indeterminate() {
    assert_eq!(
        review_push_decision("commit_0001", Err(())),
        ReviewPushDecision::Indeterminate
    );
}
