//! Network operations: fetch, pull, push.
//!
//! Every network action runs the real `git` CLI with a fixed argv (never a
//! shell string), `-C <worktree>` and no terminal prompt. Authentication is
//! left to the OS credential helper exactly as the user configured it: no
//! token is read, stored, or passed by Selora.
//!
//! Errors and summaries are redacted before they leave this module — remote
//! URLs keep their host but lose any userinfo, and credential-shaped
//! `key=value` pairs are masked. Output is bounded.
//!
//! Every operation is bounded in wall-clock time ([`NETWORK_TIMEOUT`]): the
//! child is polled rather than waited on forever, and a run that overruns is
//! killed (with its transport helper) and reported as
//! [`GitError::Timeout`]. Pipes are drained concurrently and captured with a
//! fixed cap, so a chatty remote cannot exhaust memory and a full stderr pipe
//! cannot deadlock the wait.
//!
//! Force is never used: `push` builds `refs/heads/<branch>:refs/heads/<branch>`
//! and rejects force refspec syntax during branch-name validation.

use crate::branch::validate_branch_name;
use crate::merge::ConflictEntry;
use crate::{branch_name, GitError, GitRepo};
use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// The output bound for one network operation.
pub const MAX_NETWORK_TEXT: usize = 2000;

/// Wall-clock bound for one fetch / pull / push.
///
/// A dead remote, a stalled `ssh` helper or a credential agent waiting on a
/// dialog would otherwise block the command forever. Three minutes covers a
/// large fetch on a slow link; anything longer is reported as
/// [`GitError::Timeout`] and the user can retry.
pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(180);

/// Bytes kept from each of git's two output streams. The rest is still read
/// (and discarded) so git never blocks on a full pipe.
pub const MAX_NETWORK_CAPTURE_BYTES: usize = 64 * 1024;

/// How often the bounded wait re-checks the child.
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// How long the pipe readers may take to finish after the child is gone.
const PIPE_DRAIN_GRACE: Duration = Duration::from_secs(2);

/// The hint attached to a timeout when git printed nothing useful.
const TIMEOUT_HINT: &str =
    "the remote did not answer in time; check the network, the remote URL and your credentials, \
     then retry";

/// Which network operation produced an outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteOperation {
    Fetch,
    Pull,
    Push,
}

impl RemoteOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::Pull => "pull",
            Self::Push => "push",
        }
    }
}

/// The typed answer to a network operation. `summary` is sanitized and short;
/// `conflicts` is non-empty when a pull stopped on merge conflicts (the merge
/// state is then visible through `merge_conflicts`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteOutcome {
    pub operation: RemoteOperation,
    pub remote: String,
    pub summary: String,
    pub conflicts: Vec<ConflictEntry>,
}

impl GitRepo {
    /// Fetch from `remote` (default: the current branch's remote, else
    /// `origin`, else the only configured remote). Bounded by
    /// [`NETWORK_TIMEOUT`].
    pub fn fetch(&self, remote: Option<&str>) -> Result<RemoteOutcome, GitError> {
        self.fetch_with_timeout(remote, NETWORK_TIMEOUT)
    }

    /// [`GitRepo::fetch`] with an explicit wall-clock bound. The wait polls the
    /// child, so a run past `timeout` is killed and reported as
    /// [`GitError::Timeout`] instead of hanging.
    pub fn fetch_with_timeout(
        &self,
        remote: Option<&str>,
        timeout: Duration,
    ) -> Result<RemoteOutcome, GitError> {
        let remote = self.resolve_remote(remote)?;
        let summary = self.run_git(RemoteOperation::Fetch, &remote, None, timeout)?;
        Ok(RemoteOutcome {
            operation: RemoteOperation::Fetch,
            remote,
            summary,
            conflicts: Vec::new(),
        })
    }

