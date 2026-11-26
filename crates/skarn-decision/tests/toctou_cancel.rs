use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;

use skarn_decision::{
    ActionClass, ActionDescriptor, ActionExecutor, ApprovalResponse, BufferSnapshotState,
    CalibrationTable, CheckpointError, DecisionCancelToken, DecisionOrchestrator, DecisionPhase,
    DecisionRequest, DecisionResult, DecisionType, DriftReason, EvidenceItem, EvidenceSourceKind,
    EvidenceTrust, ExecutionTier, FilesystemWorkspaceReader, PermissionMode, ProviderLocality,
    RiskLevel, ScopedPolicyStore, ScriptedMockProvider, SnapshotCapturer, TrustedFrame,
    TurnCheckpointStore, TurnFileChange, ValidatedAction, MAX_CHECKPOINT_TURNS_PER_WORKSPACE,
    MAX_DECISION_HOPS,
};

struct DiskWriteExecutor {
    writes: AtomicUsize,
}

impl DiskWriteExecutor {
    fn new() -> Self {
        Self {
            writes: AtomicUsize::new(0),
        }
    }
    fn write_count(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl ActionExecutor for DiskWriteExecutor {
    async fn execute(
        &self,
        action: &ValidatedAction,
        _request: &DecisionRequest,
    ) -> Result<String, String> {
        if let (Some(resolved), Some(content)) = (
            &action.resolved_path,
            action.args.get("content").and_then(|v| v.as_str()),
        ) {
            std::fs::write(resolved, content).map_err(|e| e.to_string())?;
            self.writes.fetch_add(1, Ordering::SeqCst);
            return Ok(format!("wrote {} bytes", content.len()));
        }
        Err("missing path or content".to_owned())
    }
}

fn build_edit_request(reader: &FilesystemWorkspaceReader, path: &str) -> DecisionRequest {
    let snap = SnapshotCapturer::capture(reader, &[path], 10_000).expect("capture snapshot");
    DecisionRequest {
        request_id: "req-toctou".to_owned(),
        task_type: "file_edit".to_owned(),
        snapshot: snap,
        trusted_frame: TrustedFrame {
            question: "Apply edit".to_owned(),
            options: Vec::new(),
            allowed_actions: vec![ActionDescriptor {
                action_id: "write_file".to_owned(),
                action_class: ActionClass::FileEdit,
                risk: RiskLevel::Medium,
                target_path: Some(path.to_owned()),
            }],
            permission_mode: PermissionMode::AutoEdit,
            rules_digest: None,
        },
        untrusted_evidence: vec![EvidenceItem::new(
            "ev-1",
            EvidenceSourceKind::File,
            path,
            EvidenceTrust::VerifiedFact,
            "initial content",
        )],
        deadline_ms: 1_500,
    }
}

#[tokio::test]
async fn toctou_mid_inference_disk_mutation_aborts_stale_with_zero_writes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("src/app.rs");
    std::fs::create_dir_all(target.parent().unwrap()).expect("mkdir");
    std::fs::write(&target, "pub fn v1() {}\n").expect("seed");

    let reader = Arc::new(FilesystemWorkspaceReader::new(dir.path(), true, false));
    let req = build_edit_request(&reader, "src/app.rs");

    let started = Arc::new(Notify::new());
    let provider = Arc::new(
        ScriptedMockProvider::new(
            "local-laya",
            ProviderLocality::Local,
            Ok(DecisionResult {
                provider: "local-laya".to_owned(),
                model: "laya-v1".to_owned(),
                decision_type: DecisionType::Action,
                value: serde_json::json!({
                    "action_id": "write_file",
                    "args": {
                        "path": "src/app.rs",
                        "content": "pub fn agent_overwrite() {}\n"
                    }
                }),
                probabilities: BTreeMap::new(),
                raw_confidence: 0.98,
                evidence_refs: vec!["ev-1".to_owned()],
                risk: RiskLevel::Medium,
                latency_ms: 20,
                reasoning_summary: None,
                validation_errors: Vec::new(),
            }),
        )
        .with_delay(Duration::from_millis(80))
        .with_started_notify(started.clone()),
    );

    let orch = Arc::new(DecisionOrchestrator::new(
        CalibrationTable::with_default_local("local-laya"),
        vec![provider],
    ));
    let store = Arc::new(ScopedPolicyStore::new());
    let exec = Arc::new(DiskWriteExecutor::new());
    let cancel = DecisionCancelToken::new();

    let notified = started.notified();
    let orch_clone = orch.clone();
    let reader_clone = reader.clone();
    let store_clone = store.clone();
    let exec_clone = exec.clone();
    let cancel_clone = cancel.clone();
    let handle = tokio::spawn(async move {
        orch_clone
            .evaluate(
                &req,
                reader_clone.as_ref(),
                &store_clone,
                None,
                Some(exec_clone.as_ref()),
                &cancel_clone,
            )
            .await
    });

    // Wait until inference has entered provider.decide(), then mutate disk externally.
    notified.await;
    std::fs::write(&target, "pub fn concurrent_human_edit() {}\n").expect("external write");

    let outcome = handle.await.expect("join");
    assert_eq!(outcome.phase, DecisionPhase::Stale);
    assert!(!outcome.executed);
    assert_eq!(exec.write_count(), 0);
    assert!(matches!(
        outcome.drift_reason,
        Some(DriftReason::FileHashMismatch { .. })
    ));
    let final_disk = std::fs::read_to_string(&target).expect("read final");
    assert_eq!(final_disk, "pub fn concurrent_human_edit() {}\n");
}

#[tokio::test]
async fn toctou_mid_inference_dirty_buffer_and_trust_revocation_abort_stale() {
    // 1. Dirty open buffer mid-inference
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("src/lib.rs");
    std::fs::create_dir_all(target.parent().unwrap()).expect("mkdir");
    std::fs::write(&target, "pub fn clean() {}\n").expect("seed");

    let reader = Arc::new(FilesystemWorkspaceReader::new(dir.path(), true, false));
    reader.set_buffer_state(
        "src/lib.rs",
        BufferSnapshotState {
            revision: 1,
            dirty: false,
        },
    );
    let req = build_edit_request(&reader, "src/lib.rs");

    let started = Arc::new(Notify::new());
    let provider = Arc::new(
        ScriptedMockProvider::new(
            "local-laya",
            ProviderLocality::Local,
            Ok(DecisionResult {
                provider: "local-laya".to_owned(),
                model: "laya-v1".to_owned(),
                decision_type: DecisionType::Action,
                value: serde_json::json!({
                    "action_id": "write_file",
                    "args": { "path": "src/lib.rs", "content": "agent" }
                }),
                probabilities: BTreeMap::new(),
                raw_confidence: 0.99,
                evidence_refs: Vec::new(),
                risk: RiskLevel::Medium,
                latency_ms: 15,
                reasoning_summary: None,
                validation_errors: Vec::new(),
            }),
        )
        .with_delay(Duration::from_millis(60))
        .with_started_notify(started.clone()),
    );

    let orch = Arc::new(DecisionOrchestrator::new(
        CalibrationTable::with_default_local("local-laya"),
        vec![provider],
    ));
    let store = Arc::new(ScopedPolicyStore::new());
    let exec = Arc::new(DiskWriteExecutor::new());
    let cancel = DecisionCancelToken::new();

    let notified = started.notified();
    let orch_c = orch.clone();
    let reader_c = reader.clone();
    let store_c = store.clone();
    let exec_c = exec.clone();
    let cancel_c = cancel.clone();
    let handle = tokio::spawn(async move {
        orch_c
            .evaluate(
                &req,
                reader_c.as_ref(),
                &store_c,
                None,
                Some(exec_c.as_ref()),
                &cancel_c,
            )
            .await
    });
    notified.await;
    reader.set_buffer_state(
        "src/lib.rs",
        BufferSnapshotState {
            revision: 2,
            dirty: true,
        },
    );
    let outcome = handle.await.expect("join");
    assert_eq!(outcome.phase, DecisionPhase::Stale);
    assert!(matches!(
        outcome.drift_reason,
        Some(DriftReason::OpenBufferDirty { .. })
    ));
    assert_eq!(exec.write_count(), 0);

    // 2. Trust revocation mid-inference
    reader.set_buffer_state(
        "src/lib.rs",
        BufferSnapshotState {
            revision: 2,
            dirty: false,
        },
    );
    let req2 = build_edit_request(&reader, "src/lib.rs");
    let started2 = Arc::new(Notify::new());
    let provider2 = Arc::new(
        ScriptedMockProvider::new(
            "local-laya",
            ProviderLocality::Local,
            Ok(DecisionResult {
                provider: "local-laya".to_owned(),
                model: "laya-v1".to_owned(),
                decision_type: DecisionType::Action,
                value: serde_json::json!({
                    "action_id": "write_file",
                    "args": { "path": "src/lib.rs", "content": "agent2" }
                }),
                probabilities: BTreeMap::new(),
                raw_confidence: 0.99,
                evidence_refs: Vec::new(),
                risk: RiskLevel::Medium,
                latency_ms: 15,
                reasoning_summary: None,
                validation_errors: Vec::new(),
            }),
        )
        .with_delay(Duration::from_millis(60))
        .with_started_notify(started2.clone()),
    );
    let orch2 = Arc::new(DecisionOrchestrator::new(
        CalibrationTable::with_default_local("local-laya"),
        vec![provider2],
    ));
    let notified2 = started2.notified();
    let orch2_c = orch2.clone();
    let reader_c2 = reader.clone();
    let store_c2 = store.clone();
    let exec_c2 = exec.clone();
    let cancel_c2 = cancel.clone();
    let handle2 = tokio::spawn(async move {
        orch2_c
            .evaluate(
                &req2,
                reader_c2.as_ref(),
                &store_c2,
                None,
                Some(exec_c2.as_ref()),
                &cancel_c2,
            )
            .await
    });
    notified2.await;
    reader.set_trusted(false);
    let outcome2 = handle2.await.expect("join2");
    assert_eq!(outcome2.phase, DecisionPhase::Stale);
    assert!(matches!(
        outcome2.drift_reason,
        Some(DriftReason::TrustDrifted { .. })
    ));
    assert_eq!(exec.write_count(), 0);
}

#[tokio::test]
async fn cancel_mid_flight_returns_cancelled_with_zero_dangling_tasks() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("mkdir");
    std::fs::write(dir.path().join("src/app.rs"), "fn main() {}\n").expect("seed");
    let reader = Arc::new(FilesystemWorkspaceReader::new(dir.path(), true, false));
    let req = build_edit_request(&reader, "src/app.rs");

