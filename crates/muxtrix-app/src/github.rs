use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde::Deserialize;

use crate::process::console_command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuthStatus {
    Checking,
    Authenticated { login: String },
    NeedsAuthentication,
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Repository {
    pub(crate) root: PathBuf,
    pub(crate) name: String,
    pub(crate) owner_and_name: Option<String>,
    pub(crate) branch: String,
    pub(crate) wsl_distribution: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileChange {
    pub(crate) path: String,
    pub(crate) previous_path: Option<String>,
    pub(crate) status: String,
    pub(crate) additions: usize,
    pub(crate) deletions: usize,
    /// GitHub's unified patch for this file. The API omits this for binary
    /// and very large files; local working-tree entries leave it unset too.
    pub(crate) patch: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiffLineKind {
    Metadata,
    Hunk,
    Context,
    Addition,
    Deletion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiffLine {
    pub(crate) kind: DiffLineKind,
    pub(crate) old_line: Option<usize>,
    pub(crate) new_line: Option<usize>,
    pub(crate) text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiffDocument {
    pub(crate) lines: Vec<DiffLine>,
    pub(crate) notice: Option<String>,
    pub(crate) truncated: bool,
    pub(crate) max_columns: usize,
}

const DIFF_MAX_BYTES: usize = 4 * 1024 * 1024;
const DIFF_MAX_LINES: usize = 50_000;
/// Bound the two source blobs before asking Git to compare them. The rendered
/// diff has a tighter cap, but this larger ceiling recovers the files GitHub
/// commonly omits from its inline `patch` field without letting a generated
/// artifact consume unbounded memory.
const REMOTE_DIFF_SOURCE_MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckSummary {
    pub(crate) passed: usize,
    pub(crate) pending: usize,
    pub(crate) failed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PullRequest {
    pub(crate) number: u64,
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) author: String,
    pub(crate) head: String,
    pub(crate) head_oid: String,
    pub(crate) head_repository: String,
    pub(crate) base: String,
    pub(crate) base_oid: String,
    pub(crate) additions: usize,
    pub(crate) deletions: usize,
    pub(crate) changed_files: usize,
    pub(crate) draft: bool,
    pub(crate) mergeable: String,
    pub(crate) merge_state: String,
    pub(crate) review_decision: String,
    pub(crate) checks: CheckSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PullRequestSummaryStatus {
    Open,
    Draft,
    Merged,
}

impl PullRequestSummaryStatus {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Draft => "Draft",
            Self::Merged => "Merged",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PullRequestSummary {
    pub(crate) number: u64,
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) author: String,
    pub(crate) head: String,
    pub(crate) base: String,
    pub(crate) status: PullRequestSummaryStatus,
}

impl PullRequestSummary {
    pub(crate) fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        query.is_empty()
            || self.title.to_ascii_lowercase().contains(&query)
            || self.author.to_ascii_lowercase().contains(&query)
            || self.head.to_ascii_lowercase().contains(&query)
            || self.base.to_ascii_lowercase().contains(&query)
            || self.status.label().to_ascii_lowercase().contains(&query)
            || self
                .number
                .to_string()
                .contains(query.trim_start_matches('#'))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PullRequestDetails {
    pub(crate) pull_request: PullRequest,
    pub(crate) files: Vec<FileChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MergeReadiness {
    Ready,
    Draft,
    Conflicts,
    Behind,
    ChecksPending,
    ChecksFailed,
    ReviewRequired,
    Blocked,
    Unknown,
}

impl PullRequest {
    pub(crate) fn readiness(&self) -> MergeReadiness {
        if self.draft {
            return MergeReadiness::Draft;
        }
        if self.mergeable == "CONFLICTING" || self.merge_state == "DIRTY" {
            return MergeReadiness::Conflicts;
        }
        if self.checks.failed > 0 {
            return MergeReadiness::ChecksFailed;
        }
        if self.checks.pending > 0 {
            return MergeReadiness::ChecksPending;
        }
        if self.review_decision == "CHANGES_REQUESTED" || self.review_decision == "REVIEW_REQUIRED"
        {
            return MergeReadiness::ReviewRequired;
        }
        if self.merge_state == "BEHIND" {
            return MergeReadiness::Behind;
        }
        if matches!(
            self.merge_state.as_str(),
            "BLOCKED" | "HAS_HOOKS" | "UNSTABLE"
        ) {
            return MergeReadiness::Blocked;
        }
        if self.mergeable == "MERGEABLE" && self.merge_state == "CLEAN" {
            return MergeReadiness::Ready;
        }
        MergeReadiness::Unknown
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelData {
    pub(crate) branch: String,
    pub(crate) files: Vec<FileChange>,
    pub(crate) additions: usize,
    pub(crate) deletions: usize,
}

pub(crate) fn repository_from(
    directory: &Path,
    wsl_distribution: &str,
) -> Result<Repository, String> {
    let root_output = git(
        directory,
        wsl_distribution,
        &["rev-parse", "--show-toplevel"],
    )?;
    if !root_output.status.success() {
        return Err("The focused pane is not inside a Git repository.".into());
    }
    let root = PathBuf::from(stdout_line(&root_output));
    if root.as_os_str().is_empty() {
        return Err("Git did not return a repository root for the focused pane.".into());
    }
    let branch_output = git(&root, wsl_distribution, &["branch", "--show-current"])?;
    let branch = if branch_output.status.success() {
        let branch = stdout_line(&branch_output);
        if branch.is_empty() {
            "Detached HEAD".into()
        } else {
            branch
        }
    } else {
        "Unknown branch".into()
    };
    let remote = git(&root, wsl_distribution, &["remote", "get-url", "origin"])
        .ok()
        .filter(|output| output.status.success())
        .map(|output| stdout_line(&output));
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Repository")
        .to_owned();
    Ok(Repository {
        root,
        name,
        owner_and_name: remote.as_deref().and_then(parse_github_remote),
        branch,
        wsl_distribution: wsl_distribution.to_owned(),
    })
}

pub(crate) fn auth_status() -> AuthStatus {
    let output = match console_command("gh")
        .args(["api", "user", "--jq", ".login"])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return AuthStatus::Unavailable {
                reason: "Install the GitHub CLI to connect Muxtrix.".into(),
            };
        }
        Err(error) => {
            return AuthStatus::Unavailable {
                reason: format!("GitHub CLI could not start: {error}"),
            };
        }
    };
    if output.status.success() {
        let login = stdout_line(&output);
        return AuthStatus::Authenticated {
            login: if login.is_empty() {
                "GitHub".into()
            } else {
                login
            },
        };
    }
    let failure = output_failure(&output);
    if failure.to_ascii_lowercase().contains("auth")
        || failure.to_ascii_lowercase().contains("login")
        || failure.to_ascii_lowercase().contains("token")
    {
        AuthStatus::NeedsAuthentication
    } else {
        AuthStatus::Unavailable {
            reason: if failure.is_empty() {
                "GitHub authentication could not be checked.".into()
            } else {
                failure
            },
        }
    }
}

pub(crate) fn authenticate() -> Result<AuthStatus, String> {
    let output = console_command("gh")
        .args([
            "auth",
            "login",
            "--hostname",
            "github.com",
            "--git-protocol",
            "https",
            "--web",
        ])
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "Install the GitHub CLI, then try connecting again.".to_owned()
            } else {
                format!("GitHub authentication could not start: {error}")
            }
        })?;
    if !output.status.success() {
        return Err(nonempty_failure(
            &output,
            "GitHub authentication did not finish. Try again when you are ready.",
        ));
    }
    match auth_status() {
        status @ AuthStatus::Authenticated { .. } => Ok(status),
        _ => Err("GitHub did not report an authenticated account after login.".into()),
    }
}

pub(crate) fn load_local(repository: &Repository) -> Result<PanelData, String> {
    let status = git(
        &repository.root,
        &repository.wsl_distribution,
        &["status", "--porcelain=v1"],
    )?;
    if !status.status.success() {
        return Err(nonempty_failure(
            &status,
            "Git could not read the repository status.",
        ));
    }
    let mut files = parse_status(&String::from_utf8_lossy(&status.stdout));
    let numstat = git(
        &repository.root,
        &repository.wsl_distribution,
        &["diff", "HEAD", "--numstat"],
    )?;
    if numstat.status.success() {
        apply_numstat(&mut files, &String::from_utf8_lossy(&numstat.stdout));
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let additions = files.iter().map(|file| file.additions).sum();
    let deletions = files.iter().map(|file| file.deletions).sum();
    Ok(PanelData {
        branch: repository.branch.clone(),
        files,
        additions,
        deletions,
    })
}

pub(crate) fn list_pull_requests(
    repository: &Repository,
) -> Result<Vec<PullRequestSummary>, String> {
    let owner_and_name = github_repository(repository)?;
    let output = console_command("gh")
        .args([
            "pr",
            "list",
            "--repo",
            owner_and_name,
            "--state",
            "open",
            "--limit",
            "1000",
            "--json",
            "number,title,url,author,headRefName,baseRefName,isDraft",
        ])
        .output()
        .map_err(|error| format!("GitHub pull requests could not be read: {error}"))?;
    if !output.status.success() {
        return Err(nonempty_failure(
            &output,
            "GitHub pull requests are unavailable.",
        ));
    }
    parse_pull_request_summaries(&output.stdout)
}

pub(crate) fn load_pull_request_details(
    repository: &Repository,
    number: u64,
) -> Result<PullRequestDetails, String> {
    let owner_and_name = github_repository(repository)?;
    let pull_request = load_pull_request(repository, number)?;
    let mut files = load_pull_request_files(owner_and_name, number)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(PullRequestDetails {
        pull_request,
        files,
    })
}

pub(crate) fn load_diff(
    repository: &Repository,
    file: &FileChange,
    github_patch: bool,
) -> Result<DiffDocument, String> {
    validate_relative_git_path(&file.path)?;
    if let Some(previous) = file.previous_path.as_deref() {
        validate_relative_git_path(previous)?;
    }
    if github_patch {
        let Some(patch) = file.patch.as_deref() else {
            return Ok(DiffDocument {
                lines: Vec::new(),
                notice: Some(
                    "GitHub did not provide a textual patch. The file may be binary or the diff may be too large."
                        .into(),
                ),
                truncated: false,
                max_columns: 0,
            });
        };
        let mut document = parse_diff(patch.as_bytes());
        let shown_additions = document
            .lines
            .iter()
            .filter(|line| line.kind == DiffLineKind::Addition)
            .count();
        let shown_deletions = document
            .lines
            .iter()
            .filter(|line| line.kind == DiffLineKind::Deletion)
            .count();
        if shown_additions < file.additions || shown_deletions < file.deletions {
            document.truncated = true;
            document.notice = Some(
                "GitHub returned only part of this patch. Showing the available lines.".into(),
            );
        }
        return Ok(document);
    }

    let output = if file.status == "Untracked" {
        git(
            &repository.root,
            &repository.wsl_distribution,
            &[
                "diff",
                "--no-ext-diff",
                "--no-color",
                "--no-index",
                "--",
                "/dev/null",
                &file.path,
            ],
        )?
    } else if let Some(previous) = file.previous_path.as_deref() {
        git(
            &repository.root,
            &repository.wsl_distribution,
            &[
                "diff",
                "--no-ext-diff",
                "--no-color",
                "HEAD",
                "--",
                previous,
                &file.path,
            ],
        )?
    } else {
        git(
            &repository.root,
            &repository.wsl_distribution,
            &[
                "diff",
                "--no-ext-diff",
                "--no-color",
                "HEAD",
                "--",
                &file.path,
            ],
        )?
    };
    // `git diff --no-index` returns one when differences exist.
    if !output.status.success() && !(file.status == "Untracked" && output.status.code() == Some(1))
    {
        return Err(nonempty_failure(
            &output,
            "Git could not read this file's diff.",
        ));
    }
    let mut document = parse_diff(&output.stdout);
    if document.lines.is_empty() && document.notice.is_none() {
        document.notice = Some("This file has no textual changes to display.".into());
    }
    Ok(document)
}

/// Loads a pull-request file diff without assuming GitHub included the
/// optional inline patch. Large textual patches are reconstructed lazily from
/// the base and head blobs only when the user opens that file; binary and
/// exceptionally large sources degrade to an honest non-text notice.
pub(crate) fn load_pull_request_diff(
    repository: &Repository,
    pull_request: &PullRequest,
    file: &FileChange,
) -> Result<DiffDocument, String> {
    validate_relative_git_path(&file.path)?;
    if let Some(previous) = file.previous_path.as_deref() {
        validate_relative_git_path(previous)?;
    }
    let inline = if file.patch.is_some() {
        let document = load_diff(repository, file, true)?;
        if !document.truncated {
            return Ok(document);
        }
        Some(document)
    } else {
        None
    };

    match reconstruct_pull_request_diff(repository, pull_request, file) {
        Ok(document) if !document.lines.is_empty() || inline.is_none() => Ok(document),
        Ok(_) | Err(_) if inline.is_some() => Ok(inline.expect("inline patch checked above")),
        Err(error) => Err(error),
        Ok(document) => Ok(document),
    }
}

fn reconstruct_pull_request_diff(
    repository: &Repository,
    pull_request: &PullRequest,
    file: &FileChange,
) -> Result<DiffDocument, String> {
    let previous_path = file.previous_path.as_deref().unwrap_or(&file.path);
    let before = if file.status == "Added" {
        Vec::new()
    } else {
        match load_remote_file(
            github_repository(repository)?,
            &pull_request.base_oid,
            previous_path,
            "base",
        )? {
            RemoteFile::Content(bytes) => bytes,
            RemoteFile::TooLarge => return Ok(remote_diff_too_large()),
        }
    };
    let after = if file.status == "Deleted" {
        Vec::new()
    } else {
        match load_remote_file(
            if pull_request.head_repository.is_empty() {
                github_repository(repository)?
            } else {
                &pull_request.head_repository
            },
            &pull_request.head_oid,
            &file.path,
            "head",
        )? {
            RemoteFile::Content(bytes) => bytes,
            RemoteFile::TooLarge => return Ok(remote_diff_too_large()),
        }
    };

    if looks_binary(&before) || looks_binary(&after) {
        return Ok(DiffDocument {
            lines: Vec::new(),
            notice: Some("This is a binary file, so there is no textual diff to display.".into()),
            truncated: false,
            max_columns: 0,
        });
    }

    diff_remote_file_contents(&before, &after)
}

fn load_remote_file(
    owner_and_name: &str,
    oid: &str,
    path: &str,
    revision_label: &str,
) -> Result<RemoteFile, String> {
    let endpoint = format!(
        "repos/{owner_and_name}/contents/{}?ref={}",
        percent_encode_github_path(path),
        percent_encode_github_path(oid),
    );
    let mut command = console_command("gh");
    command
        .args([
            "api",
            "-H",
            "Accept: application/vnd.github.raw+json",
            &endpoint,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("GitHub could not load the {revision_label} file: {error}"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "GitHub did not expose the requested file contents.".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "GitHub did not expose diagnostics for the file request.".to_owned())?;
    // Drain diagnostics concurrently so an unexpectedly verbose CLI error can
    // never block the bounded content read. Only the first 64 KiB is retained.
    let stderr_reader = std::thread::spawn(move || drain_with_storage_limit(stderr, 64 * 1024));
    let mut bytes = Vec::with_capacity(REMOTE_DIFF_SOURCE_MAX_BYTES.min(256 * 1024));
    let read_result = stdout
        .by_ref()
        .take((REMOTE_DIFF_SOURCE_MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes);
    let too_large = bytes.len() > REMOTE_DIFF_SOURCE_MAX_BYTES;
    if too_large || read_result.is_err() {
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|error| format!("GitHub file loading did not finish cleanly: {error}"))?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "GitHub's diagnostic reader stopped unexpectedly.".to_owned())?;
    read_result.map_err(|error| format!("GitHub file contents could not be read: {error}"))?;
    if too_large {
        return Ok(RemoteFile::TooLarge);
    }
    if !status.success() {
        let failure = String::from_utf8_lossy(&stderr).trim().to_owned();
        return Err(if failure.is_empty() {
            format!("GitHub could not load the {revision_label} version of {path}.")
        } else {
            failure
        });
    }
    Ok(RemoteFile::Content(bytes))
}

enum RemoteFile {
    Content(Vec<u8>),
    TooLarge,
}

fn remote_diff_too_large() -> DiffDocument {
    DiffDocument {
        lines: Vec::new(),
        notice: Some(
            "This file is larger than 16 MiB, so Muxtrix did not build an in-memory diff. Open the pull request on GitHub to inspect it."
                .into(),
        ),
        truncated: false,
        max_columns: 0,
    }
}

fn drain_with_storage_limit(mut reader: impl std::io::Read, limit: usize) -> Vec<u8> {
    let mut stored = Vec::with_capacity(limit.min(8 * 1024));
    let mut buffer = [0_u8; 8 * 1024];
    while let Ok(read) = reader.read(&mut buffer) {
        if read == 0 {
            break;
        }
        let keep = read.min(limit.saturating_sub(stored.len()));
        stored.extend_from_slice(&buffer[..keep]);
    }
    stored
}

fn percent_encode_github_path(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8 * 1024).any(|byte| *byte == 0)
}

fn diff_remote_file_contents(before: &[u8], after: &[u8]) -> Result<DiffDocument, String> {
    let files = TemporaryDiffFiles::create(before, after)?;
    let output = console_command("git")
        .args(["diff", "--no-index", "--no-color", "--no-ext-diff", "--"])
        .arg(&files.before)
        .arg(&files.after)
        .output()
        .map_err(|error| format!("Git could not reconstruct this pull request diff: {error}"))?;
    // `git diff --no-index` returns one when differences exist.
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(nonempty_failure(
            &output,
            "Git could not reconstruct this pull request diff.",
        ));
    }
    let patch = diff_hunks(&output.stdout);
    if patch.is_empty() {
        return Ok(DiffDocument {
            lines: Vec::new(),
            notice: Some(
                if output
                    .stdout
                    .windows(12)
                    .any(|window| window == b"Binary files")
                {
                    "This is a binary file, so there is no textual diff to display.".into()
                } else {
                    "The base and head versions have no textual differences.".into()
                },
            ),
            truncated: false,
            max_columns: 0,
        });
    }
    Ok(parse_diff(patch))
}

fn diff_hunks(diff: &[u8]) -> &[u8] {
    let mut start = 0;
    for line in diff.split_inclusive(|byte| *byte == b'\n') {
        if line.starts_with(b"@@") {
            return &diff[start..];
        }
        start += line.len();
    }
    &[]
}

struct TemporaryDiffFiles {
    root: PathBuf,
    before: PathBuf,
    after: PathBuf,
}

impl TemporaryDiffFiles {
    fn create(before: &[u8], after: &[u8]) -> Result<Self, String> {
        let root = std::env::temp_dir().join(format!(
            "muxtrix-pr-diff-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        create_private_directory(&root)
            .map_err(|error| format!("A temporary diff directory could not be created: {error}"))?;
        let files = Self {
            before: root.join("base"),
            after: root.join("head"),
            root,
        };
        write_private_file(&files.before, before)
            .and_then(|()| write_private_file(&files.after, after))
            .map_err(|error| {
                format!("Temporary pull request files could not be written: {error}")
            })?;
        Ok(files)
    }
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;

    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)
}

#[cfg(unix)]
fn write_private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)
}

impl Drop for TemporaryDiffFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.before);
        let _ = std::fs::remove_file(&self.after);
        let _ = std::fs::remove_dir(&self.root);
    }
}

fn validate_relative_git_path(path: &str) -> Result<(), String> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err("Git returned an unsafe repository-relative file path.".into());
    }
    Ok(())
}

