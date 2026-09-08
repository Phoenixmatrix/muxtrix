//! Dedicated task checkouts. Completion removes the checkout, never its branch.

use std::path::{Path, PathBuf};
use std::process::Output;

use muxtrix_domain::TaskWorktree;

use crate::app::{git_in, worktree_base_directory, worktree_destination};

const ADJECTIVES: &[&str] = &[
    "bold", "calm", "clever", "eager", "gentle", "happy", "keen", "kind", "lively", "lucid",
    "merry", "noble", "quiet", "sharp", "swift", "vivid",
];
const SURNAMES: &[&str] = &[
    "babbage", "bell", "bohr", "curie", "darwin", "einstein", "faraday", "franklin", "hamilton",
    "hopper", "johnson", "lovelace", "maxwell", "noether", "shannon", "turing",
];

pub(crate) fn create(directory: &Path, wsl_distribution: &str) -> Result<TaskWorktree, String> {
    create_at(
        directory,
        wsl_distribution,
        None,
        uuid::Uuid::new_v4().as_u128(),
    )
}

fn create_at(
    directory: &Path,
    distribution: &str,
    base: Option<&Path>,
    seed: u128,
) -> Result<TaskWorktree, String> {
    let base_ref = git_text(
        directory,
        distribution,
        &["symbolic-ref", "--quiet", "HEAD"],
    )
    .map_err(|error| {
        format!("Create Task Pane requires a checked-out branch, not detached HEAD: {error}")
    })?;
    validate_base_ref(&base_ref)?;
    // Pin creation to the starting commit, but retain the live branch for completion.
    let start = commit(directory, distribution, &base_ref)?;
    let entries = registrations(directory, distribution)?;
    let primary = entries.first().ok_or("Git returned no worktrees")?;
    if primary.bare {
        return Err("Create Task Pane requires a non-bare repository".into());
    }
    let repo_root = primary.path.clone();
    let default_base;
    let base = if let Some(base) = base {
        base
    } else {
        default_base = worktree_base_directory(&repo_root, distribution)
            .ok_or("Could not determine the task worktree directory")?;
        &default_base
    };
    prepare_base(base, distribution)?;
    for attempt in 0..4096_u128 {
        let name = candidate_name(seed, attempt);
        let reference = format!("refs/heads/{name}");
        if branch_exists(&repo_root, distribution, &reference)? {
            continue;
        }
        let path = worktree_destination(base, &name);
        // Reserve exclusively: `git worktree add` itself accepts existing empty directories.
        if !reserve_directory(&path, distribution)? {
            continue;
        }
        let destination = path_text(&path)?;
        let result = git_success(
            &repo_root,
            distribution,
            &["worktree", "add", "-b", &name, "--", destination, &start],
        );
        if let Err(error) = result {
            // Never recursively clean up an uncertain failed checkout or delete its branch.
            release_empty_directory(&path, distribution);
            return Err(format!(
                "Could not create task {name}: {error}. Any branch or non-empty checkout at {} has been kept.",
                path.display()
            ));
        }
        // Git resolves symlinked parent directories; persist its canonical spelling.
        let path = PathBuf::from(
            git_text(&path, distribution, &["rev-parse", "--show-toplevel"]).map_err(|error| {
                format!("Task checkout was created at {} but could not be resolved: {error}. It has been kept.", path.display())
            })?,
        );
        let task = TaskWorktree {
            repo_root,
            path,
            branch: name,
            base_ref,
            wsl_distribution: distribution.to_owned(),
        };
        validate_identity(&task).map_err(|error| {
            format!("Task checkout was created at {} but could not be verified: {error}. It has been kept.", task.path.display())
        })?;
        return Ok(task);
    }
    Err("Could not reserve an unused task name; no existing checkout was reused".into())
}

fn candidate_name(seed: u128, attempt: u128) -> String {
    let count = (ADJECTIVES.len() * SURNAMES.len()) as u128;
    let index = (seed % count + attempt % count) % count;
    let adjective = ADJECTIVES[(index / SURNAMES.len() as u128) as usize];
    let surname = SURNAMES[(index % SURNAMES.len() as u128) as usize];
    if attempt < count {
        format!("{adjective}_{surname}")
    } else {
        format!("{adjective}_{surname}-{:x}-{}", seed, attempt / count)
    }
}

