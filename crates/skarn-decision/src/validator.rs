use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::types::{
    ActionClass, DecisionRequest, DecisionResult, DecisionType, RiskLevel, ValidationError,
};

pub const MAX_RAW_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PATH_LEN: usize = 4096;
pub const MAX_CONTENT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_EVIDENCE_REFS: usize = 64;

/// Strip Windows' `\\?\` or `\\?\UNC\` verbatim prefix.
pub fn de_verbatim(path: &Path) -> PathBuf {
    let s = path.as_os_str().to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

/// Canonicalize `root` (resolving symlinks/junctions) and strip Windows `\\?\`.
pub fn canonical_root(root: &Path) -> PathBuf {
    let canon = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    de_verbatim(&canon)
}

/// Component-wise containment check (`candidate == root` counts as within).
/// Strips Windows `\\?\` prefixes and compares case-insensitively on Windows.
pub fn is_within(root: &Path, candidate: &Path) -> bool {
    let root = de_verbatim(root);
    let candidate = de_verbatim(candidate);
    let mut root = root.components();
    let mut candidate = candidate.components();
    loop {
        match (root.next(), candidate.next()) {
            (None, _) => return true,
            (Some(_), None) => return false,
            (Some(r), Some(c)) => {
                let r = r.as_os_str().to_string_lossy();
                let c = c.as_os_str().to_string_lossy();
                #[cfg(windows)]
                let equal = r.eq_ignore_ascii_case(&c);
                #[cfg(not(windows))]
                let equal = r == c;
                if !equal {
                    return false;
                }
            }
        }
    }
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    is_within(a, b) && is_within(b, a)
}

pub fn rel_display(root: &Path, path: &Path) -> String {
    let plain = de_verbatim(path);
    let root_plain = de_verbatim(root);
    let rel = plain
        .strip_prefix(&root_plain)
        .unwrap_or(&plain)
        .to_string_lossy()
        .replace('\\', "/");
    if rel.is_empty() {
        ".".to_owned()
    } else {
        rel
    }
}

pub fn is_secret_path(path: &str) -> bool {
    path.replace('\\', "/").split('/').any(|part| {
        let name = part.to_ascii_lowercase();
        name == ".env"
            || name.starts_with(".env.")
            || name.ends_with(".pem")
            || name.ends_with(".key")
            || name.ends_with(".p12")
            || name.ends_with(".pfx")
            || name == ".npmrc"
            || name == ".pypirc"
            || name.contains("credential")
            || name.contains("secret")
            || name.contains("password")
    })
}

/// Resolve `user` relative to `root`, rejecting empty paths, null bytes,
/// drive-relative Windows paths, `..` traversal escapes, external absolute
/// paths, and symlink/junction escapes outside the canonicalized `root`.
pub fn resolve_rooted(root: &Path, user: &str) -> Result<PathBuf, ValidationError> {
    if user.contains('\0') {
        return Err(ValidationError::new(
            "null_byte",
            "path",
            "path contains null byte",
        ));
    }
    if user.len() > MAX_PATH_LEN {
        return Err(ValidationError::new(
            "path_too_long",
            "path",
            "path exceeds maximum allowed length",
        ));
    }
    if user.trim().is_empty() {
        return Err(ValidationError::new(
            "empty_path",
            "path",
            "path must not be empty",
        ));
    }

    let canon_root = canonical_root(root);
    let raw = Path::new(user);

    // Reject Windows drive-relative paths like `C:foo` or verbatim prefixes with escapes.
    let has_prefix = raw
        .components()
        .any(|c| matches!(c, Component::Prefix(_)));
    let has_root_dir = raw
        .components()
        .any(|c| matches!(c, Component::RootDir));
    if has_prefix && !has_root_dir {
        return Err(ValidationError::new(
            "path_escape",
            "path",
            format!("drive-relative path `{user}` is not permitted"),
        ));
    }

    let joined = if raw.is_absolute() || has_root_dir {
        de_verbatim(raw)
    } else {
        canon_root.join(raw)
    };

    let mut norm = PathBuf::new();
    for comp in joined.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !norm.pop() {
                    return Err(ValidationError::new(
                        "path_escape",
                        "path",
                        format!("path `{user}` escapes workspace root"),
                    ));
                }
            }
            other => norm.push(other.as_os_str()),
        }
    }

    if !is_within(&canon_root, &norm) {
        return Err(ValidationError::new(
            "path_escape",
            "path",
            format!("path `{user}` escapes workspace root"),
        ));
    }

    let mut probe = norm;
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        match std::fs::canonicalize(&probe) {
            Ok(canon) => {
                let mut resolved = de_verbatim(&canon);
                for part in tail.iter().rev() {
                    resolved.push(part);
                }
                if !is_within(&canon_root, &resolved) {
                    return Err(ValidationError::new(
                        "symlink_escape",
                        "path",
                        format!("path `{user}` resolves through a link outside workspace root"),
                    ));
                }
                return Ok(resolved);
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => match probe.file_name() {
                Some(name) => {
                    tail.push(name.to_os_string());
                    if !probe.pop() {
                        return Err(ValidationError::new(
                            "path_resolve_error",
                            "path",
                            format!("cannot resolve path `{user}`: {err}"),
                        ));
                    }
                }
                None => {
                    return Err(ValidationError::new(
                        "path_resolve_error",
                        "path",
                        format!("cannot resolve path `{user}`: {err}"),
                    ))
                }
            },
            Err(err) => {
                return Err(ValidationError::new(
                    "path_resolve_error",
                    "path",
                    format!("cannot resolve path `{user}`: {err}"),
                ))
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct ActionSpec {
    pub action_id: &'static str,
    pub action_class: ActionClass,
    pub min_risk: RiskLevel,
    pub is_mutating: bool,
    pub allowed_arg_keys: &'static [&'static str],
    pub required_arg_keys: &'static [&'static str],
}

#[derive(Clone, Debug)]
pub struct ActionRegistry {
    specs: BTreeMap<String, ActionSpec>,
}

impl Default for ActionRegistry {
    fn default() -> Self {
        let mut specs = BTreeMap::new();
        let list = [
            ActionSpec {
                action_id: "read_file",
                action_class: ActionClass::ReadInspect,
                min_risk: RiskLevel::Low,
                is_mutating: false,
                allowed_arg_keys: &["path", "start_line", "end_line"],
                required_arg_keys: &["path"],
            },
            ActionSpec {
                action_id: "list_dir",
                action_class: ActionClass::ReadInspect,
                min_risk: RiskLevel::Low,
                is_mutating: false,
                allowed_arg_keys: &["path", "max_depth"],
                required_arg_keys: &[],
            },
            ActionSpec {
                action_id: "fuzzy_find",
                action_class: ActionClass::ReadInspect,
                min_risk: RiskLevel::Low,
                is_mutating: false,
                allowed_arg_keys: &["query", "path"],
                required_arg_keys: &["query"],
            },
            ActionSpec {
                action_id: "search_files",
                action_class: ActionClass::ReadInspect,
                min_risk: RiskLevel::Low,
                is_mutating: false,
                allowed_arg_keys: &["pattern", "path"],
                required_arg_keys: &["pattern"],
            },
            ActionSpec {
                action_id: "write_file",
                action_class: ActionClass::FileEdit,
                min_risk: RiskLevel::Medium,
                is_mutating: true,
                allowed_arg_keys: &["path", "content"],
                required_arg_keys: &["path", "content"],
            },
            ActionSpec {
                action_id: "edit_file",
                action_class: ActionClass::FileEdit,
                min_risk: RiskLevel::Medium,
                is_mutating: true,
                allowed_arg_keys: &["path", "old_str", "new_str", "replace_all"],
                required_arg_keys: &["path", "old_str", "new_str"],
            },
            ActionSpec {
                action_id: "delete_file",
                action_class: ActionClass::FileDelete,
                min_risk: RiskLevel::High,
                is_mutating: true,
                allowed_arg_keys: &["path"],
                required_arg_keys: &["path"],
            },
            ActionSpec {
                action_id: "bash_allowlisted",
                action_class: ActionClass::CommandAllowlisted,
                min_risk: RiskLevel::Low,
                is_mutating: false,
                allowed_arg_keys: &["command"],
                required_arg_keys: &["command"],
            },
            ActionSpec {
                action_id: "bash_gated",
                action_class: ActionClass::CommandGated,
                min_risk: RiskLevel::High,
                is_mutating: true,
                allowed_arg_keys: &["command"],
                required_arg_keys: &["command"],
            },
            ActionSpec {
                action_id: "run_workflow",
                action_class: ActionClass::Workflow,
                min_risk: RiskLevel::High,
                is_mutating: true,
                allowed_arg_keys: &["workflow_id", "inputs"],
                required_arg_keys: &["workflow_id"],
            },
            ActionSpec {
                action_id: "exit_plan_mode",
                action_class: ActionClass::PlanControl,
                min_risk: RiskLevel::High,
                is_mutating: false,
                allowed_arg_keys: &["summary"],
                required_arg_keys: &[],
            },
            ActionSpec {
                action_id: "ask_user_question",
                action_class: ActionClass::Question,
                min_risk: RiskLevel::Low,
                is_mutating: false,
                allowed_arg_keys: &["question", "options"],
                required_arg_keys: &["question"],
            },
        ];
        for spec in list {
            specs.insert(spec.action_id.to_owned(), spec);
        }
        Self { specs }
    }
}

impl ActionRegistry {
    pub fn get(&self, action_id: &str) -> Option<&ActionSpec> {
        self.specs.get(action_id)
    }

    pub fn contains(&self, action_id: &str) -> bool {
        self.specs.contains_key(action_id)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedAction {
    pub action_id: String,
    pub action_class: ActionClass,
    pub effective_risk: RiskLevel,
    pub is_mutating: bool,
    pub target_rel_path: Option<String>,
    pub resolved_path: Option<PathBuf>,
    pub args: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ValidatedDecision {
    Action(ValidatedAction),
    Choice {
        option_id: String,
        action_id: Option<String>,
        effective_risk: RiskLevel,
    },
    Binary {
        accepted: bool,
        effective_risk: RiskLevel,
    },
    Score {
        score: f64,
        effective_risk: RiskLevel,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDecisionResultGuard {
    #[allow(dead_code)]
    provider: serde_json::Value,
    #[allow(dead_code)]
    model: serde_json::Value,
    #[allow(dead_code)]
    decision_type: serde_json::Value,
    #[allow(dead_code)]
    value: serde_json::Value,
    #[allow(dead_code)]
    probabilities: serde_json::Value,
    #[allow(dead_code)]
    raw_confidence: serde_json::Value,
    #[allow(dead_code)]
    evidence_refs: serde_json::Value,
    #[allow(dead_code)]
    risk: serde_json::Value,
    #[allow(dead_code)]
    latency_ms: serde_json::Value,
    #[serde(default)]
    #[allow(dead_code)]
    reasoning_summary: serde_json::Value,
    #[serde(default)]
    #[allow(dead_code)]
    validation_errors: serde_json::Value,
}

/// Strictly parse raw JSON from a decision provider, rejecting malformed JSON,
/// truncated payloads, oversized payloads, unknown top-level fields, and wrong types.
pub fn parse_provider_json_strict(raw: &str) -> Result<DecisionResult, Vec<ValidationError>> {
    if raw.len() > MAX_RAW_PAYLOAD_BYTES {
        return Err(vec![ValidationError::new(
            "payload_too_large",
            "payload",
            "provider JSON exceeds maximum payload size",
        )]);
    }
    let _: RawDecisionResultGuard = serde_json::from_str(raw).map_err(|e| {
        vec![ValidationError::new(
            "invalid_json",
            "payload",
            format!("invalid or unknown fields in provider JSON: {e}"),
        )]
    })?;
    serde_json::from_str::<DecisionResult>(raw).map_err(|e| {
        vec![ValidationError::new(
            "invalid_schema",
            "payload",
            format!("provider JSON failed DecisionResult schema: {e}"),
        )]
    })
}

pub fn validate_decision_result(
    request: &DecisionRequest,
    result: &DecisionResult,
    registry: &ActionRegistry,
) -> Result<ValidatedDecision, Vec<ValidationError>> {
    let mut errors = Vec::new();

    if !result.validation_errors.is_empty() {
        errors.extend(
            result
                .validation_errors
                .iter()
                .map(|e| ValidationError::new(&e.code, &e.field, &e.message)),
        );
    }

    if !result.raw_confidence.is_finite() || !(0.0..=1.0).contains(&result.raw_confidence) {
        errors.push(ValidationError::new(
            "invalid_confidence",
            "raw_confidence",
            "raw_confidence must be finite and within [0.0, 1.0]",
        ));
    }

    for (k, v) in &result.probabilities {
        if !v.is_finite() || !(0.0..=1.0).contains(v) {
            errors.push(ValidationError::new(
                "invalid_probability",
                format!("probabilities.{k}"),
                "probability must be finite and within [0.0, 1.0]",
            ));
        }
    }

    if result.evidence_refs.len() > MAX_EVIDENCE_REFS {
        errors.push(ValidationError::new(
            "too_many_evidence_refs",
            "evidence_refs",
            "evidence_refs exceeds cap",
        ));
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    match result.decision_type {
        DecisionType::Action => validate_action_value(request, result, registry),
        DecisionType::Choice => {
            let option_id = match result.value.as_str() {
                Some(s) => s.to_owned(),
                None => match result.value.as_object() {
                    Some(obj) if obj.len() == 1 => obj
                        .get("option_id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            vec![ValidationError::new(
                                "invalid_choice",
                                "value",
                                "choice value must be an option id string",
                            )]
                        })?
                        .to_owned(),
                    _ => {
                        return Err(vec![ValidationError::new(
                            "invalid_choice",
                            "value",
                            "choice value must be an option id string",
                        )])
                    }
                },
            };
            let opt = request
                .trusted_frame
                .options
                .iter()
                .find(|o| o.id == option_id)
                .ok_or_else(|| {
                    vec![ValidationError::new(
                        "unknown_choice_option",
                        "value",
                        format!("option `{option_id}` is not in trusted_frame.options"),
                    )]
                })?;

            let mut effective_risk = result.risk;
            if let Some(ref aid) = opt.action_id {
                let spec = registry.get(aid).ok_or_else(|| {
                    vec![ValidationError::new(
                        "unregistered_action",
                        "action_id",
                        format!("action `{aid}` is not in closed ActionRegistry"),
                    )]
                })?;
                effective_risk = effective_risk.max(spec.min_risk);
            }
            Ok(ValidatedDecision::Choice {
                option_id,
                action_id: opt.action_id.clone(),
                effective_risk,
            })
        }
        DecisionType::Binary => match result.value.as_bool() {
            Some(accepted) => Ok(ValidatedDecision::Binary {
                accepted,
                effective_risk: result.risk,
            }),
            None => Err(vec![ValidationError::new(
                "invalid_binary",
                "value",
                "binary decision value must be boolean",
            )]),
        },
        DecisionType::Score => match result.value.as_f64() {
            Some(score) if score.is_finite() && (0.0..=1.0).contains(&score) => {
                Ok(ValidatedDecision::Score {
                    score,
                    effective_risk: result.risk,
                })
            }
            _ => Err(vec![ValidationError::new(
                "invalid_score",
                "value",
                "score decision value must be finite number in [0.0, 1.0]",
            )]),
        },
    }
}

fn validate_action_value(
    request: &DecisionRequest,
    result: &DecisionResult,
    registry: &ActionRegistry,
) -> Result<ValidatedDecision, Vec<ValidationError>> {
    let obj = result.value.as_object().ok_or_else(|| {
        vec![ValidationError::new(
            "invalid_action_payload",
            "value",
            "action value must be a JSON object",
        )]
    })?;

    for key in obj.keys() {
        if !matches!(
            key.as_str(),
            "action_id" | "args" | "target_path" | "summary"
        ) {
            return Err(vec![ValidationError::new(
                "unknown_field",
                format!("value.{key}"),
                format!("unexpected field `{key}` in action payload"),
            )]);
        }
    }

    let action_id = obj
        .get("action_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            vec![ValidationError::new(
                "missing_action_id",
                "value.action_id",
                "action_id must be a non-empty string",
            )]
        })?;

    let spec = registry.get(action_id).ok_or_else(|| {
        vec![ValidationError::new(
            "unregistered_action",
            "value.action_id",
            format!("action `{action_id}` is not registered in closed ActionRegistry"),
        )]
    })?;

    let frame_desc = request
        .trusted_frame
        .allowed_actions
        .iter()
        .find(|a| a.action_id == action_id)
        .ok_or_else(|| {
            vec![ValidationError::new(
                "capability_escalation",
                "value.action_id",
                format!("action `{action_id}` is not allowed by trusted_frame"),
            )]
        })?;

    let args_val = obj.get("args").ok_or_else(|| {
        vec![ValidationError::new(
            "missing_args",
            "value.args",
            "args object is required",
        )]
    })?;
    let args = args_val.as_object().ok_or_else(|| {
        vec![ValidationError::new(
            "invalid_args",
            "value.args",
            "args must be a JSON object",
        )]
    })?;

    let allowed: BTreeSet<&str> = spec.allowed_arg_keys.iter().copied().collect();
    for key in args.keys() {
        if !allowed.contains(key.as_str()) {
            return Err(vec![ValidationError::new(
                "unknown_arg_field",
                format!("value.args.{key}"),
                format!("unexpected argument `{key}` for `{action_id}`"),
            )]);
        }
    }
    for req_key in spec.required_arg_keys {
        if !args.contains_key(*req_key) {
            return Err(vec![ValidationError::new(
                "missing_required_arg",
                format!("value.args.{req_key}"),
                format!("missing required argument `{req_key}` for `{action_id}`"),
            )]);
        }
    }

    validate_action_arg_types(action_id, args)?;

    let root = canonical_root(Path::new(&request.snapshot.workspace_root));
    let mut target_rel_path = None;
    let mut resolved_path = None;

    if let Some(path_val) = args.get("path") {
        let user_path = path_val.as_str().ok_or_else(|| {
            vec![ValidationError::new(
                "invalid_arg_type",
                "value.args.path",
                "path must be a string",
            )]
        })?;
        let resolved = resolve_rooted(&root, user_path).map_err(|e| vec![e])?;
        let rel = rel_display(&root, &resolved);

        if matches!(
            action_id,
            "read_file" | "write_file" | "edit_file" | "delete_file"
        ) && is_secret_path(&rel)
        {
            return Err(vec![ValidationError::new(
                "secret_path_blocked",
                "value.args.path",
                format!("refusing secret-looking path `{rel}`"),
            )]);
        }

        if spec.is_mutating && same_path(&resolved, &root) {
            return Err(vec![ValidationError::new(
                "root_mutation_forbidden",
                "value.args.path",
                "refusing to mutate workspace root directly",
            )]);
        }

        if let Some(ref allowed_target) = frame_desc.target_path {
            let allowed_resolved = resolve_rooted(&root, allowed_target).map_err(|e| vec![e])?;
            if !same_path(&resolved, &allowed_resolved) {
                return Err(vec![ValidationError::new(
                    "target_path_mismatch",
                    "value.args.path",
                    "action target path does not match trusted_frame target_path",
                )]);
            }
        }

        target_rel_path = Some(rel);
        resolved_path = Some(resolved);
    }

    if action_id == "bash_allowlisted" {
        let cmd = args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if !is_simple_allowlisted_command(cmd) {
            return Err(vec![ValidationError::new(
                "command_not_allowlisted",
                "value.args.command",
                "command is not a simple allowlisted read-only invocation",
            )]);
        }
    }

    let effective_risk = result.risk.max(spec.min_risk).max(frame_desc.risk);

    Ok(ValidatedDecision::Action(ValidatedAction {
        action_id: action_id.to_owned(),
        action_class: spec.action_class,
        effective_risk,
        is_mutating: spec.is_mutating,
        target_rel_path,
        resolved_path,
        args: args.clone(),
    }))
}

fn validate_action_arg_types(
    action_id: &str,
    args: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), Vec<ValidationError>> {
    let mut errs = Vec::new();
    let check_str = |k: &str, max_bytes: usize, non_empty: bool, errs: &mut Vec<ValidationError>| {
        if let Some(v) = args.get(k) {
            match v.as_str() {
                Some(s) if non_empty && s.trim().is_empty() => errs.push(ValidationError::new(
                    "empty_arg",
                    format!("value.args.{k}"),
                    format!("`{k}` must not be empty"),
                )),
                Some(s) if s.len() > max_bytes => errs.push(ValidationError::new(
                    "arg_too_large",
                    format!("value.args.{k}"),
                    format!("`{k}` exceeds byte cap"),
                )),
                Some(_) => {}
                None => errs.push(ValidationError::new(
                    "invalid_arg_type",
                    format!("value.args.{k}"),
                    format!("`{k}` must be a string"),
                )),
            }
        }
    };
    let check_u64 = |k: &str, errs: &mut Vec<ValidationError>| {
        if let Some(v) = args.get(k) {
            if v.as_u64().is_none() {
                errs.push(ValidationError::new(
                    "invalid_arg_type",
                    format!("value.args.{k}"),
                    format!("`{k}` must be a non-negative integer"),
                ));
            }
        }
    };
    let check_bool = |k: &str, errs: &mut Vec<ValidationError>| {
        if let Some(v) = args.get(k) {
            if !v.is_boolean() {
                errs.push(ValidationError::new(
                    "invalid_arg_type",
                    format!("value.args.{k}"),
                    format!("`{k}` must be a boolean"),
                ));
            }
        }
    };

    match action_id {
        "read_file" => {
            check_str("path", MAX_PATH_LEN, true, &mut errs);
            check_u64("start_line", &mut errs);
            check_u64("end_line", &mut errs);
        }
        "list_dir" => {
            check_str("path", MAX_PATH_LEN, true, &mut errs);
            check_u64("max_depth", &mut errs);
        }
        "fuzzy_find" => {
            check_str("query", 1024, true, &mut errs);
            check_str("path", MAX_PATH_LEN, true, &mut errs);
        }
        "search_files" => {
            check_str("pattern", 2048, true, &mut errs);
            check_str("path", MAX_PATH_LEN, true, &mut errs);
        }
        "write_file" => {
            check_str("path", MAX_PATH_LEN, true, &mut errs);
            check_str("content", MAX_CONTENT_BYTES, false, &mut errs);
        }
        "edit_file" => {
            check_str("path", MAX_PATH_LEN, true, &mut errs);
            check_str("old_str", MAX_CONTENT_BYTES, true, &mut errs);
            check_str("new_str", MAX_CONTENT_BYTES, false, &mut errs);
            check_bool("replace_all", &mut errs);
        }
        "delete_file" => {
            check_str("path", MAX_PATH_LEN, true, &mut errs);
        }
        "bash_allowlisted" | "bash_gated" => {
            check_str("command", 8192, true, &mut errs);
        }
        "run_workflow" => {
            check_str("workflow_id", 256, true, &mut errs);
            if let Some(v) = args.get("inputs") {
                if !v.is_object() {
                    errs.push(ValidationError::new(
                        "invalid_arg_type",
                        "value.args.inputs",
                        "`inputs` must be a JSON object",
                    ));
                }
            }
        }
        "exit_plan_mode" => {
            check_str("summary", 16384, false, &mut errs);
        }
        "ask_user_question" => {
            check_str("question", 4096, true, &mut errs);
            if let Some(v) = args.get("options") {
                match v.as_array() {
                    Some(arr) if arr.iter().all(|item| item.is_string()) => {}
                    _ => errs.push(ValidationError::new(
                        "invalid_arg_type",
                        "value.args.options",
                        "`options` must be an array of strings",
                    )),
                }
            }
        }
        _ => {}
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

/// Simple read-only command allowlist with strict control-operator rejection.
pub fn is_simple_allowlisted_command(cmd: &str) -> bool {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return false;
    }
    let forbidden = ['\0', '\n', '\r', ';', '|', '&', '`', '>', '<', '$'];
    if trimmed.chars().any(|c| forbidden.contains(&c)) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    const ALLOW_PREFIXES: &[&str] = &[
        "git status",
        "git diff",
        "git log",
        "git show",
        "git branch",
        "cargo check",
        "cargo test",
        "cargo clippy",
        "ls",
        "dir",
        "pwd",
        "echo",
    ];
    ALLOW_PREFIXES.iter().any(|prefix| {
        if let Some(rest) = lower.strip_prefix(prefix) {
            rest.is_empty() || rest.starts_with(char::is_whitespace)
        } else {
            false
        }
    })
}
