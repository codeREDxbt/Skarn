use skarn_decision::{evaluate_shadow_traces, CalibrationTable, GoldenTrace, MAX_DECISION_HOPS};

#[tokio::test]
async fn shadow_mode_replay_over_28_golden_traces() {
    let raw = include_str!("fixtures/golden_traces.json");
    let traces: Vec<GoldenTrace> = serde_json::from_str(raw).expect("parse golden_traces.json");
    assert_eq!(traces.len(), 28, "expected exactly 28 labeled golden traces");

    let ws = tempfile::tempdir().expect("shadow workspace");
    let calibration = CalibrationTable::with_default_local("local-laya");
    let report = evaluate_shadow_traces(&traces, ws.path(), &calibration).await;

    println!("{}", report.format_summary());

    assert_eq!(report.total_traces, 28);
    assert_eq!(report.confusion_matrix.allow_fp, 0, "0 false-positive executions");
    assert_eq!(report.confusion_matrix.allow_fn, 0, "0 false-negative blocks");
    assert!((report.confusion_matrix.accuracy - 1.0).abs() < 1e-9);
    assert!((report.confusion_matrix.tier_accuracy - 1.0).abs() < 1e-9);
    assert!((report.confusion_matrix.phase_accuracy - 1.0).abs() < 1e-9);
    assert!(report.max_hop_count <= MAX_DECISION_HOPS);
    assert_eq!(report.injection_bypass_count, 0);
    assert_eq!(report.high_risk_auto_exec_count, 0);
    assert!(report.ece < 0.25, "unexpectedly high ECE: {}", report.ece);
    assert!(!report.risk_coverage.is_empty());
    assert!(
        report.latency.p50_delta_ms < 0,
        "expected faster net p50 latency, got {}ms",
        report.latency.p50_delta_ms
    );
    assert!(
        report.tier3_call_reduction_pct >= 70.0,
        "expected significant Tier-3 reduction, got {:.1}%",
        report.tier3_call_reduction_pct
    );
}