/// `None` means clean and fully merged into the live starting branch.
/// Errors are not consent to discard: missing refs and uncertain identity block completion.
pub(crate) fn inspect(task: &TaskWorktree) -> Result<Option<String>, String> {
    validate_identity(task)?;
    let base = commit(&task.repo_root, &task.wsl_distribution, &task.base_ref)?;
    let head = commit(&task.path, &task.wsl_distribution, "HEAD")?;
    let status = git_success(
        &task.path,
        &task.wsl_distribution,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored",
            "--ignore-submodules=none",
        ],
    )?;
    // Git status deliberately hides modifications behind these index flags.
    // Treat them as uncertainty instead of declaring potentially valuable files clean.
    let tracked = git_success(
        &task.path,
        &task.wsl_distribution,
        &["ls-files", "-v", "-z"],
    )?;
    let hidden_changes = tracked.stdout.split(|byte| *byte == 0).any(|entry| {
        entry
            .first()
            .is_some_and(|tag| tag.is_ascii_lowercase() || *tag == b'S')
    });
    let submodules = git_success(
        &task.path,
        &task.wsl_distribution,
        &["submodule", "status", "--recursive"],
    )?;
    let initialized_submodules = submodules
        .stdout
        .split(|byte| *byte == b'\n')
        .any(|line| !line.is_empty() && line[0] != b'-');
    let unmerged = git_text(
        &task.repo_root,
        &task.wsl_distribution,
        &["rev-list", "--count", &head, "--not", &base, "--"],
    )?
    .parse::<u64>()
    .map_err(|_| "Git returned an invalid unmerged commit count")?;
    let mut risks = Vec::new();
    if !status.stdout.is_empty() {
        risks.push("This checkout contains staged, unstaged, untracked, or ignored files that will be deleted.".to_owned());
    }
    if hidden_changes {
        risks.push("Some tracked files use assume-unchanged or skip-worktree flags, so Git may hide local changes. Completing will delete these files.".to_owned());
    }
    if initialized_submodules {
        // Git requires force even when initialized submodules appear clean;
        // their own ignored files and local repository state need consent too.
        risks.push("This checkout contains initialized submodules. Completing removes their checkout files and local worktree state as well.".to_owned());
    }
    if unmerged != 0 {
        risks.push(format!(
            "{unmerged} task commit(s) are not merged into {}. The task branch will be kept.",
            task.base_ref
                .strip_prefix("refs/heads/")
                .unwrap_or(&task.base_ref)
        ));
    }
    Ok((!risks.is_empty()).then(|| risks.join("\n\n")))
}

/// Call only after the pane's processes have stopped. A single `--force` is used
/// only with explicit discard consent; locked worktrees are never unlocked.
/// Branches remain available for recovery even after discard.
pub(crate) fn remove(task: &TaskWorktree, discard: bool) -> Result<(), String> {
    if let Some(warning) = inspect(task)?
        && !discard
    {
        return Err(warning);
    }
    validate_identity(task)?;
    let path = path_text(&task.path)?;
    let args = if discard {
        vec!["worktree", "remove", "--force", "--", path]
    } else {
        vec!["worktree", "remove", "--", path]
    };
    git_success(&task.repo_root, &task.wsl_distribution, &args)?;
    Ok(())
}

#[derive(Default)]
struct Registration {
    path: PathBuf,
    branch: Option<String>,
    locked: bool,
    prunable: bool,
    bare: bool,
}

// The existing display parser discards locks and uses newline-delimited paths.
// Destructive operations need NUL-delimited records, including those protections.
fn registrations(directory: &Path, distribution: &str) -> Result<Vec<Registration>, String> {
    let output = git_success(
        directory,
        distribution,
        &["worktree", "list", "--porcelain", "-z"],
    )?;
    let text =
        String::from_utf8(output.stdout).map_err(|_| "Git worktree paths are not valid UTF-8")?;
    let mut entries = Vec::new();
    let mut current: Option<Registration> = None;
    for field in text.split('\0') {
        if let Some(path) = field.strip_prefix("worktree ") {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            current = Some(Registration {
                path: PathBuf::from(path),
                ..Registration::default()
            });
        } else if let Some(entry) = current.as_mut() {
            if let Some(branch) = field.strip_prefix("branch ") {
                entry.branch = Some(branch.to_owned());
            } else if field == "locked" || field.starts_with("locked ") {
                entry.locked = true;
            } else if field == "prunable" || field.starts_with("prunable ") {
                entry.prunable = true;
            } else if field == "bare" {
                entry.bare = true;
            }
        }
    }
    if let Some(entry) = current {
        entries.push(entry);
    }
    Ok(entries)
}

