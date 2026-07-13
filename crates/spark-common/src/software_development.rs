//! Enforced workspace primitives used by the bundled software-development flows.
//!
//! Worktrees and locks use the canonical Git common directory. Durable run state
//! lives beside the canonical Git common directory so linked worktrees share it.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug, thiserror::Error)]
pub enum SoftwareDevelopmentError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("git {operation} failed: {detail}")]
    Git { operation: String, detail: String },
    #[error("repository integration is already running")]
    Locked,
    #[error("target checkout is dirty")]
    DirtyTarget,
    #[error("target moved during integration (expected {expected}, found {actual})")]
    TargetMoved { expected: String, actual: String },
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, SoftwareDevelopmentError>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskInput {
    pub objective: Option<String>,
    pub artifact_path: Option<PathBuf>,
    #[serde(default)]
    pub target_paths: Vec<PathBuf>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    pub validation_command: Option<String>,
    pub base_ref: Option<String>,
    pub git_result: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedTask {
    pub objective: String,
    pub artifact_path: Option<PathBuf>,
    pub target_paths: Vec<PathBuf>,
    pub constraints: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub validation_command: Option<String>,
    pub base_ref: String,
    pub git_result: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepositoryIdentity {
    pub common_dir: PathBuf,
    pub work_tree: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManagedWorkspace {
    pub run_id: String,
    pub flow_id: String,
    pub branch: String,
    pub base_ref: String,
    pub base_commit: String,
    pub source_dirty: bool,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub worktree: PathBuf,
    pub run_dir: PathBuf,
}

pub const DIRTY_SOURCE_EXCLUSION_WARNING: &str =
    "Uncommitted source-checkout changes were excluded from the execution workspace.";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ValidationEvidence {
    pub command: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    pub outcome: String,
    pub branch: Option<String>,
    pub base_commit: String,
    pub worktree: Option<PathBuf>,
    #[serde(default)]
    pub commits: Vec<String>,
    #[serde(default)]
    pub validation: Vec<ValidationEvidence>,
    pub summary: String,
    pub base_divergence: Option<String>,
}

/// Validate an input and resolve artifact content to the same canonical objective
/// representation used by inline requests.
pub fn normalize_task(repository: &Path, input: TaskInput) -> Result<NormalizedTask> {
    validate_task_input(&input)?;
    let objective = input
        .objective
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let artifact_path = input
        .artifact_path
        .map(validate_relative_path)
        .transpose()?;
    let objective = if let Some(objective) = objective {
        objective.to_owned()
    } else {
        let relative = artifact_path.as_ref().expect("checked above");
        let path = repository.join(relative);
        let canonical_repository = repository.canonicalize()?;
        let canonical_path = path.canonicalize()?;
        if !canonical_path.starts_with(&canonical_repository) || !canonical_path.is_file() {
            return Err(SoftwareDevelopmentError::InvalidInput(
                "artifact_path must name a file inside the repository".into(),
            ));
        }
        fs::read_to_string(canonical_path)?.trim().to_owned()
    };
    let target_paths = input
        .target_paths
        .into_iter()
        .map(validate_relative_path)
        .collect::<Result<Vec<_>>>()?;
    Ok(NormalizedTask {
        objective,
        artifact_path,
        target_paths,
        constraints: input.constraints,
        acceptance_criteria: input.acceptance_criteria,
        validation_command: input.validation_command,
        base_ref: input.base_ref.unwrap_or_else(|| "HEAD".into()),
        git_result: input.git_result,
    })
}

/// Validate all request structure before creating any managed workspace.
pub fn validate_task_input(input: &TaskInput) -> Result<()> {
    let has_objective = input
        .objective
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    if has_objective == input.artifact_path.is_some() {
        return Err(SoftwareDevelopmentError::InvalidInput(
            "provide exactly one of objective or artifact_path".into(),
        ));
    }
    if let Some(path) = input.artifact_path.clone() {
        validate_relative_path(path)?;
    }
    for path in input.target_paths.iter().cloned() {
        validate_relative_path(path)?;
    }
    if let Some(base_ref) = input.base_ref.as_deref() {
        if base_ref.trim().is_empty() {
            return Err(SoftwareDevelopmentError::InvalidInput(
                "base_ref must be non-empty".into(),
            ));
        }
    }
    Ok(())
}

pub fn repository_identity(checkout: &Path) -> Result<RepositoryIdentity> {
    let common = git(checkout, ["rev-parse", "--git-common-dir"])?;
    let common = PathBuf::from(common.trim());
    let common = if common.is_absolute() {
        common
    } else {
        checkout.join(common)
    };
    Ok(RepositoryIdentity {
        common_dir: common.canonicalize()?,
        work_tree: checkout.canonicalize()?,
    })
}

/// An exclusive lock shared by every linked worktree of a repository.
pub struct RepositoryLock {
    path: PathBuf,
    _file: fs::File,
}

#[derive(Debug)]
pub struct MergeSession {
    pub workspace: ManagedWorkspace,
    pub source_ref: String,
    pub target_branch: String,
    pub target_commit: String,
    target_checkout: PathBuf,
    _lock: RepositoryLock,
}

impl std::fmt::Debug for RepositoryLock {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RepositoryLock")
            .field("path", &self.path)
            .finish()
    }
}

impl RepositoryLock {
    pub fn acquire(checkout: &Path) -> Result<Self> {
        let path = repository_identity(checkout)?
            .common_dir
            .join("spark-integration.lock");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    SoftwareDevelopmentError::Locked
                } else {
                    error.into()
                }
            })?;
        Ok(Self { path, _file: file })
    }
}