    let started = Arc::new(Notify::new());
    let provider = Arc::new(
        ScriptedMockProvider::new(
            "slow-provider",
            ProviderLocality::Local,
            Ok(DecisionResult {
                provider: "slow-provider".to_owned(),
                model: "slow".to_owned(),
                decision_type: DecisionType::Action,
                value: serde_json::json!({
                    "action_id": "write_file",
                    "args": { "path": "src/app.rs", "content": "should_never_write" }
                }),
                probabilities: BTreeMap::new(),
                raw_confidence: 0.99,
                evidence_refs: Vec::new(),
                risk: RiskLevel::Medium,
                latency_ms: 5000,
                reasoning_summary: None,
                validation_errors: Vec::new(),
            }),
        )
        .with_delay(Duration::from_secs(5))
        .with_started_notify(started.clone()),
    );

    let orch = Arc::new(DecisionOrchestrator::new(
        CalibrationTable::with_default_local("slow-provider"),
        vec![provider.clone()],
    ));
    let store = Arc::new(ScopedPolicyStore::new());
    let exec = Arc::new(DiskWriteExecutor::new());
    let cancel = DecisionCancelToken::new();

    let notified = started.notified();
    let orch_c = orch.clone();
    let reader_c = reader.clone();
    let store_c = store.clone();
    let exec_c = exec.clone();
    let cancel_c = cancel.clone();
    let handle = tokio::spawn(async move {
        orch_c
            .evaluate(
                &req,
                reader_c.as_ref(),
                &store_c,
                Some(ApprovalResponse::ApproveOnce),
                Some(exec_c.as_ref()),
                &cancel_c,
            )
            .await
    });