fn validate_base_ref(reference: &str) -> Result<(), String> {
    if !reference.starts_with("refs/heads/") {
        return Err("Task has no valid starting branch; completion is blocked".into());
    }
    Ok(())
}

fn validate_identity(task: &TaskWorktree) -> Result<(), String> {
    validate_base_ref(&task.base_ref)?;
    // Reject revision expressions in persisted metadata, not merely missing refs.
    for reference in [&task.base_ref, &format!("refs/heads/{}", task.branch)] {
        git_success(
            &task.repo_root,
            &task.wsl_distribution,
            &["check-ref-format", reference],
        )?;
    }
    let entries = registrations(&task.repo_root, &task.wsl_distribution)?;
    let primary = entries.first().ok_or("Git returned no worktrees")?;
    if primary.path != task.repo_root || primary.path == task.path || primary.bare {
        return Err(
            "Task repository identity changed or checkout is primary; completion is blocked".into(),
        );
    }
    let entry = entries
        .iter()
        .skip(1)
        .find(|entry| entry.path == task.path)
        .ok_or("Task checkout is no longer registered with its repository")?;
    if entry.locked || entry.prunable || entry.bare {
        return Err(
            "Task checkout is locked, unavailable, or protected; completion is blocked".into(),
        );
    }
    let expected = format!("refs/heads/{}", task.branch);
    if entry.branch.as_deref() != Some(expected.as_str()) {
        return Err("Task checkout branch changed; completion is blocked".into());
    }
    let root = git_text(
        &task.path,
        &task.wsl_distribution,
        &["rev-parse", "--show-toplevel"],
    )?;
    let branch = git_text(
        &task.path,
        &task.wsl_distribution,
        &["symbolic-ref", "--quiet", "HEAD"],
    )?;
    let common_args = ["rev-parse", "--path-format=absolute", "--git-common-dir"];
    let common = git_text(&task.path, &task.wsl_distribution, &common_args)?;
    let expected_common = git_text(&task.repo_root, &task.wsl_distribution, &common_args)?;
    if Path::new(&root) != task.path
        || branch != expected
        || Path::new(&common) != Path::new(&expected_common)
    {
        return Err("Task checkout identity changed; completion is blocked".into());
    }
    Ok(())
}

fn path_text(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| "Task path is not valid UTF-8".into())
}