impl Drop for RepositoryLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn prepare_isolated_workspace(
    checkout: &Path,
    flow_id: &str,
    run_id: &str,
    base_ref: Option<&str>,
) -> Result<ManagedWorkspace> {
    validate_identifier("flow_id", flow_id)?;
    validate_identifier("run_id", run_id)?;
    let identity = repository_identity(checkout)?;
    let base_ref = base_ref.unwrap_or("HEAD");
    let commit_ref = format!("{base_ref}^{{commit}}");
    let base_commit = git(checkout, ["rev-parse", "--verify", &commit_ref])?
        .trim()
        .to_owned();
    let source_dirty = !git(
        checkout,
        ["status", "--porcelain", "--untracked-files=normal"],
    )?
    .is_empty();
    let root = identity.common_dir.join("spark");
    let run_dir = run_registry(&identity).join(run_id);
    let worktree = root.join("worktrees").join(run_id);
    fs::create_dir_all(&run_dir)?;
    fs::create_dir_all(worktree.parent().expect("has parent"))?;
    let branch = format!("spark/{flow_id}/{run_id}");
    git(
        checkout,
        [
            "worktree",
            "add",
            "-b",
            &branch,
            path_text(&worktree)?,
            &base_commit,
        ],
    )?;
    let workspace = ManagedWorkspace {
        run_id: run_id.into(),
        flow_id: flow_id.into(),
        branch,
        base_ref: base_ref.into(),
        base_commit,
        source_dirty,
        warnings: source_dirty
            .then(|| DIRTY_SOURCE_EXCLUSION_WARNING.to_string())
            .into_iter()
            .collect(),
        worktree,
        run_dir,
    };
    write_json_atomic(&workspace.run_dir.join("workspace.json"), &workspace)?;
    Ok(workspace)
}

pub fn record_task(run_dir: &Path, task: &NormalizedTask) -> Result<()> {
    write_json_atomic(&run_dir.join("task.json"), task)
}

pub fn record_result(run_dir: &Path, result: &RunResult) -> Result<()> {
    write_json_atomic(&run_dir.join("result.json"), result)
}

