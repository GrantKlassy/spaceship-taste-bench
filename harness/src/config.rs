use crate::agents::Agent;
use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Parser)]
#[command(
    name = "bench",
    version,
    about = "Preserve one autonomous coding-agent attempt"
)]
pub struct Cli {
    #[arg(long, global = true, default_value = ".")]
    pub repo: PathBuf,
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    /// Select strict certification or the explicitly labeled Docker sandbox pilot.
    #[arg(long, global = true, value_enum)]
    pub mode: Option<ExecutionMode>,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Inspect prerequisites without making a model request.
    Doctor {
        /// Report account prerequisites only for the agent you intend to run.
        #[arg(long)]
        agent: Option<crate::agents::Agent>,
        /// Emit structured, non-secret readiness checks.
        #[arg(long)]
        json: bool,
    },
    /// Authenticate explicitly; never changes to API billing automatically.
    Auth { agent: Agent },
    /// Deliver the task exactly once in a fresh external sandbox.
    Run {
        #[arg(long)]
        agent: Agent,
        #[arg(long, value_parser = validate_model)]
        model: String,
        #[arg(long, default_value = "spaceship-v1", value_parser = parse_component)]
        task: String,
    },
    /// Build and play an immutable submission in an offline guest.
    Play {
        #[arg(value_parser = parse_component)]
        run_id: String,
    },
    /// Recover a failed export from its retained stopped snapshot; no model request.
    Recover {
        #[arg(value_parser = parse_component)]
        run_id: String,
    },
    /// Run non-billable isolation checks with controlled fixtures (opt in).
    CheckIntegration,
    /// Probe pinned built-in agent runtimes without login, prompt, or model request.
    CheckRuntimes {
        #[arg(long)]
        agent: Option<crate::agents::Agent>,
    },
}

pub fn parse_component(s: &str) -> Result<String, String> {
    if s.is_empty()
        || s.len() > 180
        || !s.as_bytes()[0].is_ascii_alphanumeric()
        || !s
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err("expected a single ASCII identifier (letters, numbers, '-' or '_')".into());
    }
    Ok(s.into())
}