    notified.await;
    cancel.cancel();

    // Must complete promptly without waiting for the 5s provider delay.
    let outcome = tokio::time::timeout(Duration::from_millis(500), handle)
        .await
        .expect("bounded cancellation cleanup")
        .expect("join");

    assert_eq!(outcome.phase, DecisionPhase::Cancelled);
    assert!(!outcome.executed);
    assert!(!provider.finished_cleanly());
    assert_eq!(exec.write_count(), 0);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/app.rs")).unwrap(),
        "fn main() {}\n"
    );
}

#[tokio::test]
async fn cancel_token_race_free_across_concurrent_wakeups() {
    for _ in 0..500 {
        let token = DecisionCancelToken::new();
        let t1 = token.clone();
        let t2 = token.clone();
        let h1 = tokio::spawn(async move {
            t1.cancelled().await;
        });
        let h2 = tokio::spawn(async move {
            t2.cancel();
        });
        tokio::time::timeout(Duration::from_millis(250), h1)
            .await
            .expect("no missed wakeup")
            .expect("join h1");
        h2.await.expect("join h2");
    }
}

#[tokio::test]
async fn tier2_deadline_timeout_escalates_within_hop_cap() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("mkdir");
    std::fs::write(dir.path().join("src/app.rs"), "fn main() {}\n").expect("seed");
    let reader = FilesystemWorkspaceReader::new(dir.path(), true, false);
    let mut req = build_edit_request(&reader, "src/app.rs");
    req.deadline_ms = 30;

    let provider = Arc::new(
        ScriptedMockProvider::new(
            "slow-local",
            ProviderLocality::Local,
            Ok(DecisionResult {
                provider: "slow-local".to_owned(),
                model: "slow".to_owned(),
                decision_type: DecisionType::Action,
                value: serde_json::json!({
                    "action_id": "write_file",
                    "args": { "path": "src/app.rs", "content": "nope" }
                }),
                probabilities: BTreeMap::new(),
                raw_confidence: 0.99,
                evidence_refs: Vec::new(),
                risk: RiskLevel::Medium,
                latency_ms: 500,
                reasoning_summary: None,
                validation_errors: Vec::new(),
            }),
        )
        .with_delay(Duration::from_millis(300)),
    );

    let orch = DecisionOrchestrator::new(
        CalibrationTable::with_default_local("slow-local"),
        vec![provider],
    );
    let store = ScopedPolicyStore::new();
    let cancel = DecisionCancelToken::new();

    let outcome = orch
        .evaluate(&req, &reader, &store, None, None, &cancel)
        .await;
    assert_eq!(outcome.phase, DecisionPhase::Escalated);
    assert_eq!(
        outcome.explanation.selected_tier,
        ExecutionTier::Tier3GeneralLlm
    );
    assert!(outcome.explanation.hop_count <= MAX_DECISION_HOPS);
    assert!(!outcome.executed);
}