/// Record runtime-owned lifecycle fields while retaining substantive fields
/// authored by the flow. Malformed or non-object output is repaired by being
/// replaced with the standardized lifecycle result.
pub fn record_result_preserving_authored_output(run_dir: &Path, result: &RunResult) -> Result<()> {
    let path = run_dir.join("result.json");
    let mut output = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let lifecycle = serde_json::to_value(result)?
        .as_object()
        .cloned()
        .expect("RunResult serializes as an object");
    output.extend(lifecycle);
    write_json_atomic(&path, &output)
}

pub fn run_validation(worktree: &Path, command: &str) -> Result<ValidationEvidence> {
    let output = Command::new("sh")
        .args(["-c", command])
        .current_dir(worktree)
        .output()?;
    Ok(evidence(command, output))
}

/// Discover a deterministic repository-wide validation gate from executable
/// project metadata. Explicit caller input remains authoritative; this is the
/// safe launcher fallback for mutating flows.
pub fn discover_validation_command(repository: &Path) -> Result<String> {
    let mut commands = Vec::new();
    if repository.join("Cargo.toml").is_file() {
        commands.push("cargo fmt --all -- --check".to_string());
        commands.push("cargo test --workspace --all-features".to_string());
    }

    let frontend_package = repository.join("frontend/package.json");
    if frontend_package.is_file() {
        let package: Value = serde_json::from_slice(&fs::read(frontend_package)?)?;
        let scripts = package.get("scripts").and_then(Value::as_object);
        if scripts.is_some_and(|scripts| scripts.contains_key("test:unit")) {
            commands.push("npm --prefix frontend run test:unit".to_string());
        }
        if scripts.is_some_and(|scripts| scripts.contains_key("build")) {
            commands.push("npm --prefix frontend run build".to_string());
        }
    }

    let root_package = repository.join("package.json");
    if commands.is_empty() && root_package.is_file() {
        let package: Value = serde_json::from_slice(&fs::read(root_package)?)?;
        let scripts = package.get("scripts").and_then(Value::as_object);
        if scripts.is_some_and(|scripts| scripts.contains_key("test")) {
            commands.push("npm test".to_string());
        }
        if scripts.is_some_and(|scripts| scripts.contains_key("build")) {
            commands.push("npm run build".to_string());
        }
    }

    Ok(if commands.is_empty() {
        "git diff --check".into()
    } else {
        commands.join(" && ")
    })
}

pub fn finalize_commit(worktree: &Path, message: &str) -> Result<String> {
    git(worktree, ["add", "--all"])?;
    git(
        worktree,
        ["commit", "--allow-empty", "--no-gpg-sign", "-m", message],
    )?;
    Ok(git(worktree, ["rev-parse", "HEAD"])?.trim().into())
}

/// Consolidate all work since `base_commit`, including commits authored by an
/// agent, into the single validated commit required by bounded flows.
pub fn finalize_bounded_commit(
    worktree: &Path,
    base_commit: &str,
    message: &str,
) -> Result<String> {
    git(worktree, ["reset", "--soft", base_commit])?;
    finalize_commit(worktree, message)
}

pub fn commits_since(worktree: &Path, base_commit: &str) -> Result<Vec<String>> {
    Ok(git(
        worktree,
        ["rev-list", "--reverse", &format!("{base_commit}..HEAD")],
    )?
    .lines()
    .map(str::to_owned)
    .collect())
}

pub fn has_uncommitted_changes(worktree: &Path) -> Result<bool> {
    Ok(!git(
        worktree,
        ["status", "--porcelain", "--untracked-files=normal"],
    )?
    .is_empty())
}

pub fn base_divergence(checkout: &Path, base_ref: &str, base_commit: &str) -> Result<String> {
    let current_base = git(
        checkout,
        ["rev-parse", "--verify", &format!("{base_ref}^{{commit}}")],
    )?;
    Ok(git(
        checkout,
        [
            "rev-list",
            "--left-right",
            "--count",
            &format!("{base_commit}...{}", current_base.trim()),
        ],
    )?
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" "))
}

