use cadence_aria::runtime_units::{
    intake_capture::IntakeCaptureUnit, session_bootstrap::SessionBootstrapUnit,
    task_init::TaskInitUnit, RuntimeUnit,
};

#[test]
fn p1_runtime_units_declare_covered_protocol_nodes() {
    assert_eq!(SessionBootstrapUnit.covered_protocol_nodes(), vec!["N00"]);
    assert_eq!(IntakeCaptureUnit.covered_protocol_nodes(), vec!["N01"]);
    assert_eq!(TaskInitUnit.covered_protocol_nodes(), vec!["N02", "N03"]);
}