#[test]
fn checkpoint_existed_before_false_roundtrip_and_conflict_preflight() {
    let ws = tempfile::tempdir().expect("ws");
    let ckpt_dir = tempfile::tempdir().expect("ckpt");
    let store = TurnCheckpointStore::new(ckpt_dir.path());

    let created_rel = "src/new_module.rs";
    let created_abs = ws.path().join(created_rel);
    std::fs::create_dir_all(created_abs.parent().unwrap()).expect("mkdir");
    let after_bytes = b"pub fn newly_created() -> i32 { 42 }\n".to_vec();
    std::fs::write(&created_abs, &after_bytes).expect("write created file");

    let manifest = store
        .record_turn(
            "ws-1",
            "sess-1",
            "turn-1",
            1_000,
            &[TurnFileChange {
                rel_path: created_rel.to_owned(),
                before_bytes: None,
                after_bytes: Some(after_bytes.clone()),
            }],
        )
        .expect("record turn");
    assert!(!manifest.files[0].existed_before);
    assert!(manifest.files[0].existed_after);

    // Human edits the newly created file -> Undo MUST abort in Phase 1 preflight with Conflict
    std::fs::write(&created_abs, b"pub fn human_edited() -> i32 { 99 }\n").expect("human edit");
    let err = store
        .undo_turn(ws.path(), "ws-1", "sess-1", "turn-1", None)
        .expect_err("conflict must abort undo");
    assert!(matches!(err, CheckpointError::Conflict(ref c) if c.len() == 1));
    assert_eq!(
        std::fs::read(&created_abs).unwrap(),
        b"pub fn human_edited() -> i32 { 99 }\n"
    );

    // Restore exact after-bytes -> Undo deletes the newly created file cleanly (no .skarn-bak)
    std::fs::write(&created_abs, &after_bytes).expect("restore after bytes");
    store
        .undo_turn(ws.path(), "ws-1", "sess-1", "turn-1", None)
        .expect("undo deletes new file");
    assert!(!created_abs.exists());
    assert!(!ws.path().join("src/.new_module.rs.skarn-bak").exists());

    // Reapply recreates it atomically with exact bytes
    store
        .reapply_turn(ws.path(), "ws-1", "sess-1", "turn-1", None)
        .expect("reapply recreates new file");
    assert_eq!(std::fs::read(&created_abs).unwrap(), after_bytes);
}