pub fn cleanup_success(checkout: &Path, workspace: &ManagedWorkspace) -> Result<()> {
    git(
        checkout,
        [
            "worktree",
            "remove",
            "--force",
            path_text(&workspace.worktree)?,
        ],
    )?;
    Ok(())
}

/// Create a detached checkout for a read-only flow and make its files
/// non-writable before any agent node is launched.
pub fn prepare_read_only_workspace(
    checkout: &Path,
    flow_id: &str,
    run_id: &str,
    base_ref: Option<&str>,
) -> Result<ManagedWorkspace> {
    validate_identifier("flow_id", flow_id)?;
    validate_identifier("run_id", run_id)?;
    let identity = repository_identity(checkout)?;
    let commit_ref = format!("{}^{{commit}}", base_ref.unwrap_or("HEAD"));
    let base_commit = git(checkout, ["rev-parse", "--verify", &commit_ref])?
        .trim()
        .to_owned();
    let source_dirty = !git(
        checkout,
        ["status", "--porcelain", "--untracked-files=normal"],
    )?
    .is_empty();
    let worktree = identity
        .common_dir
        .join("spark/read-only-worktrees")
        .join(run_id);
    let run_dir = run_registry(&identity).join(run_id);
    fs::create_dir_all(&run_dir)?;
    fs::create_dir_all(worktree.parent().expect("has parent"))?;
    git(
        checkout,
        [
            "worktree",
            "add",
            "--detach",
            path_text(&worktree)?,
            &base_commit,
        ],
    )?;
    set_tree_read_only(&worktree)?;
    let workspace = ManagedWorkspace {
        run_id: run_id.into(),
        flow_id: flow_id.into(),
        branch: String::new(),
        base_ref: base_ref.unwrap_or("HEAD").into(),
        base_commit,
        source_dirty,
        warnings: source_dirty
            .then(|| DIRTY_SOURCE_EXCLUSION_WARNING.to_string())
            .into_iter()
            .collect(),
        worktree,
        run_dir,
    };
    write_json_atomic(&workspace.run_dir.join("workspace.json"), &workspace)?;
    Ok(workspace)
}

pub fn cleanup_read_only(checkout: &Path, workspace: &ManagedWorkspace) -> Result<()> {
    set_tree_writable(&workspace.worktree)?;
    cleanup_success(checkout, workspace)
}

/// Integrate with an explicit merge commit, validating before the target is
/// fast-forwarded. Any failure intentionally leaves the integration worktree.
pub fn merge_change(
    target_checkout: &Path,
    source_ref: &str,
    run_id: &str,
    validation_command: &str,
) -> Result<RunResult> {
    let session = prepare_merge_change(target_checkout, source_ref, run_id)?;
    finish_merge_change(session, validation_command)
}

/// Prepare an integration worktree while retaining the repository lock. A
/// conflicted merge is deliberately returned to the caller so an execution
/// node can resolve behavior-preserving conflicts in the managed worktree.
pub fn prepare_merge_change(
    target_checkout: &Path,
    source_ref: &str,
    run_id: &str,
) -> Result<MergeSession> {
    validate_identifier("run_id", run_id)?;
    if source_ref.trim().is_empty() {
        return Err(SoftwareDevelopmentError::InvalidInput(
            "source_ref must be non-empty".into(),
        ));
    }
    git(
        target_checkout,
        ["rev-parse", "--verify", &format!("{source_ref}^{{commit}}")],
    )?;
    let _lock = RepositoryLock::acquire(target_checkout)?;
    ensure_clean(target_checkout)?;
    let target_branch = git(
        target_checkout,
        ["symbolic-ref", "--quiet", "--short", "HEAD"],
    )?
    .trim()
    .to_owned();
    let target_commit = git(target_checkout, ["rev-parse", "HEAD"])?
        .trim()
        .to_owned();
    let workspace = prepare_isolated_workspace(
        target_checkout,
        "merge-change",
        run_id,
        Some(&target_commit),
    )?;
    let merge = Command::new("git")
        .args(["merge", "--no-ff", "--no-commit", source_ref])
        .current_dir(&workspace.worktree)
        .output()?;
    if !merge.status.success() {
        let unmerged = git(
            &workspace.worktree,
            ["diff", "--name-only", "--diff-filter=U"],
        )?;
        if unmerged.trim().is_empty() {
            return Err(SoftwareDevelopmentError::Git {
                operation: "merge --no-ff --no-commit".into(),
                detail: String::from_utf8_lossy(&merge.stderr).trim().into(),
            });
        }
    }
    Ok(MergeSession {
        workspace,
        source_ref: source_ref.into(),
        target_branch,
        target_commit,
        target_checkout: target_checkout.into(),
        _lock,
    })
}