pub(crate) fn parse_diff(bytes: &[u8]) -> DiffDocument {
    let byte_truncated = bytes.len() > DIFF_MAX_BYTES;
    let bytes = &bytes[..bytes.len().min(DIFF_MAX_BYTES)];
    let text = String::from_utf8_lossy(bytes);
    let mut old_line = None;
    let mut new_line = None;
    let mut lines = Vec::new();
    let mut line_truncated = false;
    for raw in text.lines() {
        if lines.len() == DIFF_MAX_LINES {
            line_truncated = true;
            break;
        }
        let (kind, old, new) = if raw.starts_with("@@") {
            if let Some((old_start, new_start)) = parse_hunk_starts(raw) {
                old_line = Some(old_start);
                new_line = Some(new_start);
            }
            (DiffLineKind::Hunk, None, None)
        } else if raw.starts_with("+++")
            || raw.starts_with("---")
            || raw.starts_with("diff ")
            || raw.starts_with("index ")
            || raw.starts_with("new file ")
            || raw.starts_with("deleted file ")
            || raw.starts_with("similarity ")
            || raw.starts_with("rename ")
            || raw.starts_with('\\')
        {
            (DiffLineKind::Metadata, None, None)
        } else if raw.starts_with('+') {
            let current = new_line;
            new_line = new_line.map(|line| line + 1);
            (DiffLineKind::Addition, None, current)
        } else if raw.starts_with('-') {
            let current = old_line;
            old_line = old_line.map(|line| line + 1);
            (DiffLineKind::Deletion, current, None)
        } else {
            let current_old = old_line;
            let current_new = new_line;
            old_line = old_line.map(|line| line + 1);
            new_line = new_line.map(|line| line + 1);
            (DiffLineKind::Context, current_old, current_new)
        };
        lines.push(DiffLine {
            kind,
            old_line: old,
            new_line: new,
            text: raw.replace('\t', "    "),
        });
    }
    let truncated = byte_truncated || line_truncated;
    let max_columns = lines
        .iter()
        .map(|line| line.text.chars().count())
        .max()
        .unwrap_or(0);
    DiffDocument {
        lines,
        notice: truncated.then(|| {
            "Diff truncated after 4 MiB or 50,000 lines to keep the viewer responsive.".into()
        }),
        truncated,
        max_columns,
    }
}

