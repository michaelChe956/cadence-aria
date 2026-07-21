use super::{ReviewPushDecision, review_push_decision};

#[test]
fn remote_commit_match_is_pushed() {
    assert_eq!(
        review_push_decision("commit_0001", Ok::<_, ()>(Some("commit_0001")), false,),
        ReviewPushDecision::Pushed
    );
}

#[test]
fn transport_ambiguity_with_old_or_absent_remote_ref_remains_indeterminate() {
    assert_eq!(
        review_push_decision("commit_0001", Ok::<_, ()>(Some("commit_other")), false,),
        ReviewPushDecision::Indeterminate
    );
    assert_eq!(
        review_push_decision("commit_0001", Ok::<_, ()>(None), false),
        ReviewPushDecision::Indeterminate
    );
}

#[test]
fn explicit_remote_rejection_with_unmodified_remote_is_failed() {
    assert_eq!(
        review_push_decision("commit_0001", Ok::<_, ()>(None), true),
        ReviewPushDecision::Failed
    );
}

#[test]
fn remote_query_error_is_indeterminate() {
    assert_eq!(
        review_push_decision("commit_0001", Err(()), false),
        ReviewPushDecision::Indeterminate
    );
}
