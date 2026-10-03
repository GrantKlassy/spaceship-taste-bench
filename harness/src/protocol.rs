use crate::{
    agents::{Agent, Outcome},
    archive,
    config::{ExecutionMode, Limits, parse_component},
};
use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

pub const SCHEMA: u32 = 1;
pub const PROTOCOL: &str = "single-attempt-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskContract {
    pub schema_version: u32,
    pub name: String,
    pub environment: String,
    pub launch: Vec<String>,
    pub play_launch: Vec<String>,
    pub workspace: String,
    pub columns: u16,
    pub rows: u16,
    pub term: String,
    pub locale: String,
    pub dependencies: String,
}
impl TaskContract {
    fn parse(bytes: &[u8], name: &str) -> Result<Self> {
        let contract: Self = toml::from_str(std::str::from_utf8(bytes)?)?;
        ensure!(
            contract.schema_version == SCHEMA && contract.name == name,
            "task name/schema mismatch"
        );
        ensure!(
            crate::config::supported_environment(&contract.environment)
                && contract.workspace == "/workspace",
            "unsupported task environment"
        );
        ensure!(
            contract.launch == ["cargo", "run", "--release", "--locked"]
                && contract.play_launch == ["cargo", "run", "--release", "--frozen"],
            "unsupported launch contract"
        );
        ensure!(
            contract.columns > 0
                && contract.rows > 0
                && contract.term == "xterm-256color"
                && contract.locale == "C.UTF-8"
                && contract.dependencies == "crates.io-only",
            "unsupported terminal/dependency contract"
        );
        Ok(contract)
    }
}
#[derive(Debug, Clone)]
pub struct Task {
    pub contract: TaskContract,
    pub contract_bytes: Vec<u8>,
    pub prompt: Vec<u8>,
    pub prompt_sha256: String,
    pub contract_sha256: String,
}
impl Task {
    pub fn load(repo: &Path, name: &str) -> Result<Self> {
        parse_component(name).map_err(anyhow::Error::msg)?;
        let path = repo.join("tasks").join(name);
        archive::ensure_directory(&path)?;
        let contract_bytes = archive::read_regular(&path.join("task.toml"), 64 * 1024)?;
        let contract = TaskContract::parse(&contract_bytes, name)?;
        let prompt = archive::read_regular(&path.join("prompt.md"), 1024 * 1024)?;
        ensure!(
            !prompt.is_empty() && std::str::from_utf8(&prompt).is_ok(),
            "prompt must be nonempty UTF-8"
        );
        Ok(Self {
            prompt_sha256: archive::hash(&prompt),
            contract_sha256: archive::hash(&contract_bytes),
            contract,
            contract_bytes,
            prompt,
        })
    }
}