    /// Pull into the current branch. `branch` overrides the remote branch
    /// name (default: the checked-out branch). Merge conflicts are returned as
    /// an outcome, not hidden behind a generic CLI failure. Bounded by
    /// [`NETWORK_TIMEOUT`].
    pub fn pull(
        &self,
        remote: Option<&str>,
        branch: Option<&str>,
    ) -> Result<RemoteOutcome, GitError> {
        self.pull_with_timeout(remote, branch, NETWORK_TIMEOUT)
    }

    /// [`GitRepo::pull`] with an explicit wall-clock bound.
    pub fn pull_with_timeout(
        &self,
        remote: Option<&str>,
        branch: Option<&str>,
        timeout: Duration,
    ) -> Result<RemoteOutcome, GitError> {
        let remote = self.resolve_remote(remote)?;
        let branch = match branch {
            Some(branch) => {
                validate_branch_name(branch)?;
                branch.to_owned()
            }
            None => self.require_current_branch()?,
        };
        let summary = self.run_git(RemoteOperation::Pull, &remote, Some(&branch), timeout)?;
        let conflicts = self.merge_conflicts()?;
        Ok(RemoteOutcome {
            operation: RemoteOperation::Pull,
            remote,
            summary,
            conflicts,
        })
    }

    /// Push the current branch (or `branch`) to `remote`, never forced.
    /// Bounded by [`NETWORK_TIMEOUT`].
    pub fn push(
        &self,
        remote: Option<&str>,
        branch: Option<&str>,
    ) -> Result<RemoteOutcome, GitError> {
        self.push_with_timeout(remote, branch, NETWORK_TIMEOUT)
    }

    /// [`GitRepo::push`] with an explicit wall-clock bound.
    pub fn push_with_timeout(
        &self,
        remote: Option<&str>,
        branch: Option<&str>,
        timeout: Duration,
    ) -> Result<RemoteOutcome, GitError> {
        let remote = self.resolve_remote(remote)?;
        let branch = match branch {
            Some(branch) => {
                validate_branch_name(branch)?;
                self.repo
                    .find_branch(branch, git2::BranchType::Local)
                    .map_err(|_| GitError::Other(format!("branch `{branch}` does not exist")))?;
                branch.to_owned()
            }
            None => self.require_current_branch()?,
        };
        let summary = self.run_git(RemoteOperation::Push, &remote, Some(&branch), timeout)?;
        Ok(RemoteOutcome {
            operation: RemoteOperation::Push,
            remote,
            summary,
            conflicts: Vec::new(),
        })
    }

    /// Configured remote names, sorted.
    pub fn remotes(&self) -> Result<Vec<String>, GitError> {
        let mut out = Vec::new();
        let remotes = self.repo.remotes()?;
        for name in remotes.iter() {
            if let Ok(Some(name)) = name {
                out.push(name.to_owned());
            }
        }
        out.sort();
        Ok(out)
    }

    fn require_current_branch(&self) -> Result<String, GitError> {
        branch_name(&self.repo).ok_or_else(|| {
            GitError::Other("a network operation needs a checked-out branch".to_owned())
        })
    }

    fn resolve_remote(&self, requested: Option<&str>) -> Result<String, GitError> {
        let configured = self.remotes()?;
        if let Some(name) = requested {
            validate_remote_name(name)?;
            if !configured.iter().any(|candidate| candidate == name) {
                return Err(GitError::InvalidName {
                    kind: "remote",
                    name: name.to_owned(),
                    reason: "no such configured remote".to_owned(),
                });
            }
            return Ok(name.to_owned());
        }
        if let Some(branch) = branch_name(&self.repo) {
            let key = format!("branch.{branch}.remote");
            if let Ok(remote) = self
                .repo
                .config()
                .and_then(|config| config.get_string(&key))
            {
                if configured.iter().any(|candidate| candidate == &remote) {
                    return Ok(remote);
                }
            }
        }
        if configured.iter().any(|candidate| candidate == "origin") {
            return Ok("origin".to_owned());
        }
        match configured.as_slice() {
            [only] => Ok(only.clone()),
            [] => Err(GitError::Other(
                "no git remote is configured; add one with `git remote add origin <url>`"
                    .to_owned(),
            )),
            _ => Err(GitError::Other(
                "several remotes are configured and none is the branch upstream".to_owned(),
            )),
        }
    }