fn parse_hunk_starts(line: &str) -> Option<(usize, usize)> {
    let mut ranges = line.split_whitespace();
    let _marker = ranges.next()?;
    let old = ranges.next()?.strip_prefix('-')?;
    let new = ranges.next()?.strip_prefix('+')?;
    let start = |range: &str| range.split(',').next()?.parse::<usize>().ok();
    Some((start(old)?, start(new)?))
}

fn load_pull_request_files(owner_and_name: &str, number: u64) -> Result<Vec<FileChange>, String> {
    let endpoint = format!("repos/{owner_and_name}/pulls/{number}/files");
    let output = console_command("gh")
        .args(["api", &endpoint, "--paginate", "--slurp"])
        .output()
        .map_err(|error| format!("GitHub changed files could not be read: {error}"))?;
    if !output.status.success() {
        return Err(nonempty_failure(
            &output,
            "GitHub changed files are unavailable.",
        ));
    }
    parse_pull_request_files(&output.stdout)
}

pub(crate) fn merge(
    repository: &Repository,
    number: u64,
    head_oid: &str,
) -> Result<String, String> {
    let owner_and_name = github_repository(repository)?;
    let number = number.to_string();
    let output = console_command("gh")
        .args([
            "pr",
            "merge",
            &number,
            "--repo",
            owner_and_name,
            "--merge",
            "--match-head-commit",
            head_oid,
        ])
        .output()
        .map_err(|error| format!("GitHub merge could not start: {error}"))?;
    if output.status.success() {
        Ok(format!("Merged pull request #{number}"))
    } else {
        Err(nonempty_failure(
            &output,
            "GitHub could not merge this pull request.",
        ))
    }
}