pub fn validate_model(s: &str) -> Result<String, String> {
    if s.trim().is_empty()
        || s.len() > 200
        || !s.as_bytes()[0].is_ascii_alphanumeric()
        || !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-._/:".contains(c))
    {
        return Err(
            "model must be an explicit identifier, without whitespace, controls, or shell syntax"
                .into(),
        );
    }
    Ok(s.into())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionMode {
    #[default]
    Strict,
    DockerPilot,
}
impl ExecutionMode {
    pub fn is_pilot(self) -> bool {
        self == Self::DockerPilot
    }
    pub fn protocol(self) -> &'static str {
        match self {
            Self::Strict => "single-attempt-v1",
            Self::DockerPilot => "single-attempt-docker-pilot-v1",
        }
    }
    pub fn limitations(self) -> Vec<String> {
        if !self.is_pilot() {
            return Vec::new();
        }
        [
            "Docker-managed MCP gateway and provider/integration bindings remain accessible, including during replay.",
            "Docker-generated agent configuration is accepted; full configuration neutrality is not certified.",
            "Generation uses Docker's Codex network defaults, including OpenAI/code/package hosts, plus crates.io.",
            "Subscription refresh and exhaustive network isolation are not certified; the pilot checks OAuth mode and its configured network rules.",
            "Guest disks and archive parsing are bounded; host-side snapshot/cache growth has no enforced quota.",
        ].into_iter().map(String::from).collect()
    }
    pub fn validate_agent(self, agent: Agent) -> Result<()> {
        ensure!(
            !self.is_pilot() || agent == Agent::Codex,
            "Docker pilot generation currently supports Codex only"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub cpus: u32,
    pub memory_mib: u64,
    pub disk_mib: u64,
    pub export_bytes: u64,
    pub export_files: usize,
    pub replay_bytes: u64,
    pub replay_files: usize,
    pub log_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            cpus: 4,
            memory_mib: 8192,
            disk_mib: 20480,
            // A self-contained terminal project can vendor Windows support
            // crates too, even when the intended runtime is Linux.
            export_bytes: 512 * 1024 * 1024,
            export_files: 10000,
            replay_bytes: 1024 * 1024 * 1024,
            replay_files: 100000,
            log_bytes: 256 * 1024 * 1024,
        }
    }
}
impl Limits {
    pub fn validate(&self) -> Result<()> {
        ensure!((1..=64).contains(&self.cpus), "cpus must be 1..64");
        ensure!(
            (512..=262144).contains(&self.memory_mib),
            "memory_mib must be 512..262144"
        );
        ensure!(
            (2048..=1048576).contains(&self.disk_mib),
            "disk_mib must be 2048..1048576"
        );
        ensure!(
            self.export_bytes > 0 && self.export_bytes <= self.replay_bytes,
            "invalid export/replay size limits"
        );
        ensure!(
            self.replay_bytes <= self.disk_mib * 1024 * 1024,
            "replay limit exceeds disk limit"
        );
        ensure!(
            self.export_files > 0
                && self.export_files <= self.replay_files
                && self.replay_files <= 1_000_000,
            "invalid file count limits"
        );
        ensure!(
            (1024..=4 * 1024 * 1024 * 1024).contains(&self.log_bytes),
            "invalid log size limit"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub backend: String,
    pub sbx: PathBuf,
    pub environment: String,
    pub mode: ExecutionMode,
    pub limits: Limits,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: 1,
            backend: "sbx".into(),
            sbx: "sbx".into(),
            environment: "linux-rust-v2".into(),
            mode: ExecutionMode::Strict,
            limits: Limits::default(),
        }
    }
}
impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let config: Self = match path {
            Some(p) => toml::from_str(&fs::read_to_string(p).context("read configuration")?)
                .context("parse configuration")?,
            None => Self::default(),
        };
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1 && self.backend == "sbx",
            "only schema 1 and local sbx are supported; no host/container fallback"
        );
        ensure!(
            supported_environment(&self.environment),
            "unsupported environment version"
        );
        ensure!(
            !self.sbx.to_string_lossy().ends_with(".exe"),
            "Windows sbx.exe transport is unverified; use a Linux-local backend (see SECURITY.md)"
        );
        ensure!(!self.sbx.as_os_str().is_empty(), "sbx executable is empty");
        self.limits.validate()
    }
}

pub fn state_dir(repo: &Path) -> Result<PathBuf> {
    let path = if let Some(p) = std::env::var_os("BENCH_STATE_DIR") {
        PathBuf::from(p)
    } else if let Some(p) = std::env::var_os("XDG_STATE_HOME") {
        PathBuf::from(p).join("spaceship-taste-bench")
    } else {
        PathBuf::from(
            std::env::var_os("HOME")
                .context("set BENCH_STATE_DIR to a private directory outside the checkout")?,
        )
        .join(".local/state/spaceship-taste-bench")
    };
    ensure!(path.is_absolute(), "state directory must be absolute");
    // Check the nearest existing ancestor before creating anything.
    let ancestor = path
        .ancestors()
        .find(|p| p.exists())
        .context("state parent missing")?
        .canonicalize()?;
    ensure!(
        !ancestor.starts_with(repo),
        "local state must be outside the Git checkout"
    );
    if path.exists() && !path.join(".bench-state-v1").exists() {
        ensure!(
            fs::read_dir(&path)?.next().is_none(),
            "BENCH_STATE_DIR must be a dedicated empty directory, not an existing directory containing unrelated files"
        );
    }
    fs::create_dir_all(&path)?;
    let path = path.canonicalize()?;
    ensure!(
        !path.starts_with(repo) && !repo.starts_with(&path),
        "state and repository must be disjoint"
    );
    crate::archive::private_dir(&path)?;
    let marker = path.join(".bench-state-v1");
    if marker.exists() {
        ensure!(
            crate::archive::read_regular(&marker, 64)? == b"spaceship-taste-bench state v1\n",
            "invalid state directory marker"
        );
    } else {
        fs::write(marker, b"spaceship-taste-bench state v1\n")?;
    }
    Ok(path)
}

/// Explicitly reviewed environment versions; arbitrary directories are rejected.
pub fn supported_environment(value: &str) -> bool {
    matches!(value, "linux-rust-v1" | "linux-rust-v2")
}