    fn run_git(
        &self,
        operation: RemoteOperation,
        remote: &str,
        branch: Option<&str>,
        timeout: Duration,
    ) -> Result<String, GitError> {
        // Fixed argv: program + argument vector, no shell, no interpolation of
        // paths or refs into a command string. `--` terminates options.
        let mut args: Vec<String> = vec![
            "--no-optional-locks".to_owned(),
            "-C".to_owned(),
            self.workdir.to_string_lossy().into_owned(),
        ];
        match operation {
            RemoteOperation::Fetch => {
                args.extend(["fetch".to_owned(), "--".to_owned(), remote.to_owned()]);
            }
            RemoteOperation::Pull => {
                args.extend([
                    "pull".to_owned(),
                    "--no-rebase".to_owned(),
                    "--no-edit".to_owned(),
                    "--".to_owned(),
                    remote.to_owned(),
                ]);
                if let Some(branch) = branch {
                    args.push(branch.to_owned());
                }
            }
            RemoteOperation::Push => {
                let branch =
                    branch.ok_or_else(|| GitError::Other("push needs a branch".to_owned()))?;
                args.extend([
                    "push".to_owned(),
                    "--porcelain".to_owned(),
                    "--".to_owned(),
                    remote.to_owned(),
                    // Explicit non-forced refspec. Branch validation already
                    // rejects `:` / `+` / `^`, so this can never gain force
                    // semantics.
                    format!("refs/heads/{branch}:refs/heads/{branch}"),
                ]);
            }
        }

        let mut command = Command::new("git");
        command
            .args(&args)
            .current_dir(&self.workdir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_PAGER", "cat")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let child = command.spawn().map_err(|err| GitError::Network {
            operation: operation.as_str(),
            message: if err.kind() == std::io::ErrorKind::NotFound {
                "the `git` executable was not found on PATH".to_owned()
            } else {
                sanitize_remote_text(&format!("cannot start git: {err}"))
            },
        })?;
        let run = run_captured(child, operation, timeout)?;
        let status = run.status;
        let stdout = run.stdout;
        let stderr = run.stderr;

        if !status.success() {
            // A pull that produced conflicts is a merge state, not an opaque
            // transport error: surface the conflicts so the UI can resolve.
            if operation == RemoteOperation::Pull
                && self.repo.state() == git2::RepositoryState::Merge
            {
                let conflicts = self.merge_conflicts()?;
                if !conflicts.is_empty() {
                    return Ok(format!(
                        "{} conflicted path(s); resolve them, then commit",
                        conflicts.len()
                    ));
                }
            }
            let detail = if stderr.trim().is_empty() {
                stdout
            } else {
                stderr
            };
            let detail = sanitize_remote_text(&detail);
            let detail = if detail.is_empty() {
                format!("git exited with {}", status)
            } else {
                detail
            };
            return Err(GitError::Network {
                operation: operation.as_str(),
                message: detail,
            });
        }

        let summary = sanitize_remote_text(&stdout);
        Ok(if summary.is_empty() {
            format!("{} {} ok", operation.as_str(), remote)
        } else {
            summary
        })
    }
}