fn git_success(directory: &Path, distribution: &str, args: &[&str]) -> Result<Output, String> {
    let output = git_in(directory, distribution, args)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(format!(
            "Git {} failed: {}",
            args[0],
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn git_text(directory: &Path, distribution: &str, args: &[&str]) -> Result<String, String> {
    let output = git_success(directory, distribution, args)?;
    let text = String::from_utf8(output.stdout).map_err(|_| "Git returned invalid UTF-8")?;
    let text = text.strip_suffix('\n').unwrap_or(&text);
    Ok(text.strip_suffix('\r').unwrap_or(text).to_owned())
}

fn commit(directory: &Path, distribution: &str, reference: &str) -> Result<String, String> {
    git_text(
        directory,
        distribution,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{reference}^{{commit}}"),
        ],
    )
}

fn branch_exists(directory: &Path, distribution: &str, reference: &str) -> Result<bool, String> {
    // A branch namespace is occupied too: `name/topic` prevents Git from
    // creating `name`, even though an exact show-ref lookup reports no branch.
    let output = git_success(
        directory,
        distribution,
        &[
            "for-each-ref",
            "--count=1",
            "--format=%(refname)",
            reference,
        ],
    )?;
    Ok(!output.stdout.is_empty())
}

#[cfg(target_os = "windows")]
fn wsl_fs(distribution: &str, args: &[&str], path: &Path) -> Result<Output, String> {
    use crate::process::{HELPER_COMMAND_TIMEOUT, ProcessCancellation, command_output};
    let mut command = crate::app::wsl_command(distribution);
    command.arg("--exec").args(args).arg(path);
    command_output(
        &mut command,
        HELPER_COMMAND_TIMEOUT,
        &ProcessCancellation::default(),
    )
    .map_err(|error| format!("Could not access WSL task directory: {error}"))
}

fn prepare_base(base: &Path, distribution: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    if crate::app::path_is_wsl_side(base) {
        let output = wsl_fs(distribution, &["mkdir", "-p", "--"], base)?;
        return if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
        };
    }
    let _ = distribution;
    std::fs::create_dir_all(base)
        .map_err(|error| format!("Could not create task directory: {error}"))
}

fn reserve_directory(path: &Path, distribution: &str) -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    if crate::app::path_is_wsl_side(path) {
        let output = wsl_fs(distribution, &["mkdir", "--"], path)?;
        if output.status.success() {
            return Ok(true);
        }
        // Include broken symlinks: those names are occupied too.
        for flag in ["-e", "-L"] {
            let exists = wsl_fs(distribution, &["test", flag], path)?;
            match exists.status.code() {
                Some(0) => return Ok(false),
                Some(1) => {}
                _ => return Err("Could not check WSL task directory collision".into()),
            }
        }
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let _ = distribution;
    match std::fs::create_dir(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(format!("Could not reserve task directory: {error}")),
    }
}

fn release_empty_directory(path: &Path, distribution: &str) {
    #[cfg(target_os = "windows")]
    if crate::app::path_is_wsl_side(path) {
        let _ = wsl_fs(distribution, &["rmdir", "--"], path);
        return;
    }
    let _ = distribution;
    let _ = std::fs::remove_dir(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Repository {
        scratch: PathBuf,
        root: PathBuf,
        base: PathBuf,
    }

    impl Repository {
        fn new() -> Self {
            let scratch =
                std::env::temp_dir().join(format!("muxtrix-task-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&scratch).expect("isolated scratch directory");
            let repository = Self {
                root: scratch.join("source"),
                base: scratch.join("tasks"),
                scratch,
            };
            std::fs::create_dir(&repository.root).expect("repository directory");
            repository.git(
                &repository.root,
                &["-c", "init.templateDir=", "init", "-q", "-b", "main"],
            );
            repository.git(&repository.root, &["config", "user.name", "Task Test"]);
            repository.git(
                &repository.root,
                &["config", "user.email", "task@example.invalid"],
            );
            repository.git(&repository.root, &["config", "commit.gpgsign", "false"]);
            repository.git(&repository.root, &["config", "core.autocrlf", "false"]);
            std::fs::write(repository.root.join("tracked"), "original\n").expect("tracked file");
            std::fs::write(repository.root.join(".gitignore"), "ignored/\n").expect("ignore rules");
            repository.git(&repository.root, &["add", "."]);
            repository.git(&repository.root, &["commit", "-qm", "initial"]);
            repository
        }

        fn git(&self, directory: &Path, args: &[&str]) {
            git_success(directory, "", args).unwrap_or_else(|error| panic!("{args:?}: {error}"));
        }

        fn task(&self, seed: u128) -> TaskWorktree {
            create_at(&self.root, "", Some(&self.base), seed).expect("create task")
        }

        fn commit_task(&self, task: &TaskWorktree) {
            std::fs::write(task.path.join("tracked"), "task change\n").expect("task file");
            self.git(&task.path, &["add", "tracked"]);
            self.git(&task.path, &["commit", "-qm", "task work"]);
        }
    }

    impl Drop for Repository {
        fn drop(&mut self) {
            // Everything, including Git's linked registrations, lives under scratch.
            let _ = std::fs::remove_dir_all(&self.scratch);
        }
    }

    #[test]
    fn names_never_reuse_existing_branches_or_even_empty_directories() {
        let repo = Repository::new();
        let branch_name = candidate_name(0, 0);
        let empty_name = candidate_name(0, 1);
        let occupied_name = candidate_name(0, 2);
        repo.git(&repo.root, &["branch", &branch_name]);
        let original =
            commit(&repo.root, "", &format!("refs/heads/{branch_name}")).expect("existing branch");
        std::fs::create_dir_all(repo.base.join(&empty_name)).expect("existing empty directory");
        std::fs::create_dir(repo.base.join(&occupied_name)).expect("existing occupied directory");
        std::fs::write(repo.base.join(&occupied_name).join("keep"), "valuable")
            .expect("existing data");
        let task = repo.task(0);
        assert_eq!(task.branch, candidate_name(0, 3));
        assert_eq!(
            commit(&repo.root, "", &format!("refs/heads/{branch_name}")).expect("existing branch"),
            original
        );
        assert!(repo.base.join(&empty_name).is_dir());
        assert_eq!(
            std::fs::read_to_string(repo.base.join(&occupied_name).join("keep"))
                .expect("preserved data"),
            "valuable"
        );
        assert_eq!(inspect(&task).expect("inspection"), None);
        remove(&task, false).expect("safe completion");
        assert!(!task.path.exists());
        assert!(
            branch_exists(&repo.root, "", &format!("refs/heads/{}", task.branch))
                .expect("preserved task branch")
        );
    }

    #[test]
    fn names_skip_existing_branch_namespaces() {
        let repo = Repository::new();
        let occupied = format!("{}/topic", candidate_name(0, 0));
        repo.git(&repo.root, &["branch", &occupied]);
        let original =
            commit(&repo.root, "", &format!("refs/heads/{occupied}")).expect("namespace branch");
        let task = repo.task(0);
        assert_ne!(task.branch, candidate_name(0, 0));
        assert_eq!(
            commit(&repo.root, "", &format!("refs/heads/{occupied}")).expect("preserved branch"),
            original
        );
        remove(&task, false).expect("complete distinct task");
    }

    #[test]
    fn unmerged_commits_block_completion_until_live_base_contains_them() {
        let repo = Repository::new();
        let task = repo.task(0);
        repo.commit_task(&task);
        assert!(inspect(&task).expect("inspection").is_some());
        assert!(remove(&task, false).is_err());
        assert!(task.path.is_dir());
        repo.git(&repo.root, &["merge", "--ff-only", &task.branch]);
        assert_eq!(inspect(&task).expect("merged inspection"), None);
        remove(&task, false).expect("merged completion");
        assert!(!task.path.exists());
    }

    #[test]
    fn completion_rechecks_changes_made_after_initial_inspection() {
        let repo = Repository::new();
        let task = repo.task(0);
        assert_eq!(inspect(&task).expect("initial inspection"), None);
        std::fs::write(task.path.join("tracked"), "unstaged work").expect("unstaged file");
        assert!(remove(&task, false).is_err());
        repo.git(&task.path, &["add", "tracked"]);
        assert!(remove(&task, false).is_err());
        assert_eq!(
            std::fs::read_to_string(task.path.join("tracked")).expect("kept staged work"),
            "unstaged work"
        );
        remove(&task, true).expect("explicit discard");
        assert!(!task.path.exists());
    }

    #[test]
    fn untracked_and_ignored_files_each_require_explicit_discard() {
        let repo = Repository::new();
        let task = repo.task(0);
        std::fs::write(task.path.join("untracked"), "untracked work").expect("untracked file");
        assert!(remove(&task, false).is_err());
        std::fs::remove_file(task.path.join("untracked")).expect("remove fixture");
        std::fs::create_dir(task.path.join("ignored")).expect("ignored directory");
        std::fs::write(task.path.join("ignored").join("secret"), "ignored work")
            .expect("ignored file");
        assert!(inspect(&task).expect("ignored inspection").is_some());
        assert!(remove(&task, false).is_err());
        assert_eq!(
            std::fs::read_to_string(task.path.join("ignored").join("secret"))
                .expect("kept ignored work"),
            "ignored work"
        );
        remove(&task, true).expect("explicit ignored discard");
        assert!(!task.path.exists());
    }

    #[test]
    fn discard_preserves_unmerged_commits_on_the_task_branch() {
        let repo = Repository::new();
        let task = repo.task(0);
        repo.commit_task(&task);
        let head = commit(&task.path, "", "HEAD").expect("task commit");
        remove(&task, true).expect("explicit unmerged completion");
        assert!(!task.path.exists());
        assert_eq!(
            commit(&repo.root, "", &format!("refs/heads/{}", task.branch))
                .expect("recoverable commit"),
            head
        );
    }

    #[test]
    fn initialized_submodules_require_consent_even_when_superproject_is_clean() {
        let repo = Repository::new();
        let dependency = Repository::new();
        repo.git(
            &repo.root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "--",
                path_text(&dependency.root).expect("dependency path"),
                "dependency",
            ],
        );
        repo.git(&repo.root, &["commit", "-qm", "add dependency"]);
        let task = repo.task(0);
        repo.git(
            &task.path,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "update",
                "--init",
            ],
        );
        assert!(inspect(&task).expect("submodule inspection").is_some());
        assert!(remove(&task, false).is_err());
        assert!(task.path.join("dependency/tracked").exists());
        remove(&task, true).expect("confirmed submodule removal");
        assert!(!task.path.exists());
        assert!(dependency.root.join("tracked").exists());
    }

    #[test]
    fn linked_checkout_uses_its_own_live_branch_as_base_and_detached_creation_fails() {
        let repo = Repository::new();
        let parent = repo.task(0);
        repo.commit_task(&parent);
        std::fs::create_dir(parent.path.join("nested")).expect("nested working directory");
        let child = create_at(&parent.path.join("nested"), "", Some(&repo.base), 10)
            .expect("task from linked checkout");
        assert_eq!(child.repo_root, parent.repo_root);
        assert_eq!(child.base_ref, format!("refs/heads/{}", parent.branch));
        assert_eq!(
            commit(&child.path, "", "HEAD").expect("child head"),
            commit(&parent.path, "", "HEAD").expect("parent head")
        );
        assert_eq!(inspect(&child).expect("child inspection"), None);
        remove(&child, false).expect("child completion");
        repo.git(&parent.path, &["checkout", "--detach", "-q"]);
        assert!(create_at(&parent.path, "", Some(&repo.base), 20).is_err());
        assert!(remove(&parent, true).is_err());
        assert!(parent.path.exists());
    }

    #[test]
    fn locked_primary_and_switched_branch_checkouts_cannot_be_discarded() {
        let repo = Repository::new();
        let task = repo.task(0);
        repo.git(
            &repo.root,
            &[
                "worktree",
                "lock",
                "--reason",
                "keep this checkout",
                path_text(&task.path).expect("path"),
            ],
        );
        assert!(inspect(&task).is_err());
        assert!(remove(&task, true).is_err());
        assert!(task.path.exists());
        repo.git(
            &repo.root,
            &["worktree", "unlock", path_text(&task.path).expect("path")],
        );
        repo.git(&task.path, &["checkout", "-qb", "unrelated"]);
        assert!(remove(&task, true).is_err());
        assert!(task.path.exists());
        let primary = TaskWorktree {
            path: repo.root.clone(),
            branch: "main".into(),
            ..task
        };
        assert!(remove(&primary, true).is_err());
        assert!(repo.root.join("tracked").is_file());
    }

    #[test]
    fn missing_base_and_foreign_repository_metadata_fail_closed_even_on_discard() {
        let repo = Repository::new();
        let task = repo.task(0);
        let foreign = Repository::new();
        let mismatched = TaskWorktree {
            repo_root: foreign.root.clone(),
            ..task.clone()
        };
        assert!(remove(&mismatched, true).is_err());
        repo.git(&repo.root, &["update-ref", "-d", "refs/heads/main"]);
        assert!(inspect(&task).is_err());
        assert!(remove(&task, true).is_err());
        assert!(task.path.is_dir());
        assert!(foreign.root.join("tracked").is_file());
    }

    #[test]
    fn moved_or_replaced_worktree_is_not_removed_from_stale_metadata() {
        let repo = Repository::new();
        let task = repo.task(0);
        let moved = repo.scratch.join("moved");
        repo.git(
            &repo.root,
            &[
                "worktree",
                "move",
                path_text(&task.path).expect("path"),
                path_text(&moved).expect("moved path"),
            ],
        );
        std::fs::create_dir(&task.path).expect("replacement directory");
        std::fs::write(task.path.join("valuable"), "not the task checkout")
            .expect("replacement data");
        assert!(remove(&task, true).is_err());
        assert_eq!(
            std::fs::read_to_string(task.path.join("valuable")).expect("replacement kept"),
            "not the task checkout"
        );
        assert!(moved.join("tracked").exists());
    }

    #[test]
    fn index_flags_cannot_hide_local_changes_from_completion() {
        let repo = Repository::new();
        let task = repo.task(0);
        repo.git(
            &task.path,
            &["update-index", "--assume-unchanged", "tracked"],
        );
        std::fs::write(task.path.join("tracked"), "hidden work").expect("hidden modification");
        assert!(remove(&task, false).is_err());
        repo.git(
            &task.path,
            &["update-index", "--no-assume-unchanged", "tracked"],
        );
        repo.git(&task.path, &["update-index", "--skip-worktree", "tracked"]);
        assert!(remove(&task, false).is_err());
        assert_eq!(
            std::fs::read_to_string(task.path.join("tracked")).expect("kept hidden work"),
            "hidden work"
        );
        remove(&task, true).expect("explicit hidden-work discard");
        assert!(!task.path.exists());
    }
}