#[test]
fn checkpoint_multi_file_all_or_nothing_atomic_restore() {
    let ws = tempfile::tempdir().expect("ws");
    let ckpt_dir = tempfile::tempdir().expect("ckpt");
    let store = TurnCheckpointStore::new(ckpt_dir.path());

    let a_abs = ws.path().join("a.txt");
    let b_abs = ws.path().join("b.txt");
    std::fs::write(&a_abs, b"a-after").expect("a");
    std::fs::write(&b_abs, b"b-human-edited").expect("b");

    store
        .record_turn(
            "ws-multi",
            "sess-1",
            "turn-1",
            2_000,
            &[
                TurnFileChange {
                    rel_path: "a.txt".to_owned(),
                    before_bytes: Some(b"a-before".to_vec()),
                    after_bytes: Some(b"a-after".to_vec()),
                },
                TurnFileChange {
                    rel_path: "b.txt".to_owned(),
                    before_bytes: Some(b"b-before".to_vec()),
                    after_bytes: Some(b"b-after".to_vec()),
                },
            ],
        )
        .expect("record");

    // Preflight detects conflict on b.txt -> neither a.txt nor b.txt is touched!
    let err = store
        .undo_turn(ws.path(), "ws-multi", "sess-1", "turn-1", None)
        .expect_err("must abort");
    assert!(matches!(err, CheckpointError::Conflict(ref c) if c.len() == 1 && c[0].rel_path == "b.txt"));
    assert_eq!(std::fs::read(&a_abs).unwrap(), b"a-after");
    assert_eq!(std::fs::read(&b_abs).unwrap(), b"b-human-edited");

    // Once b.txt matches after-bytes, undo restores both atomically.
    std::fs::write(&b_abs, b"b-after").expect("sync b");
    store
        .undo_turn(ws.path(), "ws-multi", "sess-1", "turn-1", None)
        .expect("undo succeeds");
    assert_eq!(std::fs::read(&a_abs).unwrap(), b"a-before");
    assert_eq!(std::fs::read(&b_abs).unwrap(), b"b-before");

    store
        .reapply_turn(ws.path(), "ws-multi", "sess-1", "turn-1", None)
        .expect("reapply succeeds");
    assert_eq!(std::fs::read(&a_abs).unwrap(), b"a-after");
    assert_eq!(std::fs::read(&b_abs).unwrap(), b"b-after");
}

#[test]
fn checkpoint_bounded_retention_gc_prunes_beyond_50_turns_and_byte_cap() {
    // 1. 50-turn workspace cap
    let ckpt_dir = tempfile::tempdir().expect("ckpt");
    let store = TurnCheckpointStore::new(ckpt_dir.path());
    for idx in 1..=55u64 {
        let turn_id = format!("turn-{idx:03}");
        let before = format!("before-{idx}").into_bytes();
        let after = format!("after-{idx}").into_bytes();
        store
            .record_turn(
                "ws-gc",
                "sess-1",
                &turn_id,
                idx * 10,
                &[TurnFileChange {
                    rel_path: "file.txt".to_owned(),
                    before_bytes: Some(before),
                    after_bytes: Some(after),
                }],
            )
            .expect("record turn");
    }

    let turns = store.list_workspace_turns("ws-gc").expect("list turns");
    assert_eq!(turns.len(), MAX_CHECKPOINT_TURNS_PER_WORKSPACE);
    assert_eq!(turns.first().unwrap().turn_id, "turn-006");
    assert_eq!(turns.last().unwrap().turn_id, "turn-055");

    // 2. Byte-cap pruning + blob sweep
    let ckpt_small = tempfile::tempdir().expect("ckpt-small");
    let small_store = TurnCheckpointStore::with_limits(ckpt_small.path(), 50, 1500);
    for idx in 1..=10u64 {
        let payload = vec![b'x'; 500];
        let mut after = payload.clone();
        after[0] = (idx as u8) + b'0';
        small_store
            .record_turn(
                "ws-bytes",
                "sess-1",
                &format!("turn-{idx:03}"),
                idx * 100,
                &[TurnFileChange {
                    rel_path: "big.bin".to_owned(),
                    before_bytes: Some(vec![idx as u8; 200]),
                    after_bytes: Some(after),
                }],
            )
            .expect("record big turn");
    }
    let stats = small_store.prune_workspace("ws-bytes").expect("prune");
    assert!(stats.total_bytes_remaining <= 1500);
    let remaining = small_store.list_workspace_turns("ws-bytes").expect("list");
    assert!(remaining.len() < 10);
}