pub fn finish_merge_change(session: MergeSession, validation_command: &str) -> Result<RunResult> {
    if validation_command.trim().is_empty() {
        return Err(SoftwareDevelopmentError::InvalidInput(
            "validation_command must be non-empty".into(),
        ));
    }
    let MergeSession {
        workspace,
        source_ref,
        target_branch,
        target_commit,
        target_checkout,
        _lock,
    } = session;
    let mut attempted_validation = None;
    let attempted = (|| -> Result<RunResult> {
        git(&workspace.worktree, ["add", "--all"])?;
        let unmerged = git(
            &workspace.worktree,
            ["diff", "--cached", "--name-only", "--diff-filter=U"],
        )?;
        if !unmerged.trim().is_empty() {
            return Err(SoftwareDevelopmentError::InvalidInput(format!(
                "integration has unresolved conflicts: {}",
                unmerged.split_whitespace().collect::<Vec<_>>().join(", ")
            )));
        }
        git(
            &workspace.worktree,
            ["commit", "--no-gpg-sign", "-m", "Merge change"],
        )?;
        let merge_commit = git(&workspace.worktree, ["rev-parse", "HEAD"])?
            .trim()
            .to_owned();
        let validation = run_validation(&workspace.worktree, validation_command)?;
        attempted_validation = Some(validation.clone());
        if validation.exit_code != Some(0) {
            return Err(SoftwareDevelopmentError::InvalidInput(
                "integration validation failed; worktree preserved".into(),
            ));
        }
        ensure_clean(&target_checkout)?;
        let actual = git(&target_checkout, ["rev-parse", "HEAD"])?
            .trim()
            .to_owned();
        if actual != target_commit {
            return Err(SoftwareDevelopmentError::TargetMoved {
                expected: target_commit.clone(),
                actual,
            });
        }
        Ok(RunResult {
            outcome: "success".into(),
            branch: Some(target_branch.clone()),
            base_commit: workspace.base_commit.clone(),
            worktree: None,
            commits: vec![merge_commit],
            validation: vec![validation],
            summary: "validated explicit merge commit integrated".into(),
            base_divergence: None,
        })
    })();
    let result = match attempted {
        Ok(result) => result,
        Err(error) => {
            let failed = RunResult {
                outcome: "fail".into(),
                branch: Some(workspace.branch.clone()),
                base_commit: workspace.base_commit.clone(),
                worktree: Some(workspace.worktree.clone()),
                commits: Vec::new(),
                validation: attempted_validation.into_iter().collect(),
                summary: error.to_string(),
                base_divergence: None,
            };
            record_result(&workspace.run_dir, &failed)?;
            return Err(error);
        }
    };
    record_result(&workspace.run_dir, &result)?;
    write_json_atomic(
        &workspace.run_dir.join("integration.json"),
        &serde_json::json!({"integration_branch": workspace.branch, "source_ref": source_ref}),
    )?;
    if let Err(error) = git(&target_checkout, ["merge", "--ff-only", &result.commits[0]]) {
        let mut failed = result.clone();
        failed.outcome = "fail".into();
        failed.worktree = Some(workspace.worktree.clone());
        failed.summary = error.to_string();
        record_result(&workspace.run_dir, &failed)?;
        return Err(error);
    }
    if let Err(error) = cleanup_success(&target_checkout, &workspace)
        .and_then(|_| remove_source_worktree(&target_checkout, &source_ref))
    {
        let mut failed = result.clone();
        failed.outcome = "fail".into();
        failed.worktree = Some(workspace.worktree.clone());
        failed.summary = error.to_string();
        record_result(&workspace.run_dir, &failed)?;
        return Err(error);
    }
    Ok(result)
}