fn load_pull_request(repository: &Repository, number: u64) -> Result<PullRequest, String> {
    let output = pull_request_view_command(repository, number)?
        .output()
        .map_err(|error| format!("GitHub pull request details could not be read: {error}"))?;
    if !output.status.success() {
        let failure = output_failure(&output);
        return Err(if failure.is_empty() {
            "GitHub pull request details are unavailable.".into()
        } else {
            failure
        });
    }
    parse_pull_request(&output.stdout)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestResponse {
    number: u64,
    title: String,
    url: String,
    author: Option<PullRequestAuthor>,
    head_ref_name: String,
    head_ref_oid: String,
    head_repository: Option<PullRequestRepository>,
    base_ref_name: String,
    base_ref_oid: String,
    additions: usize,
    deletions: usize,
    changed_files: usize,
    is_draft: bool,
    mergeable: String,
    merge_state_status: String,
    #[serde(default)]
    review_decision: String,
    #[serde(default)]
    status_check_rollup: Vec<PullRequestCheck>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestSummaryResponse {
    number: u64,
    title: String,
    url: String,
    author: Option<PullRequestAuthor>,
    head_ref_name: String,
    base_ref_name: String,
    is_draft: bool,
}

#[derive(Debug, Deserialize)]
struct PullRequestAuthor {
    login: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestRepository {
    name_with_owner: String,
}

#[derive(Debug, Deserialize)]
struct PullRequestCheck {
    #[serde(default)]
    status: String,
    conclusion: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PullRequestFileResponse {
    filename: String,
    previous_filename: Option<String>,
    status: String,
    additions: usize,
    deletions: usize,
    patch: Option<String>,
}

fn parse_pull_request_files(bytes: &[u8]) -> Result<Vec<FileChange>, String> {
    let pages: Vec<Vec<PullRequestFileResponse>> = serde_json::from_slice(bytes)
        .map_err(|error| format!("GitHub returned invalid changed-file details: {error}"))?;
    Ok(pages
        .into_iter()
        .flatten()
        .map(|file| FileChange {
            path: file.filename,
            previous_path: file.previous_filename,
            status: match file.status.as_str() {
                "added" => "Added",
                "removed" => "Deleted",
                "renamed" => "Renamed",
                "copied" => "Copied",
                _ => "Modified",
            }
            .into(),
            additions: file.additions,
            deletions: file.deletions,
            patch: file.patch,
        })
        .collect())
}

fn parse_pull_request_summaries(bytes: &[u8]) -> Result<Vec<PullRequestSummary>, String> {
    let responses: Vec<PullRequestSummaryResponse> = serde_json::from_slice(bytes)
        .map_err(|error| format!("GitHub returned an invalid pull request list: {error}"))?;
    Ok(responses
        .into_iter()
        .map(|pull_request| PullRequestSummary {
            number: pull_request.number,
            title: pull_request.title,
            url: pull_request.url,
            author: pull_request
                .author
                .map_or_else(|| "Unknown author".into(), |author| author.login),
            head: pull_request.head_ref_name,
            base: pull_request.base_ref_name,
            status: if pull_request.is_draft {
                PullRequestSummaryStatus::Draft
            } else {
                PullRequestSummaryStatus::Open
            },
        })
        .collect())
}

fn parse_pull_request(bytes: &[u8]) -> Result<PullRequest, String> {
    let response: PullRequestResponse = serde_json::from_slice(bytes)
        .map_err(|error| format!("GitHub returned invalid pull request details: {error}"))?;
    let mut checks = CheckSummary {
        passed: 0,
        pending: 0,
        failed: 0,
    };
    for check in response.status_check_rollup {
        if check.status != "COMPLETED" {
            checks.pending += 1;
            continue;
        }
        match check.conclusion.as_deref().unwrap_or_default() {
            "SUCCESS" | "NEUTRAL" | "SKIPPED" => checks.passed += 1,
            "" => checks.pending += 1,
            _ => checks.failed += 1,
        }
    }
    Ok(PullRequest {
        number: response.number,
        title: response.title,
        url: response.url,
        author: response
            .author
            .map_or_else(|| "Unknown author".into(), |author| author.login),
        head: response.head_ref_name,
        head_oid: response.head_ref_oid,
        head_repository: response
            .head_repository
            .map(|repository| repository.name_with_owner)
            .unwrap_or_default(),
        base: response.base_ref_name,
        base_oid: response.base_ref_oid,
        additions: response.additions,
        deletions: response.deletions,
        changed_files: response.changed_files,
        draft: response.is_draft,
        mergeable: response.mergeable,
        merge_state: response.merge_state_status,
        review_decision: response.review_decision,
        checks,
    })
}

fn git(directory: &Path, wsl_distribution: &str, arguments: &[&str]) -> Result<Output, String> {
    super::git_in(directory, wsl_distribution, arguments)
        .map_err(|error| format!("Git could not start: {error}"))
}

fn github_repository(repository: &Repository) -> Result<&str, String> {
    repository
        .owner_and_name
        .as_deref()
        .ok_or_else(|| "The origin remote is not a GitHub repository.".into())
}

fn pull_request_view_command(repository: &Repository, number: u64) -> Result<Command, String> {
    let owner_and_name = github_repository(repository)?;
    let mut command = console_command("gh");
    command.args([
        "pr",
        "view",
        &number.to_string(),
        "--repo",
        owner_and_name,
        "--json",
        "number,title,url,author,headRefName,headRefOid,headRepository,baseRefName,baseRefOid,additions,deletions,changedFiles,isDraft,mergeable,mergeStateStatus,reviewDecision,statusCheckRollup",
    ]);
    Ok(command)
}

fn parse_status(status: &str) -> Vec<FileChange> {
    status
        .lines()
        .filter_map(|line| {
            if line.len() < 4 {
                return None;
            }
            let code = line.get(..2)?.trim();
            let raw_path = line.get(3..)?.trim();
            let path = raw_path
                .rsplit_once(" -> ")
                .map_or(raw_path, |(_, destination)| destination)
                .trim_matches('"')
                .to_owned();
            let status = match code.chars().next().unwrap_or('M') {
                '?' => "Untracked",
                'A' => "Added",
                'D' => "Deleted",
                'R' => "Renamed",
                'C' => "Copied",
                'U' => "Conflict",
                _ => "Modified",
            }
            .to_owned();
            Some(FileChange {
                path,
                previous_path: raw_path
                    .rsplit_once(" -> ")
                    .map(|(source, _)| source.trim_matches('"').to_owned()),
                status,
                additions: 0,
                deletions: 0,
                patch: None,
            })
        })
        .collect()
}

fn apply_numstat(files: &mut [FileChange], numstat: &str) {
    let counts: BTreeMap<String, (usize, usize)> = numstat
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let additions = fields.next()?.parse().unwrap_or(0);
            let deletions = fields.next()?.parse().unwrap_or(0);
            let path = fields.next()?.to_owned();
            Some((path, (additions, deletions)))
        })
        .collect();
    for file in files {
        if let Some((additions, deletions)) = counts.get(&file.path) {
            file.additions = *additions;
            file.deletions = *deletions;
        }
    }
}

fn parse_github_remote(remote: &str) -> Option<String> {
    let trimmed = remote.trim().trim_end_matches(".git");
    let repository = if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("ssh://git@github.com/") {
        rest
    } else {
        trimmed.strip_prefix("https://github.com/")?
    };
    (repository.split('/').count() == 2).then(|| repository.to_owned())
}

fn stdout_line(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn output_failure(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    } else {
        stderr
    }
}

fn nonempty_failure(output: &Output, fallback: &str) -> String {
    let failure = output_failure(output);
    if failure.is_empty() {
        fallback.into()
    } else {
        failure
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_remote_parser_accepts_https_and_ssh() {
        assert_eq!(
            parse_github_remote("https://github.com/Phoenixmatrix/muxtrix.git"),
            Some("Phoenixmatrix/muxtrix".into())
        );
        assert_eq!(
            parse_github_remote("git@github.com:Phoenixmatrix/muxtrix.git"),
            Some("Phoenixmatrix/muxtrix".into())
        );
        assert_eq!(parse_github_remote("git@example.com:a/b.git"), None);
    }

    #[test]
    fn pull_request_lookup_does_not_use_the_wsl_path_as_a_windows_working_directory() {
        let repository = Repository {
            root: "/home/user/dev/muxtrix".into(),
            name: "muxtrix".into(),
            owner_and_name: Some("Phoenixmatrix/muxtrix".into()),
            branch: "wsl-fix".into(),
            wsl_distribution: "Ubuntu-24.04".into(),
        };
        let command =
            pull_request_view_command(&repository, 42).expect("GitHub command should build");
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(command.get_current_dir(), None);
        assert!(
            arguments
                .windows(2)
                .any(|arguments| { arguments == ["--repo", "Phoenixmatrix/muxtrix"] })
        );
        assert!(arguments.iter().any(|argument| argument == "42"));
    }

    #[test]
    fn pull_request_search_matches_identity_and_branches() {
        let pull_request = PullRequestSummary {
            number: 391,
            title: "Native GitHub review panel".into(),
            url: "https://github.com/example/repo/pull/391".into(),
            author: "phoenixmatrix".into(),
            head: "github-support".into(),
            base: "main".into(),
            status: PullRequestSummaryStatus::Open,
        };

        assert!(pull_request.matches("review panel"));
        assert!(pull_request.matches("#391"));
        assert!(pull_request.matches("PHOENIX"));
        assert!(pull_request.matches("github-support"));
        assert!(!pull_request.matches("unrelated"));
    }

    #[test]
    fn pull_request_list_parser_keeps_searchable_identity() {
        let pull_requests = parse_pull_request_summaries(
            br#"[{"number":17,"title":"Keep diffs readable","url":"https://github.com/example/repo/pull/17","author":{"login":"octocat"},"headRefName":"diff-wrap","baseRefName":"main","isDraft":false}]"#,
        )
        .expect("GitHub list should parse");

        assert_eq!(pull_requests.len(), 1);
        assert_eq!(pull_requests[0].number, 17);
        assert_eq!(pull_requests[0].author, "octocat");
        assert_eq!(pull_requests[0].status, PullRequestSummaryStatus::Open);
        assert!(pull_requests[0].matches("diff-wrap"));
    }

    #[test]
    fn status_and_numstat_form_truthful_file_rows() {
        let mut files =
            parse_status(" M src/main.rs\nA  src/new.rs\nR  old.rs -> src/moved.rs\n?? notes.md\n");
        apply_numstat(
            &mut files,
            "12\t3\tsrc/main.rs\n7\t0\tsrc/new.rs\n1\t1\tsrc/moved.rs\n",
        );
        assert_eq!(files.len(), 4);
        assert_eq!(files[0].additions, 12);
        assert_eq!(files[2].path, "src/moved.rs");
        assert_eq!(files[2].previous_path.as_deref(), Some("old.rs"));
        assert_eq!(files[3].status, "Untracked");
    }

    #[test]
    fn unified_diff_parser_tracks_line_numbers_and_kinds() {
        let document = parse_diff(
            b"diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -10,3 +10,4 @@ fn main() {\n unchanged\n-old\n+new\n+extra\n trailing\n",
        );

        assert!(!document.truncated);
        assert_eq!(document.lines[0].kind, DiffLineKind::Metadata);
        assert_eq!(document.lines[3].kind, DiffLineKind::Hunk);
        assert_eq!(
            (document.lines[4].old_line, document.lines[4].new_line),
            (Some(10), Some(10))
        );
        assert_eq!(
            (document.lines[5].kind, document.lines[5].old_line),
            (DiffLineKind::Deletion, Some(11))
        );
        assert_eq!(
            (document.lines[6].kind, document.lines[6].new_line),
            (DiffLineKind::Addition, Some(11))
        );
        assert_eq!(document.lines[8].old_line, Some(12));
        assert_eq!(document.lines[8].new_line, Some(13));
    }

    #[test]
    fn unified_diff_parser_bounds_large_documents() {
        let input = " context\n".repeat(DIFF_MAX_LINES + 1);
        let document = parse_diff(input.as_bytes());

        assert_eq!(document.lines.len(), DIFF_MAX_LINES);
        assert!(document.truncated);
        assert!(
            document
                .notice
                .as_deref()
                .is_some_and(|notice| notice.contains("truncated"))
        );
    }

    #[test]
    fn diff_paths_must_stay_inside_the_repository() {
        assert!(validate_relative_git_path("src/main.rs").is_ok());
        assert!(validate_relative_git_path("../secrets.txt").is_err());
        assert!(validate_relative_git_path("/etc/passwd").is_err());
        assert!(validate_relative_git_path("").is_err());
    }

    #[test]
    fn partial_github_patches_are_labeled_instead_of_presented_as_complete() {
        let repository = Repository {
            root: "/unused".into(),
            name: "muxtrix".into(),
            owner_and_name: Some("example/muxtrix".into()),
            branch: "diff-viewer".into(),
            wsl_distribution: String::new(),
        };
        let file = FileChange {
            path: "src/main.rs".into(),
            previous_path: None,
            status: "Modified".into(),
            additions: 8,
            deletions: 4,
            patch: Some("@@ -1 +1 @@\n-old\n+new".into()),
        };

        let document = load_diff(&repository, &file, true).expect("patch should parse");
        assert!(document.truncated);
        assert!(
            document
                .notice
                .as_deref()
                .is_some_and(|notice| notice.contains("only part"))
        );
    }

    #[test]
    fn omitted_large_text_patch_can_be_reconstructed_from_file_contents() {
        let mut before = (0..12_000)
            .map(|line| format!("shared line {line}\n"))
            .collect::<String>();
        let mut after = before.clone();
        before.push_str("old ending\n");
        after.push_str("new ending\n");

        let document = diff_remote_file_contents(before.as_bytes(), after.as_bytes())
            .expect("large textual source should diff");

        assert!(
            document
                .lines
                .iter()
                .any(|line| line.kind == DiffLineKind::Deletion && line.text == "-old ending")
        );
        assert!(
            document
                .lines
                .iter()
                .any(|line| line.kind == DiffLineKind::Addition && line.text == "+new ending")
        );
        assert!(document.notice.is_none());
    }

    #[test]
    fn github_content_paths_are_encoded_without_losing_directory_boundaries() {
        assert_eq!(
            percent_encode_github_path("docs/design notes/日本語+#.md"),
            "docs/design%20notes/%E6%97%A5%E6%9C%AC%E8%AA%9E%2B%23.md"
        );
    }

    #[test]
    fn binary_detection_is_bounded_and_explicit() {
        assert!(looks_binary(b"header\0payload"));
        assert!(!looks_binary(b"ordinary UTF-8 text\n"));
    }

    #[cfg(unix)]
    #[test]
    fn reconstructed_diff_files_are_owner_only_and_removed_after_use() {
        use std::os::unix::fs::PermissionsExt as _;

        let root;
        {
            let files = TemporaryDiffFiles::create(b"private base", b"private head")
                .expect("private diff files should be created");
            root = files.root.clone();
            assert_eq!(
                std::fs::metadata(&files.root)
                    .expect("directory metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(&files.before)
                    .expect("file metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert!(!root.exists());
    }

    #[test]
    fn diagnostic_drain_stores_only_its_declared_limit() {
        let bytes = vec![b'x'; 128 * 1024];
        assert_eq!(
            drain_with_storage_limit(bytes.as_slice(), 64 * 1024).len(),
            64 * 1024
        );
    }

    #[test]
    fn pull_request_parser_keeps_both_blob_repositories_and_oids() {
        let pull_request = parse_pull_request(
            br#"{"number":42,"title":"Large diff","url":"https://github.com/example/repo/pull/42","author":{"login":"octocat"},"headRefName":"large-diff","headRefOid":"head123","headRepository":{"nameWithOwner":"fork/repo"},"baseRefName":"main","baseRefOid":"base456","additions":2,"deletions":1,"changedFiles":1,"isDraft":false,"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","reviewDecision":"APPROVED","statusCheckRollup":[]}"#,
        )
        .expect("pull request should parse");

        assert_eq!(pull_request.head_repository, "fork/repo");
        assert_eq!(pull_request.head_oid, "head123");
        assert_eq!(pull_request.base_oid, "base456");
    }

    #[test]
    fn pull_request_readiness_accounts_for_checks_and_conflicts() {
        let mut pull_request = PullRequest {
            number: 42,
            title: "GitHub panel".into(),
            url: "https://github.com/example/repo/pull/42".into(),
            author: "octocat".into(),
            head: "github-panel".into(),
            head_oid: "abc123".into(),
            head_repository: "example/repo".into(),
            base: "main".into(),
            base_oid: "def456".into(),
            additions: 40,
            deletions: 8,
            changed_files: 4,
            draft: false,
            mergeable: "MERGEABLE".into(),
            merge_state: "CLEAN".into(),
            review_decision: "APPROVED".into(),
            checks: CheckSummary {
                passed: 3,
                pending: 0,
                failed: 0,
            },
        };
        assert_eq!(pull_request.readiness(), MergeReadiness::Ready);
        pull_request.checks.failed = 1;
        assert_eq!(pull_request.readiness(), MergeReadiness::ChecksFailed);
        pull_request.checks.failed = 0;
        pull_request.mergeable = "CONFLICTING".into();
        assert_eq!(pull_request.readiness(), MergeReadiness::Conflicts);
    }

    #[test]
    fn paginated_pull_request_files_are_flattened() {
        let files = parse_pull_request_files(
            br#"[[{"filename":"src/main.rs","status":"renamed","previous_filename":"src/lib.rs","additions":12,"deletions":3,"patch":"@@ -1 +1 @@\n-old\n+new"}],[{"filename":"src/new.rs","status":"added","additions":8,"deletions":0}]]"#,
        )
        .expect("pull request file payload should parse");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].status, "Renamed");
        assert_eq!(files[0].previous_path.as_deref(), Some("src/lib.rs"));
        assert!(
            files[0]
                .patch
                .as_deref()
                .is_some_and(|patch| patch.contains("+new"))
        );
        assert_eq!(files[1].status, "Added");
        assert_eq!(files[1].additions, 8);
    }
}
