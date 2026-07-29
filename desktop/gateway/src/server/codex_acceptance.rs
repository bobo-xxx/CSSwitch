#[test]
fn harness_feature_gate_compiles() {
    assert!(cfg!(all(test, feature = "acceptance-build")));
}