pub fn run_id(agent: Agent, model: &str, now: DateTime<Utc>) -> String {
    let mut slug = String::new();
    for c in model.chars() {
        if slug.len() == 64 {
            break;
        }
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    format!(
        "{}-{}-{}",
        agent,
        slug.trim_end_matches('-'),
        now.format("%Y-%m-%d")
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessIdentity {
    pub commit: Option<String>,
    pub dirty: Option<bool>,
}
pub fn harness_identity(repo: &Path) -> HarnessIdentity {
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(repo)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    HarnessIdentity {
        commit: git(&["rev-parse", "HEAD"]),
        dirty: git(&["status", "--porcelain", "--untracked-files=normal"]).map(|s| !s.is_empty()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentity {
    pub name: Agent,
    pub cli_version: Option<String>,
    pub requested_model: String,
    pub reported_model: Option<String>,
    pub invocation: Vec<String>,
    pub settings: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentIdentity {
    pub backend: String,
    pub backend_version: Option<String>,
    pub environment: String,
    pub image_digest: Option<String>,
    pub rust: Option<String>,
    pub architecture: Option<String>,
    pub effective_limits: Option<Limits>,
    pub network_policy: Option<Value>,
    pub isolation_verified: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStatus {
    Pending,
    Complete,
    Failed,
    Unavailable,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Export {
    pub status: ArtifactStatus,
    pub reason: Option<String>,
    pub files: BTreeMap<String, archive::FileRecord>,
    pub tree_sha256: Option<String>,
}
impl Default for Export {
    fn default() -> Self {
        Self {
            status: ArtifactStatus::Pending,
            reason: None,
            files: BTreeMap::new(),
            tree_sha256: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub schema_version: u32,
    pub protocol_version: String,
    #[serde(default)]
    pub mode: ExecutionMode,
    #[serde(default)]
    pub accepted_limitations: Vec<String>,
    pub run_id: String,
    pub task_name: String,
    pub prompt_sha256: String,
    pub task_sha256: String,
    pub input_sha256: BTreeMap<String, String>,
    pub harness: HarnessIdentity,
    pub agent: AgentIdentity,
    pub environment: EnvironmentIdentity,
    pub requested_limits: Limits,
    pub allocated_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub elapsed_seconds: Option<f64>,
    pub outcome: Outcome,
    pub export: Export,
    pub cleanup: ArtifactStatus,
    pub replay_preparation: ArtifactStatus,
}

pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("metadata parent missing")?;
    archive::ensure_directory(parent)?;
    if path.exists() {
        ensure!(
            fs::symlink_metadata(path)?.file_type().is_file(),
            "metadata is not a regular file"
        );
    }
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut tmp, value)?;
    tmp.write_all(b"\n")?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub struct RunStore {
    pub root: PathBuf,
    _lock: File,
}
impl RunStore {
    pub fn open(repo: &Path) -> Result<Self> {
        let root = repo.join("runs");
        fs::create_dir_all(&root)?;
        archive::ensure_directory(&root)?;
        let lock_path = root.join(".allocation.lock");
        if lock_path.exists() {
            ensure!(
                fs::symlink_metadata(&lock_path)?.is_file(),
                "invalid allocation lock"
            );
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.lock_exclusive()?;
        Ok(Self { root, _lock: lock })
    }
    pub fn verify_archives(&self) -> Result<()> {
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            ensure!(
                entry.file_type()?.is_dir(),
                "unexpected entry in runs; inspect it before allocating"
            );
            let id = entry.file_name();
            load_run_metadata(
                self.root.parent().context("runs parent missing")?,
                id.to_str().context("invalid run directory name")?,
            )
            .context("invalid archived inputs; inspect the archive before allocating")?;
        }
        Ok(())
    }
    pub fn allocate(
        &self,
        run: &Run,
        task: &Task,
        review: &[u8],
        inputs: &BTreeMap<String, Vec<u8>>,
    ) -> Result<PathBuf> {
        self.verify_archives()?;
        parse_component(&run.run_id).map_err(anyhow::Error::msg)?;
        let final_dir = self.root.join(&run.run_id);
        ensure!(
            !final_dir.exists(),
            "run {} already exists; only one run per agent/model and UTC date is allowed",
            run.run_id
        );
        let stage = tempfile::Builder::new()
            .prefix(".alloc-")
            .tempdir_in(&self.root)?;
        fs::write(stage.path().join("prompt.md"), &task.prompt)?;
        fs::write(stage.path().join("task.toml"), &task.contract_bytes)?;
        fs::write(stage.path().join("review.md"), review)?;
        for (name, bytes) in inputs {
            ensure!(
                ["environment.lock.json", "settings.json"].contains(&name.as_str()),
                "invalid input snapshot name"
            );
            ensure!(
                run.input_sha256.get(name) == Some(&archive::hash(bytes)),
                "input snapshot checksum mismatch"
            );
            fs::write(stage.path().join(name), bytes)?;
        }
        fs::create_dir(stage.path().join("media"))?;
        atomic_json(&stage.path().join("run.json"), run)?;
        fs::rename(stage.path(), &final_dir)?;
        Ok(final_dir)
    }
}

pub fn archived_task_contract(run_dir: &Path, run: &Run) -> Result<TaskContract> {
    let bytes = archive::read_regular(&run_dir.join("task.toml"), 64 * 1024)?;
    ensure!(
        archive::hash(&bytes) == run.task_sha256,
        "task checksum mismatch"
    );
    TaskContract::parse(&bytes, &run.task_name)
}

pub fn load_run_metadata(repo: &Path, id: &str) -> Result<(PathBuf, Run)> {
    parse_component(id).map_err(anyhow::Error::msg)?;
    let path = repo.join("runs").join(id);
    archive::ensure_directory(&path)?;
    let run: Run = serde_json::from_slice(&archive::read_regular(
        &path.join("run.json"),
        8 * 1024 * 1024,
    )?)?;
    run.requested_limits.validate()?;
    ensure!(
        run.schema_version == SCHEMA
            && run.protocol_version == run.mode.protocol()
            && run.accepted_limitations == run.mode.limitations(run.agent.name)
            && run.run_id == id,
        "run identity/schema mismatch"
    );
    ensure!(
        archive::hash(&archive::read_regular(
            &path.join("prompt.md"),
            1024 * 1024
        )?) == run.prompt_sha256,
        "prompt checksum mismatch"
    );
    archived_task_contract(&path, &run)?;
    for (name, expected) in &run.input_sha256 {
        ensure!(
            ["environment.lock.json", "settings.json"].contains(&name.as_str()),
            "invalid input snapshot reference"
        );
        ensure!(
            archive::hash(&archive::read_regular(&path.join(name), 1024 * 1024)?) == *expected,
            "input snapshot checksum mismatch"
        );
    }
    Ok((path, run))
}

pub fn load_run(repo: &Path, id: &str) -> Result<(PathBuf, Run)> {
    let (path, run) = load_run_metadata(repo, id)?;
    if run.export.status != ArtifactStatus::Complete {
        bail!("this attempt has no complete source export");
    }
    let actual = archive::inventory(
        &path.join("solution"),
        run.requested_limits.export_bytes,
        run.requested_limits.export_files,
    )?;
    ensure!(
        actual == run.export.files && Some(archive::tree_hash(&actual)?) == run.export.tree_sha256,
        "immutable source checksum mismatch"
    );
    Ok((path, run))
}