/// What one bounded git run produced.
#[derive(Debug)]
struct NetworkRun {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

/// Wait for `child`, capture both pipes with a fixed cap, and kill the whole
/// run when `timeout` elapses.
///
/// The two readers are detached threads that send their bounded capture over a
/// channel: a transport helper that inherited the pipes and outlived a timeout
/// can therefore never make this function block on `join`.
fn run_captured(
    mut child: Child,
    operation: RemoteOperation,
    timeout: Duration,
) -> Result<NetworkRun, GitError> {
    let stdout_pipe = child.stdout.take().map(capture_stream);
    let stderr_pipe = child.stderr.take().map(capture_stream);
    let status = match wait_with_timeout(&mut child, timeout) {
        Ok(Some(status)) => status,
        Ok(None) => {
            terminate_tree(&mut child);
            // Partial output from a killed run still explains the timeout far
            // better than a generic hint, so drain what the pipes already hold.
            let stdout = drain(stdout_pipe);
            let stderr = drain(stderr_pipe);
            let detail = sanitize_remote_text(if stderr.trim().is_empty() {
                &stdout
            } else {
                &stderr
            });
            return Err(GitError::Timeout {
                operation: operation.as_str(),
                seconds: timeout.as_secs().max(1),
                detail: if detail.is_empty() {
                    TIMEOUT_HINT.to_owned()
                } else {
                    detail
                },
            });
        }
        Err(err) => {
            terminate_tree(&mut child);
            return Err(GitError::Network {
                operation: operation.as_str(),
                message: sanitize_remote_text(&format!("cannot wait for git: {err}")),
            });
        }
    };
    Ok(NetworkRun {
        status,
        stdout: drain(stdout_pipe),
        stderr: drain(stderr_pipe),
    })
}

/// Poll a child until it exits or `timeout` elapses. `Ok(None)` means the
/// deadline passed and the child is still running; the caller must kill it.
fn wait_with_timeout(child: &mut Child, timeout: Duration) -> std::io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        std::thread::sleep(WAIT_POLL_INTERVAL.min(deadline - now));
    }
}

/// Read one pipe to EOF, keeping at most [`MAX_NETWORK_CAPTURE_BYTES`].
///
/// The rest is read and discarded rather than left in the pipe: a program that
/// fills its output pipe blocks forever, so "stop reading" is not an option.
fn capture_stream(mut pipe: impl Read + Send + 'static) -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut kept: Vec<u8> = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            match pipe.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    let room = MAX_NETWORK_CAPTURE_BYTES.saturating_sub(kept.len());
                    kept.extend_from_slice(&buffer[..room.min(read)]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        // The receiver is gone after a timeout; sending then is a no-op.
        let _ = sender.send(kept);
    });
    receiver
}

/// Collect one bounded capture, waiting at most [`PIPE_DRAIN_GRACE`]. A reader
/// that never reaches EOF (a surviving grandchild holds the pipe) yields an
/// empty string instead of blocking the request.
fn drain(pipe: Option<Receiver<Vec<u8>>>) -> String {
    match pipe {
        Some(receiver) => receiver
            .recv_timeout(PIPE_DRAIN_GRACE)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default(),
        None => String::new(),
    }
}

/// Stop a timed-out run and reap it.
///
/// `child.kill` alone leaves git's transport helper (`ssh`, `git-remote-https`)
/// alive holding the output pipes, so on Windows the process tree is killed
/// first. Both steps are best-effort: the timeout error is reported either way.
fn terminate_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let pid = child.id().to_string();
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", pid.as_str()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Remote names are restrictive by construction: no leading `-` (option
/// injection), no whitespace/control, and only the characters git itself uses
/// in remote names.
pub(crate) fn validate_remote_name(name: &str) -> Result<(), GitError> {
    let invalid = |reason: &str| GitError::InvalidName {
        kind: "remote",
        name: name.to_owned(),
        reason: reason.to_owned(),
    };
    if name.is_empty() {
        return Err(invalid("name is empty"));
    }
    if name.len() > 128 {
        return Err(invalid("name is longer than 128 bytes"));
    }
    if name.starts_with('-') {
        return Err(invalid("name must not start with `-`"));
    }
    if name.contains("..")
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/'))
    {
        return Err(invalid("name contains a forbidden character"));
    }
    Ok(())
}