fn remove_source_worktree(checkout: &Path, source_ref: &str) -> Result<()> {
    let listing = git(checkout, ["worktree", "list", "--porcelain"])?;
    let wanted = format!("refs/heads/{source_ref}");
    for block in listing.split("\n\n") {
        let mut path = None;
        let mut branch = None;
        for line in block.lines() {
            if let Some(value) = line.strip_prefix("worktree ") {
                path = Some(value);
            }
            if let Some(value) = line.strip_prefix("branch ") {
                branch = Some(value);
            }
        }
        if branch == Some(wanted.as_str()) {
            if let Some(path) = path {
                if Path::new(path) != checkout {
                    git(checkout, ["worktree", "remove", "--force", path])?;
                }
            }
        }
    }
    Ok(())
}

fn set_tree_read_only(root: &Path) -> Result<()> {
    set_tree_permissions(root, true)
}

fn set_tree_writable(root: &Path) -> Result<()> {
    set_tree_permissions(root, false)
}

fn set_tree_permissions(root: &Path, read_only: bool) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path)?;
        // chmod follows symlinks on supported platforms. Never allow a link in
        // a selected revision to alter permissions outside the managed checkout.
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&path)? {
                pending.push(entry?.path());
            }
        }
        let mut permissions = metadata.permissions();
        permissions.set_readonly(read_only);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

fn run_registry(identity: &RepositoryIdentity) -> PathBuf {
    canonical_repository_root(identity).join(".spark/software-development/runs")
}

fn canonical_repository_root(identity: &RepositoryIdentity) -> PathBuf {
    identity
        .common_dir
        .parent()
        .expect("a Git common directory has a parent")
        .to_path_buf()
}

fn ensure_clean(checkout: &Path) -> Result<()> {
    let status = git(
        checkout,
        ["status", "--porcelain", "--untracked-files=normal"],
    )?;
    if status.lines().all(|line| {
        line.get(3..).is_some_and(|path| {
            path == ".spark/" || path.starts_with(".spark/software-development/runs/")
        })
    }) {
        Ok(())
    } else {
        Err(SoftwareDevelopmentError::DirtyTarget)
    }
}

fn validate_relative_path(path: PathBuf) -> Result<PathBuf> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        Err(SoftwareDevelopmentError::InvalidInput(
            "paths must be non-empty repository-relative paths without '..'".into(),
        ))
    } else {
        Ok(path)
    }
}

fn validate_identifier(name: &str, value: &str) -> Result<()> {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        Ok(())
    } else {
        Err(SoftwareDevelopmentError::InvalidInput(format!(
            "invalid {name}"
        )))
    }
}

fn git<const N: usize>(cwd: &Path, args: [&str; N]) -> Result<String> {
    let operation = args.join(" ");
    let output = Command::new("git").args(args).current_dir(cwd).output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(SoftwareDevelopmentError::Git {
            operation,
            detail: String::from_utf8_lossy(&output.stderr).trim().into(),
        })
    }
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| SoftwareDevelopmentError::InvalidInput("Git paths must be UTF-8".into()))
}

fn evidence(command: &str, output: Output) -> ValidationEvidence {
    ValidationEvidence {
        command: command.into(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    fs::create_dir_all(path.parent().ok_or_else(|| {
        SoftwareDevelopmentError::InvalidInput("state path has no parent".into())
    })?)?;
    let temporary = path.with_extension("json.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temporary)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    Ok(())
}