/// Bound and redact CLI output before it reaches an error or an event.
///
/// Redacts:
/// * URL userinfo (`https://user:token@host/…` → `https://***@host/…`);
/// * `key=value` pairs whose key looks like a credential;
/// * `Authorization:` header values;
/// * well-known token prefixes (`ghp_…`, `github_pat_…`, …).
pub fn sanitize_remote_text(text: &str) -> String {
    let text = text.replace('\r', "");
    let mut out = String::new();
    for (index, line) in text.lines().take(60).enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&redact_line(line));
        if out.len() >= MAX_NETWORK_TEXT {
            break;
        }
    }
    let mut out = out.trim().to_owned();
    if out.len() > MAX_NETWORK_TEXT {
        let mut cut = MAX_NETWORK_TEXT;
        while cut > 0 && !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push('…');
    }
    out
}

fn redact_line(line: &str) -> String {
    let mut line = redact_authorization(line);
    line = redact_url_userinfo(&line);
    line = redact_key_values(&line);
    line = redact_token_prefixes(&line);
    line.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

fn redact_authorization(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    if let Some(at) = lower.find("authorization:") {
        let end = at + "authorization:".len();
        return format!("{} ***", &line[..end]);
    }
    line.to_owned()
}

fn redact_url_userinfo(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(scheme_at) = rest.find("://") {
        let authority_start = scheme_at + 3;
        out.push_str(&rest[..authority_start]);
        let authority = &rest[authority_start..];
        let authority_end = authority
            .find(|ch: char| ch.is_whitespace() || matches!(ch, '/' | '?' | '#'))
            .unwrap_or(authority.len());
        let authority = &authority[..authority_end];
        if let Some(at) = authority.rfind('@') {
            out.push_str("***@");
            out.push_str(&authority[at + 1..]);
        } else {
            out.push_str(authority);
        }
        rest = &rest[authority_start + authority_end..];
    }
    out.push_str(rest);
    out
}

fn redact_key_values(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for word in line.split_inclusive(char::is_whitespace) {
        let (body, tail) = split_trailing_whitespace(word);
        match body.split_once('=') {
            Some((key, _value)) if is_secret_key(key) => {
                out.push_str(key);
                out.push_str("=***");
                out.push_str(tail);
            }
            _ => out.push_str(word),
        }
    }
    out
}

fn split_trailing_whitespace(word: &str) -> (&str, &str) {
    let end = word.trim_end_matches(char::is_whitespace).len();
    word.split_at(end)
}

fn is_secret_key(key: &str) -> bool {
    let key = key
        .trim_start_matches(|ch: char| !ch.is_ascii_alphanumeric())
        .to_ascii_lowercase();
    [
        "password",
        "passwd",
        "token",
        "secret",
        "authorization",
        "api_key",
        "apikey",
        "access_token",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

fn redact_token_prefixes(line: &str) -> String {
    const PREFIXES: [&str; 5] = ["ghp_", "gho_", "ghu_", "ghs_", "github_pat_"];
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while !rest.is_empty() {
        let next = PREFIXES
            .iter()
            .filter_map(|prefix| rest.find(prefix).map(|at| (at, *prefix)))
            .min_by_key(|(at, _)| *at);
        match next {
            Some((at, prefix)) => {
                out.push_str(&rest[..at]);
                let token = &rest[at + prefix.len()..];
                let token_end = token
                    .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                    .unwrap_or(token.len());
                out.push_str(prefix);
                out.push_str("***");
                rest = &token[token_end..];
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A process that stays alive far longer than any test timeout, so the
    /// bounded-wait and kill paths are exercised without any network.
    #[cfg(windows)]
    fn silent_child() -> Child {
        Command::new("cmd")
            .args(["/C", "ping -n 30 127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn a silent child")
    }

    #[cfg(unix)]
    fn silent_child() -> Child {
        Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn a silent child")
    }

    #[test]
    fn bounded_wait_reports_a_running_child_then_kill_reaps_it() {
        let mut child = silent_child();
        let started = Instant::now();
        let status = wait_with_timeout(&mut child, Duration::from_millis(150)).expect("poll");
        assert!(status.is_none(), "the child was still running");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the wait returned near its deadline, not at child exit"
        );
        terminate_tree(&mut child);
        assert!(
            child.try_wait().expect("reap").is_some(),
            "a timed-out child is reaped, never left running"
        );
    }

    #[test]
    fn a_timed_out_run_is_a_typed_error() {
        let child = silent_child();
        let started = Instant::now();
        let error = run_captured(child, RemoteOperation::Pull, Duration::from_millis(150))
            .expect_err("a silent git must time out");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the timeout is enforced, not waited out"
        );
        match &error {
            GitError::Timeout {
                operation,
                seconds,
                detail,
            } => {
                assert_eq!(*operation, "pull");
                assert_eq!(*seconds, 1, "sub-second bounds report one second");
                assert!(!detail.is_empty(), "a hint or captured output is attached");
            }
            other => panic!("expected a timeout, got {other:?}"),
        }
        assert_eq!(error.code(), "network_timeout");
    }

    #[test]
    fn capture_keeps_a_bounded_prefix_of_a_huge_stream() {
        let bulk = vec![b'x'; MAX_NETWORK_CAPTURE_BYTES * 4];
        let text = drain(Some(capture_stream(std::io::Cursor::new(bulk))));
        assert_eq!(
            text.len(),
            MAX_NETWORK_CAPTURE_BYTES,
            "the capture is capped"
        );
        assert!(
            text.bytes().all(|byte| byte == b'x'),
            "the kept bytes are the stream prefix"
        );
    }

    #[test]
    fn capture_does_not_block_when_the_writer_is_slow_to_close() {
        // The reader is detached and the collector has its own grace, so a
        // pipe that never reaches EOF cannot stall the request. `drain` on a
        // channel that is never fed returns an empty string by design.
        let (_sender, receiver) = mpsc::channel::<Vec<u8>>();
        let started = Instant::now();
        assert_eq!(drain(Some(receiver)), "");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn remote_name_validation_rejects_option_injection() {
        for bad in ["", "-oProxyCommand=x", "a b", "a;b", "a..b", "a\\b", "a\tb"] {
            assert!(validate_remote_name(bad).is_err(), "{bad:?}");
        }
        for good in ["origin", "upstream", "work/remote"] {
            assert!(validate_remote_name(good).is_ok(), "{good:?}");
        }
    }

    #[test]
    fn sanitizer_redacts_userinfo_key_values_and_tokens() {
        let url = sanitize_remote_text(
            "fatal: unable to access 'https://alice:supersecret@example.invalid/r.git/': refused",
        );
        assert!(!url.contains("supersecret"), "{url}");
        assert!(!url.contains("alice:"), "{url}");
        assert!(url.contains("https://***@example.invalid/r.git/"), "{url}");

        let header = sanitize_remote_text("Authorization: Bearer abc.def.ghi");
        assert!(!header.contains("abc.def.ghi"), "{header}");

        let kv = sanitize_remote_text("password=hunter2 token=abc123 visible=ok");
        assert!(!kv.contains("hunter2"), "{kv}");
        assert!(!kv.contains("abc123"), "{kv}");
        assert!(kv.contains("visible=ok"), "{kv}");

        let token = sanitize_remote_text("remote: ghp_abcdef123456 rejected");
        assert!(!token.contains("ghp_abcdef123456"), "{token}");
        assert!(token.contains("ghp_***"), "{token}");
    }

    #[test]
    fn sanitizer_bounds_output_and_strips_controls() {
        let long = "x".repeat(MAX_NETWORK_TEXT * 2);
        let out = sanitize_remote_text(&long);
        assert!(out.len() <= MAX_NETWORK_TEXT + 4, "{}", out.len());
        assert!(out.ends_with('…'));
        let control = sanitize_remote_text("a\u{1b}[31mred\u{7}");
        assert!(!control.chars().any(char::is_control), "{control:?}");
    }

    #[test]
    fn sanitizer_leaves_plain_text_alone() {
        assert_eq!(
            sanitize_remote_text("From github.com:org/repo\n * branch main -> FETCH_HEAD"),
            "From github.com:org/repo\n * branch main -> FETCH_HEAD"
        );
    }
}
