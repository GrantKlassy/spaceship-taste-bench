//! One backend, using the documented local `sbx` CLI. No production mock or host fallback.
use crate::{
    agents::Agent,
    archive,
    config::{Config, parse_component, state_dir},
    inspection::{
        CodexBroker, GuestProbe, Inspect, validate_allowlist_policy, validate_denial,
        validate_offline_policy, validate_offline_policy_with_masked_allows,
        validate_protocol_denial,
    },
    process,
    protocol::EnvironmentIdentity,
    readiness::{Report, Status},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub const BACKEND_VERSION: &str = "0.45.0";
const CRATES_DESTINATIONS: &[&str] = &["index.crates.io:443", "static.crates.io:443"];
// Observed immutable built-in Codex allowances in sbx 0.45.0, plus crates.io.
// Pilot generation uses Docker's existing defaults; packaging/playback are narrower.
const CODEX_DESTINATIONS: &[&str] = &[
    "api.openai.com:443",
    "openai.com:443",
    "auth.openai.com:443",
    "chatgpt.com:443",
    "files.openai.com:443",
    "registry.npmjs.org:443",
    "releases.openai.com:443",
    "api.github.com:443",
    "github.com:443",
    "release-assets.githubusercontent.com:443",
    "codeload.github.com:443",
    "archive.ubuntu.com:80",
    "security.ubuntu.com:80",
    "ports.ubuntu.com:80",
    "download.docker.com:443",
    "index.crates.io:443",
    "static.crates.io:443",
];
// sbx 0.45's built-in Claude runtime attaches these fresh ext4 devices in
// addition to the root disk. Charge their full nominal capacities to the
// configured disk budget, including filesystem overhead.
pub(crate) const CLAUDE_VOLUMES: [(&str, u64); 5] = [
    ("/home/agent/.claude/projects", 2048),
    ("/home/agent/.claude/sessions", 512),
    ("/home/agent/.claude/todos", 512),
    ("/home/agent/.claude/shell-snapshots", 512),
    ("/home/agent/.claude/statsig", 512),
];
/// These are release blockers, not configurable assertions of trust. Evidence must
/// come from supported backend inspection and live tests before removing a blocker.
pub const BLOCKERS: &[&str] = &[
    "sbx 0.45 automatically exposes an MCP gateway even in a mountless shell guest with deny-all networking. A reachable gateway was observed; no supported per-guest disable mechanism has been verified.",
    "Unrelated provider credential bindings are injected into every guest. Selected-provider scope and credential-free playback are not established.",
    "Subscription egress destinations and sentinel-token refresh across fresh guests have not been observed for either provider. No guessed allowlist is enabled.",
    "The guest root disk limit is observed, but the backend's host-side snapshot/cache growth still lacks a verified bound during export.",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Generation(Agent),
    Package,
    Playback,
    Diagnostic,
}
impl Role {
    pub(crate) fn root_disk_mib(self, total_mib: u64) -> Result<u64> {
        let reserved: u64 = if self == Self::Generation(Agent::Claude) {
            CLAUDE_VOLUMES.iter().map(|(_, mib)| mib).sum()
        } else {
            0
        };
        let root = total_mib.saturating_sub(reserved);
        ensure!(
            root >= 2048,
            "{} requires disk_mib >= {} to cover its root disk and runtime volumes",
            self.agent(),
            reserved + 2048
        );
        Ok(root)
    }
    pub fn image_key(self) -> &'static str {
        match self {
            Self::Generation(Agent::Claude) => "claude",
            Self::Generation(Agent::Codex) => "codex",
            _ => "base",
        }
    }
    pub(crate) fn agent(self) -> &'static str {
        match self {
            Self::Generation(Agent::Claude) => "claude",
            Self::Generation(Agent::Codex) => "codex",
            _ => "shell",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    pub reference: String,
    pub image_id: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct EnvironmentLock {
    pub schema_version: u32,
    pub environment: String,
    pub rust_version: String,
    pub images: std::collections::BTreeMap<String, Option<Image>>,
}

#[derive(Deserialize)]
struct Templates {
    images: Vec<Template>,
}
#[derive(Deserialize)]
struct Template {
    id: String,
    repository: String,
    tag: String,
}
impl Templates {
    fn contains(&self, image: &Image) -> Result<()> {
        ensure!(
            self.images
                .iter()
                .filter(|candidate| {
                    let reference = format!(
                        "{}:{}",
                        candidate
                            .repository
                            .trim_start_matches("docker.io/library/"),
                        candidate.tag
                    );
                    let id = candidate.id.trim_start_matches("sha256:");
                    reference == image.reference
                        && (12..=64).contains(&id.len())
                        && id.bytes().all(|b| b.is_ascii_hexdigit())
                        && image.image_id.trim_start_matches("sha256:").starts_with(id)
                })
                .count()
                == 1,
            "resolved image absent or ambiguous in sandbox store"
        );
        Ok(())
    }
}

fn disabled_setting(bytes: &[u8], key: &str) -> Result<()> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    ensure!(
        value["key"] == key && value["type"] == "bool" && value["value"] == false,
        "host integration is enabled or its setting is unknown"
    );
    Ok(())
}

const DIAGNOSTIC_CHECKS: &[&str] = &[
    "CLI binary",
    "Binary version",
    "Daemon",
    "Daemon diagnostics",
    "Virtualization",
    "mkfs.erofs",
    "Storage directories",
    "Directory permissions",
    "Disk space",
    "Version match",
    "Socket",
    "SSH client config",
    "Authentication",
];

#[derive(Deserialize)]
struct Diagnostics {
    checks: Vec<DiagnosticCheck>,
}
#[derive(Deserialize)]
struct DiagnosticCheck {
    name: String,
    status: DiagnosticStatus,
    // Intentionally discard messages, details, identities and upload suggestions.
}
#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum DiagnosticStatus {
    Pass,
    Warn,
    Fail,
    Skip,
}

fn diagnostic_summary(exit: Option<i32>, data: &[u8]) -> Result<Vec<String>> {
    let report: Diagnostics = serde_json::from_slice(data)?;
    ensure!(
        report.checks.len() == DIAGNOSTIC_CHECKS.len(),
        "unexpected diagnostic checks"
    );
    let mut lines = Vec::new();
    for &name in DIAGNOSTIC_CHECKS {
        let matches: Vec<_> = report.checks.iter().filter(|c| c.name == name).collect();
        ensure!(matches.len() == 1, "missing or duplicate diagnostic check");
        let status = &matches[0].status;
        if *status == DiagnosticStatus::Pass {
            continue;
        }
        let label = match status {
            DiagnosticStatus::Fail => "MISSING/FAILED",
            DiagnosticStatus::Warn => "WARNING",
            DiagnosticStatus::Skip => "SKIPPED",
            DiagnosticStatus::Pass => unreachable!(),
        };
        let fix = match name {
            "Daemon" => " Run: sbx daemon start --detach.",
            "Authentication" => " Sign in yourself with: sbx login.",
            _ => " Inspect sbx diagnose locally; review its output before sharing.",
        };
        lines.push(format!("{label}: backend {name}.{fix}"));
    }
    if lines.is_empty() {
        ensure!(
            exit == Some(0),
            "diagnostic status disagrees with exit code"
        );
        lines.push("Backend diagnostics: all 13 checks passed, including Docker sign-in (not agent-provider authentication or isolation certification).".into());
    }
    Ok(lines)
}

/// Controller-facing operations. Only tests implement a simulated backend.
pub trait Sandbox {
    fn preflight(&self, role: Role) -> Result<()>;
    fn create(&self, name: &str, role: Role) -> Result<()>;
    fn verify(&self, name: &str, role: Role) -> Result<EnvironmentIdentity>;
    fn exec(&self, name: &str, args: &[String], interactive: bool) -> Result<Command>;
    fn import(&self, name: &str, archive: &Path, destination: &str) -> Result<()>;
    fn stop(&self, name: &str) -> Result<()>;
    fn export(
        &self,
        name: &str,
        prefix: &str,
        destination: &Path,
        raw: &Path,
        bytes: u64,
        files: usize,
    ) -> Result<()>;
    fn destroy(&self, name: &str) -> Result<()>;
}

pub struct Sbx {
    config: Config,
    lock: EnvironmentLock,
    diagnostics: PathBuf,
    abort: Arc<AtomicBool>,
}
impl Sbx {
    pub fn new(repo: &Path, config: &Config) -> Result<Self> {
        config.validate()?;
        let lock: EnvironmentLock = serde_json::from_slice(&archive::read_regular(
            &repo
                .join("environments")
                .join(&config.environment)
                .join("environment.lock.json"),
            1024 * 1024,
        )?)?;
        ensure!(
            lock.schema_version == 1
                && lock.environment == config.environment
                && lock.rust_version == "1.97.0",
            "environment lock mismatch"
        );
        Ok(Self {
            config: config.clone(),
            lock,
            diagnostics: state_dir(repo)?
                .join("backend")
                .join(uuid::Uuid::new_v4().simple().to_string()),
            abort: Arc::new(AtomicBool::new(false)),
        })
    }
    pub fn with_abort(mut self, abort: Arc<AtomicBool>) -> Self {
        self.abort = abort;
        self
    }
    fn control(&self, command: Command) -> Result<Vec<u8>> {
        process::control_logged_abort(command, &self.diagnostics, &self.abort)
    }
    fn control_output(&self, command: Command) -> Result<(Option<i32>, Vec<u8>)> {
        process::control_output_logged_abort(command, &self.diagnostics, &self.abort)
    }
    fn cleanup_control(&self, command: Command) -> Result<Vec<u8>> {
        // Cancellation must never suppress destruction, stop, or template cleanup.
        process::control_logged(command, &self.diagnostics)
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = process::host_command(&self.config.sbx);
        cmd.args(args);
        cmd
    }
    pub fn version(&self) -> Result<String> {
        let output = self.control(self.command(&["version"]))?;
        let text = std::str::from_utf8(&output)?.trim();
        ensure!(
            text.starts_with("sbx version: v0.45.0 "),
            "expected local sbx 0.45.0; re-verify changed CLI versions before use"
        );
        Ok(format!("sbx {BACKEND_VERSION}"))
    }
    pub fn doctor(&self, selected: Option<Agent>) -> Report {
        let selected = selected.or_else(|| self.config.mode.is_pilot().then_some(Agent::Codex));
        let mut report = Report::default();
        report.push("execution_mode", Status::Pass, self.config.mode.protocol());
        let cli_available = self.version().is_ok();
        report.push(
            "backend_version",
            if cli_available {
                Status::Pass
            } else {
                Status::Blocked
            },
            if cli_available {
                "Local sbx 0.45.0 recognized."
            } else {
                "Install the pinned Linux-local sbx 0.45.0; see docs/SETUP.md."
            },
        );
        match crate::terminal::supported_host() {
            Ok(()) => report.push(
                "host",
                Status::Pass,
                "Supported Ubuntu amd64 host with read/write KVM access.",
            ),
            Err(error) => report.push("host", Status::Blocked, error.to_string()),
        }
        if cli_available {
            match self
                .control_output(self.command(&["diagnose", "--json"]))
                .and_then(|(exit, data)| diagnostic_summary(exit, &data))
            {
                Ok(lines) => {
                    for line in lines {
                        let status = if line.starts_with("Backend diagnostics:") {
                            Status::Pass
                        } else if line.starts_with("WARNING:") {
                            Status::Warning
                        } else {
                            Status::Blocked
                        };
                        report.push("backend_diagnostics", status, line);
                    }
                }
                Err(_) => report.push(
                    "backend_diagnostics",
                    Status::Blocked,
                    "Backend diagnostics unavailable; inspect sbx diagnose locally.",
                ),
            }
            match self.control(self.command(&["policy", "ls", "--json"])) {
                Ok(_) => report.push("network_policy", Status::Pass, "Policy query available; per-guest effective rules are checked separately."),
                Err(_) => report.push("network_policy", Status::Blocked, "Network policy unavailable. For a NEW installation only: sbx policy init deny-all."),
            }
            for (id, key) in [
                ("ssh_forwarding", "ssh.agentForwardingEnabled"),
                ("clipboard", "clipboard.imagePaste"),
                ("claude_remote_control", "claude.remoteControl"),
            ] {
                match self.control(self.command(&["settings", "get", "--json", key]))
                    .and_then(|bytes| disabled_setting(&bytes, key)) {
                    Ok(()) => report.push(id, Status::Pass, format!("{key} is disabled; fresh-guest probes check effective isolation.")),
                    Err(_) => report.push(id, Status::Blocked, format!("Require {key}=false. Use sbx settings set {key} false; restart the daemon if requested.")),
                }
            }
        }
        let available = if cli_available {
            self.control(self.command(&["template", "ls", "--json"]))
                .and_then(|bytes| Ok(serde_json::from_slice::<Templates>(&bytes)?))
                .ok()
        } else {
            None
        };
        for role in [
            Role::Playback,
            Role::Generation(Agent::Claude),
            Role::Generation(Agent::Codex),
        ] {
            let id = format!("image_{}", role.image_key());
            let result = self.image(role).and_then(|image| {
                available
                    .as_ref()
                    .context("image store unavailable")?
                    .contains(image)
            });
            match result {
                Ok(()) => report.push(&id, Status::Pass, "Resolved image is present in the local sandbox store."),
                Err(_) => report.push(&id, Status::Blocked, "Image missing or mismatched. Load the preserved environment archive; do not rebuild this frozen environment."),
            }
        }
        for agent in [Agent::Claude, Agent::Codex] {
            if selected.is_some_and(|selected| selected != agent) {
                continue;
            }
            if let Err(error) = self.config.mode.validate_agent(agent) {
                report.push("agent", Status::Blocked, error.to_string());
                continue;
            }
            match Role::Generation(agent).root_disk_mib(self.config.limits.disk_mib) {
                Ok(root_mib) => report.push(
                    &format!("disk_{agent}"),
                    Status::Pass,
                    format!("{root_mib} MiB root disk plus runtime volumes fit the total disk budget; actual devices are checked in guests."),
                ),
                Err(error) => report.push(
                    &format!("disk_{agent}"), Status::Blocked, error.to_string(),
                ),
            }
            let configured = self.has_credential(agent);
            match configured {
                Ok(true) if self.config.mode.is_pilot() => report.push(&format!("auth_{agent}"), Status::Pass,
                    "Broker entry exists; a fresh guest must also pass the OAuth checks below."),
                Ok(true) => report.push(&format!("auth_{agent}"), Status::Blocked,
                    "A broker credential entry exists; subscription mode, refresh and fresh-guest reuse remain unverified."),
                Ok(false) => report.push(&format!("auth_{agent}"), Status::Blocked,
                    format!("No {agent} broker credential is stored. Run: bench auth {agent}.")),
                Err(_) => report.push(&format!("auth_{agent}"), Status::Blocked, "Credential status unavailable; no authentication was attempted."),
            }
        }
        if self.config.mode.is_pilot() {
            for limitation in self.config.mode.limitations() {
                report.push("pilot_limitation", Status::Warning, limitation);
            }
            if report.ready {
                match self.check_runtimes(selected) {
                    Ok(runtime) => {
                        for check in runtime.checks {
                            report.push(&check.id, check.status, check.detail);
                        }
                    }
                    Err(error) => report.push("runtime", Status::Blocked, error.to_string()),
                }
            }
        } else {
            for (id, detail) in [
                "mcp_isolation",
                "credential_scope",
                "provider_egress",
                "snapshot_storage",
            ]
            .into_iter()
            .zip(BLOCKERS)
            {
                report.push(id, Status::Blocked, *detail);
            }
        }
        report
    }
    fn has_credential(&self, agent: Agent) -> Result<bool> {
        let service = match agent {
            Agent::Codex => "openai",
            Agent::Claude => "anthropic",
        };
        // Names only, never credential values; do not persist authentication output.
        let (exit, data) = process::control_output(self.command(&[
            "secret",
            "ls",
            "--service",
            service,
            "--quiet",
        ]))?;
        ensure!(exit == Some(0), "credential inventory unavailable");
        Ok(data.iter().any(|b| !b.is_ascii_whitespace()))
    }
    /// Only fixed version/help/probe commands are accepted by this diagnostic.
    /// No official preflight bypass and no user-supplied task/source is exposed.
    pub fn check_runtimes(&self, selected: Option<Agent>) -> Result<Report> {
        let selected = selected.or_else(|| self.config.mode.is_pilot().then_some(Agent::Codex));
        let mut report = Report::default();
        for agent in [Agent::Claude, Agent::Codex] {
            if selected.is_some_and(|selected| selected != agent) {
                continue;
            }
            self.config.mode.validate_agent(agent)?;
            let role = Role::Generation(agent);
            self.prerequisites(role)?;
            let result = (|| -> Result<()> {
                let guest = Guest::new(self, role)?;
                self.inspect(&guest.name)?.validate_identity(
                    &guest.name,
                    role,
                    self.image(role)?,
                    &self.config.limits,
                )?;
                self.verify_ports(&guest.name)?;
                let probe = self.probe(&guest.name)?;
                probe.validate_sockets()?;
                match probe.validate_resources(&self.config.limits, role) {
                    Ok(()) => report.push(
                        &format!("{agent}_resources"),
                        Status::Pass,
                        "CPU, memory and all writable disk volumes fit the configured limits.",
                    ),
                    Err(error) => report.push(
                        &format!("{agent}_resources"),
                        Status::Blocked,
                        error.to_string(),
                    ),
                }
                report.push(
                    &format!("{agent}_ssh"),
                    if probe.ssh_socket_absent() {
                        Status::Pass
                    } else {
                        Status::Blocked
                    },
                    if probe.ssh_socket_absent() {
                        "Fresh guest has no SSH-agent socket."
                    } else {
                        "Fresh guest exposes an SSH-agent socket."
                    },
                );
                let neutral = probe.validate_machine(&self.config.limits, role).is_ok();
                report.push(&format!("{agent}_configuration"), if neutral { Status::Pass } else { self.accepted_status() },
                    if neutral { "No pre-existing agent state." } else { "Built-in runtime writes agent configuration (including gateway configuration); effective neutral settings and broker state require verification." });
                let output = self.control(self.exec(
                    &guest.name,
                    &[agent.to_string(), "--version".into()],
                    false,
                )?)?;
                let expected = agent.version_banner();
                ensure!(
                    std::str::from_utf8(&output)?.trim() == expected,
                    "runtime CLI version mismatch"
                );
                // --help validates the installed parser without stdin or a request.
                let mut args = agent.invocation("bench-diagnostic-unused");
                args.push("--help".into());
                self.control(self.exec(&guest.name, &args, false)?)?;
                report.push(&format!("{agent}_runtime"), Status::Pass, "Fresh built-in runtime starts, reports the pinned version and accepts the adapter flags.");
                if agent == Agent::Codex {
                    if self.config.mode.is_pilot() {
                        self.configure_generation(&guest.name)?;
                        report.push("codex_network", Status::Pass, "Docker's pinned Codex network defaults plus crates.io are configured; checked unrelated destinations remain denied. Docker-managed services are outside this guarantee.");
                    }
                    match self.codex_broker(&guest.name)?.validate_subscription() {
                        Ok(()) => report.push("codex_subscription", Status::Pass,
                            "Fresh guest reports broker OAuth mode and contains only the expected auth placeholder. Refresh and provider access still require verification."),
                        Err(error) => report.push("codex_subscription", Status::Blocked, error.to_string()),
                    }
                }
                for result in [
                    self.inspect(&guest.name)?.validate_services(),
                    probe.validate_services(),
                ] {
                    if let Err(error) = result {
                        report.push(
                            &format!("{agent}_services"),
                            self.accepted_status(),
                            error.to_string(),
                        );
                    }
                }
                if agent == Agent::Claude {
                    // A replacement guest must not inherit writable runtime
                    // state, even after the original guest has been deleted.
                    probe.validate_resources(&self.config.limits, role)?;
                    let marker = format!(".bench-{}", uuid::Uuid::new_v4().simple());
                    self.runtime_freshness(&guest.name, &marker, "write")?;
                    guest.destroy()?;
                    let fresh = Guest::new(self, role)?;
                    self.inspect(&fresh.name)?.validate_identity(
                        &fresh.name,
                        role,
                        self.image(role)?,
                        &self.config.limits,
                    )?;
                    self.probe(&fresh.name)?
                        .validate_resources(&self.config.limits, role)?;
                    self.runtime_freshness(&fresh.name, &marker, "check")?;
                    fresh.destroy()?;
                    report.push(
                        "claude_storage_freshness",
                        Status::Pass,
                        "A replacement guest has no marker from any previous runtime volume.",
                    );
                } else {
                    guest.destroy()?;
                }
                Ok(())
            })();
            if let Err(error) = result {
                if self.abort.load(Ordering::SeqCst) {
                    return Err(error);
                }
                report.push(
                    &format!("{agent}_runtime"),
                    Status::Blocked,
                    format!("Runtime check failed: {error}. Inspect private backend logs."),
                );
            }
        }
        Ok(report)
    }
    fn accepted_status(&self) -> Status {
        if self.config.mode.is_pilot() {
            Status::Warning
        } else {
            Status::Blocked
        }
    }
    pub fn auth(&self, agent: Agent) -> Result<()> {
        self.version()?;
        crate::terminal::supported_host()?;
        match agent {
            Agent::Codex => {
                eprintln!(
                    "Opening the backend's supported OpenAI OAuth flow. No benchmark task will start."
                );
                let exit =
                    process::interactive(self.command(&["secret", "set", "openai", "--oauth"]))?;
                ensure!(exit == Some(0), "backend OAuth did not complete");
                eprintln!(
                    "OAuth flow completed. Run: bench doctor --agent codex. Fresh-guest checks verify subscription mode before a task starts."
                );
                Ok(())
            }
            Agent::Claude => anyhow::bail!(
                "Claude subscription auth is documented as /login inside a sandbox, but safe reuse through the broker in a fresh mountless guest is unverified. See docs/SETUP.md. bench will not copy ~/.claude or silently select API billing."
            ),
        }
    }
    fn image(&self, role: Role) -> Result<&Image> {
        let image = self
            .lock
            .images
            .get(role.image_key())
            .and_then(Option::as_ref)
            .context("sandbox image has not been built and resolved; run prepare.py")?;
        ensure!(
            image.image_id.starts_with("sha256:")
                && image.image_id.len() == 71
                && image.image_id[7..].bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid resolved image ID"
        );
        ensure!(
            image.reference.starts_with("spaceship-bench-")
                && !image.reference.chars().any(char::is_whitespace),
            "invalid local image reference"
        );
        Ok(image)
    }
    fn checked(&self, args: &[&str]) -> Result<()> {
        self.control(self.command(args)).map(|_| ())
    }
    fn prerequisites(&self, role: Role) -> Result<()> {
        crate::terminal::supported_host()?;
        self.version()?;
        self.image(role)?;
        role.root_disk_mib(self.config.limits.disk_mib)?;
        Ok(())
    }
    fn codex_broker(&self, name: &str) -> Result<CodexBroker> {
        Ok(serde_json::from_slice(&self.control(self.exec(
            name,
            &[
                "python3".into(),
                "-c".into(),
                include_str!("../probes/codex_broker.py").into(),
            ],
            false,
        )?)?)?)
    }
    fn runtime_freshness(&self, name: &str, marker: &str, mode: &str) -> Result<()> {
        self.control(self.exec(
            name,
            &[
                "python3".into(),
                "-c".into(),
                include_str!("../probes/runtime_freshness.py").into(),
                marker.into(),
                mode.into(),
            ],
            false,
        )?)?;
        Ok(())
    }
    fn inspect(&self, name: &str) -> Result<Inspect> {
        Ok(serde_json::from_slice(
            &self.control(self.command(&["inspect", name, "--json"]))?,
        )?)
    }
    fn probe(&self, name: &str) -> Result<GuestProbe> {
        Ok(serde_json::from_slice(&self.control(self.exec(
            name,
            &[
                "python3".into(),
                "-c".into(),
                include_str!("../probes/inspect_guest.py").into(),
            ],
            false,
        )?)?)?)
    }
    fn verify_ports(&self, name: &str) -> Result<()> {
        let ports: Vec<serde_json::Value> =
            serde_json::from_slice(&self.control(self.command(&["ports", name, "--json"]))?)?;
        ensure!(ports.is_empty(), "unexpected published guest ports");
        Ok(())
    }
    fn verify_offline_policy(&self, name: &str) -> Result<()> {
        let bytes = self.control(self.command(&["policy", "ls", name, "--json"]))?;
        if self.config.mode.is_pilot() {
            validate_offline_policy_with_masked_allows(&bytes, name, CODEX_DESTINATIONS)?;
        } else {
            validate_offline_policy(&bytes, name)?;
        }
        for target in [
            "example.com:443",
            "github.com:443",
            "api.anthropic.com:443",
            "api.openai.com:443",
            "index.crates.io:443",
            "169.254.169.254:80",
            "host.docker.internal:80",
        ] {
            let (exit, data) = self.control_output(self.command(&[
                "policy",
                "check",
                "network",
                "--sandbox",
                name,
                "--json",
                target,
            ]))?;
            validate_denial(exit, &data, name, target)?;
        }
        for (target, protocol) in [
            ("1.1.1.1:443", "tcp"),
            ("[2606:4700:4700::1111]:443", "tcp"),
            ("127.0.0.1:80", "tcp"),
            ("10.0.0.1:80", "tcp"),
            ("192.168.0.1:80", "tcp"),
            ("1.1.1.1:53", "udp"),
            ("[2606:4700:4700::1111]:53", "udp"),
        ] {
            let (exit, data) = self.control_output(self.command(&[
                "policy",
                "check",
                "network",
                "--sandbox",
                name,
                "--json",
                "--protocol",
                protocol,
                target,
            ]))?;
            validate_protocol_denial(exit, &data, name, target, protocol)?;
        }
        Ok(())
    }
    fn configure_offline(&self, name: &str, previous: &[&str]) -> Result<()> {
        validate_allowlist_policy(
            &self.control(self.command(&["policy", "ls", name, "--json"]))?,
            name,
            previous,
        )?;
        self.checked(&["policy", "deny", "network", "--sandbox", name, "**"])?;
        for target in previous {
            self.checked(&[
                "policy",
                "rm",
                "network",
                "--sandbox",
                name,
                "--resource",
                target,
                "--force",
            ])?;
        }
        self.verify_offline_policy(name)
    }
    fn configure_crates(&self, name: &str) -> Result<()> {
        self.configure_allowlist(name, CRATES_DESTINATIONS, CRATES_DESTINATIONS)
    }
    fn configure_generation(&self, name: &str) -> Result<()> {
        ensure!(
            self.config.mode.is_pilot(),
            "generation network policy is currently available only for the Docker pilot"
        );
        // Docker supplies the immutable Codex rules; add only Rust registry access.
        self.configure_allowlist(name, CRATES_DESTINATIONS, CODEX_DESTINATIONS)
    }
    fn configure_allowlist(&self, name: &str, added: &[&str], targets: &[&str]) -> Result<()> {
        // Never changes the global baseline or any other guest. Start closed and
        // install narrow exceptions before removing this guest's blanket deny.
        self.verify_offline_policy(name)?;
        self.checked(&[
            "policy",
            "allow",
            "network",
            "--sandbox",
            name,
            &added.join(","),
        ])?;
        self.checked(&[
            "policy",
            "rm",
            "network",
            "--sandbox",
            name,
            "--resource",
            "**",
            "--force",
        ])?;
        validate_allowlist_policy(
            &self.control(self.command(&["policy", "ls", name, "--json"]))?,
            name,
            targets,
        )?;
        for target in CODEX_DESTINATIONS.iter().copied().chain([
            "example.com:443",
            "api.anthropic.com:443",
            "169.254.169.254:80",
            "host.docker.internal:80",
        ]) {
            let allowed = targets.contains(&target);
            let (exit, bytes) = self.control_output(self.command(&[
                "policy",
                "check",
                "network",
                "--sandbox",
                name,
                "--json",
                target,
            ]))?;
            let value: serde_json::Value = serde_json::from_slice(&bytes)?;
            ensure!(
                exit == Some(if allowed { 0 } else { 1 })
                    && value["allowed"] == allowed
                    && value["context"] == format!("sandbox:{name}")
                    && value["target"] == target,
                "effective dependency policy decision mismatch"
            );
        }
        Ok(())
    }
    pub fn evidence_gate(&self) -> Result<()> {
        if self.config.mode.is_pilot() {
            return Ok(());
        }
        anyhow::bail!(
            "required isolation is not established:\n- {}\nSee docs/VERIFICATION.md. There is no override or host fallback.",
            BLOCKERS.join("\n- ")
        )
    }
}
impl Sandbox for Sbx {
    fn preflight(&self, role: Role) -> Result<()> {
        self.prerequisites(role)?;
        self.evidence_gate()?;
        if let Role::Generation(agent) = role {
            self.config.mode.validate_agent(agent)?;
        }
        for key in [
            "ssh.agentForwardingEnabled",
            "clipboard.imagePaste",
            "claude.remoteControl",
        ] {
            disabled_setting(
                &self.control(self.command(&["settings", "get", "--json", key]))?,
                key,
            )?;
        }
        let templates: Templates =
            serde_json::from_slice(&self.control(self.command(&["template", "ls", "--json"]))?)?;
        templates.contains(self.image(role)?)?;
        if let Role::Generation(agent) = role {
            templates.contains(self.image(Role::Playback)?)?;
            ensure!(
                self.has_credential(agent)?,
                "No subscription credential is stored. Run: bench auth {agent}"
            );
            let report = self.check_runtimes(Some(agent))?;
            ensure!(
                report.ready,
                "agent preflight failed:\n{}",
                report.lines().collect::<Vec<_>>().join("\n")
            );
        }
        Ok(())
    }
    fn create(&self, name: &str, role: Role) -> Result<()> {
        ensure!(
            !self.abort.load(Ordering::SeqCst),
            "aborted before guest creation"
        );
        parse_component(name).map_err(anyhow::Error::msg)?;
        let image = self.image(role)?;
        // No positional workspace, no clone, no volume, no port mapping, no host socket.
        // Initially deny all egress, before any task/code can execute.
        let mut command = self.command(&[
            "create",
            "--name",
            name,
            "--skills",
            "off",
            "--deny-network",
            "**",
            "--cpus",
            &self.config.limits.cpus.to_string(),
            "--memory",
            &format!("{}m", self.config.limits.memory_mib),
            "--pull",
            "never",
            "--template",
            &image.reference,
            role.agent(),
        ]);
        // This is consumed by the host backend when it creates the virtual disk,
        // not an environment-only restriction inside the guest. Production stays
        // gated until the effective disk and all other writable mounts are checked.
        command.env(
            "DOCKER_SANDBOXES_ROOT_SIZE",
            format!("{}m", role.root_disk_mib(self.config.limits.disk_mib)?),
        );
        eprintln!("Provisioning guest: {name}");
        // Let the bounded creation RPC settle before cleanup. Killing just its
        // client could let the daemon finish provisioning after an early rm.
        self.cleanup_control(command)?;
        ensure!(
            !self.abort.load(Ordering::SeqCst),
            "aborted during guest creation"
        );
        Ok(())
    }
    fn verify(&self, name: &str, role: Role) -> Result<EnvironmentIdentity> {
        if let Role::Generation(agent) = role {
            self.config.mode.validate_agent(agent)?;
        }
        let inspect = self.inspect(name)?;
        inspect.validate_identity(name, role, self.image(role)?, &self.config.limits)?;
        self.verify_ports(name)?;
        let probe = self.probe(name)?;
        if self.config.mode.is_pilot() {
            probe.validate_resources(&self.config.limits, role)?;
            probe.validate_sockets()?;
        } else {
            inspect.validate_services()?;
            probe.validate_machine(&self.config.limits, role)?;
            probe.validate_services()?;
        }
        if role == Role::Generation(Agent::Codex) {
            self.codex_broker(name)?.validate_subscription()?;
        }
        if role == Role::Package {
            self.configure_crates(name)?;
        } else if role == Role::Generation(Agent::Codex) && self.config.mode.is_pilot() {
            self.configure_generation(name)?;
        } else {
            self.verify_offline_policy(name)?;
        }
        // Generation needs a separately observed selected-provider policy; the
        // current backend cannot remove its automatically attached host services.
        self.evidence_gate()?;
        Ok(EnvironmentIdentity {
            backend: "sbx".into(),
            backend_version: Some(BACKEND_VERSION.into()),
            environment: self.config.environment.clone(),
            image_digest: Some(inspect.image_digest),
            rust: Some(probe.rust),
            architecture: Some(probe.architecture),
            effective_limits: Some(self.config.limits.clone()),
            network_policy: Some(
                if role == Role::Package || role == Role::Generation(Agent::Codex) {
                    serde_json::json!({"external_enforcement": "sbx", "default": "deny", "allow_tcp": if role == Role::Package { CRATES_DESTINATIONS } else { CODEX_DESTINATIONS }, "managed_services_excluded": self.config.mode.is_pilot()})
                } else {
                    serde_json::json!({"external_enforcement": "sbx", "explicit_deny": ["**"], "managed_services_excluded": self.config.mode.is_pilot()})
                },
            ),
            isolation_verified: !self.config.mode.is_pilot(),
        })
    }
    fn exec(&self, name: &str, args: &[String], interactive: bool) -> Result<Command> {
        parse_component(name).map_err(anyhow::Error::msg)?;
        let mut cmd = self.command(&["exec", "-i", "--workdir", "/workspace"]);
        if interactive {
            cmd.arg("--tty");
        }
        cmd.arg(name).args(args);
        Ok(cmd)
    }
    fn import(&self, name: &str, path: &Path, destination: &str) -> Result<()> {
        ensure!(
            ["/workspace", "/replay"].contains(&destination),
            "invalid guest import destination"
        );
        // Only a harness-produced validated tar goes INTO a fresh guest. Never use
        // sbx cp to extract an untrusted directory onto the host.
        let dest = format!("{name}:/tmp/bench-input.tar");
        let mut cmd = self.command(&["cp"]);
        cmd.arg(path).arg(dest);
        self.control(cmd)?;
        self.checked(&[
            "exec",
            "--user",
            "root",
            name,
            "tar",
            "--no-same-owner",
            "--no-same-permissions",
            "-xf",
            "/tmp/bench-input.tar",
            "-C",
            destination,
        ])
    }
    fn stop(&self, name: &str) -> Result<()> {
        self.cleanup_control(self.command(&["stop", name]))
            .map(|_| ())
    }
    fn export(
        &self,
        name: &str,
        prefix: &str,
        destination: &Path,
        raw: &Path,
        bytes: u64,
        files: usize,
    ) -> Result<()> {
        // No exec/cp after stop: capture storage, never restart the modified guest.
        ensure!(
            self.inspect(name)?.state == "stopped",
            "snapshot requires a stopped guest"
        );
        let image = raw.join("stopped-image.tar");
        ensure!(
            !image.exists(),
            "refusing to overwrite a retained stopped snapshot"
        );
        let tag = format!("spaceship-bench-export:{name}");
        let mut cmd = self.command(&["template", "save", name, &tag, "--output"]);
        cmd.arg(&image);
        // Snapshot RPCs also settle before template/guest cleanup. Cancellation
        // is observed before parsing the snapshot, without racing the daemon.
        eprintln!("Snapshotting guest: {name}");
        let saved = self.cleanup_control(cmd);
        let result = saved.and_then(|_| {
            ensure!(
                !self.abort.load(Ordering::SeqCst),
                "aborted during snapshot creation"
            );
            ensure!(
                self.inspect(name)?.state == "stopped",
                "snapshot unexpectedly changed guest state"
            );
            archive::extract_image_workspace_abort(
                &image,
                prefix,
                destination,
                self.config.limits.disk_mib * 1024 * 1024,
                bytes,
                files,
                &self.abort,
            )
            .map(|_| ())
        });
        let cleanup = self.cleanup_control(self.command(&["template", "rm", "--force", &tag]));
        finish_export(&image, raw, result.and(cleanup.map(|_| ())))
    }
    fn destroy(&self, name: &str) -> Result<()> {
        self.cleanup_control(self.command(&["rm", "--force", name]))
            .map(|_| ())
    }
}

fn finish_export(image: &Path, raw: &Path, result: Result<()>) -> Result<()> {
    if let Err(error) = &result {
        // Preserve the stopped filesystem when validation fails. Cleanup still
        // removes the guest/template, but must not erase the only recovery input.
        if image.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(image, fs::Permissions::from_mode(0o600))?;
            }
        }
        let detail = format!("{error:#}\n");
        let mut log = process::private_file(&raw.join("export-error.log"))?;
        log.write_all(&detail.as_bytes()[..detail.len().min(64 * 1024)])?;
    } else if image.exists() {
        fs::remove_file(image)?;
    }
    result
}

/// Armed BEFORE create so partial provisioning is also cleaned up.
pub struct Guest<'a, B: Sandbox> {
    backend: &'a B,
    pub name: String,
    armed: bool,
}
impl<'a, B: Sandbox> Guest<'a, B> {
    pub fn new(backend: &'a B, role: Role) -> Result<Self> {
        Self::named(
            backend,
            role,
            format!("bench-{}", uuid::Uuid::new_v4().simple()),
        )
    }
    pub fn named(backend: &'a B, role: Role, name: String) -> Result<Self> {
        parse_component(&name).map_err(anyhow::Error::msg)?;
        let guest = Self {
            backend,
            name,
            armed: true,
        };
        backend.create(&guest.name, role)?;
        Ok(guest)
    }
    pub fn destroy(mut self) -> Result<()> {
        self.backend.destroy(&self.name)?;
        self.armed = false;
        Ok(())
    }
}
impl<B: Sandbox> Drop for Guest<'_, B> {
    fn drop(&mut self) {
        if self.armed && self.backend.destroy(&self.name).is_err() {
            eprintln!("Guest cleanup failed. Run: sbx rm --force {}", self.name);
        }
    }
}

pub fn integration_check(backend: &Sbx, repo: &Path, state: &Path) -> Result<()> {
    integration_check_abort(backend, repo, state, &AtomicBool::new(false))
}

/// Diagnostic-only exception to the production preflight: fixed trusted probes
/// and a fixed harmless fixture, never a task, archived source, login or agent.
/// Strict mode retains its certification gate. Pilot mode exercises the accepted
/// Docker boundary with the same fixed fixtures and no model request.
pub fn integration_check_abort(
    backend: &Sbx,
    repo: &Path,
    state: &Path,
    abort: &AtomicBool,
) -> Result<()> {
    backend.prerequisites(Role::Diagnostic)?;
    let check_abort = || -> Result<()> {
        ensure!(
            !abort.load(Ordering::SeqCst),
            "integration check aborted; owned guests are being destroyed"
        );
        Ok(())
    };
    check_abort()?;
    eprintln!(
        "Checking a disposable VM with trusted probes only; no agent or authentication flow."
    );
    let sentinels = tempfile::Builder::new()
        .prefix(".bench-sentinel-")
        .tempdir_in(repo)?;
    fs::write(
        sentinels.path().join("host-sentinel"),
        b"CONTROLLED TEST VALUE",
    )?;
    fs::create_dir_all(repo.join("runs"))?;
    archive::ensure_directory(&repo.join("runs"))?;
    let sibling = tempfile::Builder::new()
        .prefix(".bench-sibling-")
        .tempdir_in(repo.join("runs"))?;
    fs::write(
        sibling.path().join("sibling-sentinel"),
        b"CONTROLLED TEST VALUE",
    )?;
    let first = Guest::new(backend, Role::Diagnostic)?;
    eprintln!("Diagnostic guest: {}", first.name);
    check_abort()?;
    let inspect = backend.inspect(&first.name)?;
    inspect.validate_identity(
        &first.name,
        Role::Diagnostic,
        backend.image(Role::Diagnostic)?,
        &backend.config.limits,
    )?;
    backend.verify_ports(&first.name)?;
    let probe = backend.probe(&first.name)?;
    probe.validate_machine(&backend.config.limits, Role::Diagnostic)?;
    eprintln!(
        "PASS: image identity, empty workspace, CPU/memory/root disk observations and expected guest mount table."
    );
    // Collect these failures but continue ONLY the fixed trusted transport tests.
    let services = [inspect.validate_services(), probe.validate_services()];
    for finding in &services {
        if let Err(error) = finding {
            eprintln!(
                "{}: {error}",
                if backend.config.mode.is_pilot() {
                    "PILOT LIMITATION"
                } else {
                    "FAIL"
                }
            );
        }
    }
    backend.verify_offline_policy(&first.name)?;
    if backend.config.mode.is_pilot() {
        // The shell fixture has no immutable agent rules. Reproduce the same
        // effective destinations here; check-runtimes tests the actual Codex kit.
        backend.configure_allowlist(&first.name, CODEX_DESTINATIONS, CODEX_DESTINATIONS)?;
        backend.configure_offline(&first.name, CODEX_DESTINATIONS)?;
        eprintln!("PASS: pilot generation allowlist and restoration to deny-all.");
        let identity = backend.verify(&first.name, Role::Package)?;
        ensure!(
            !identity.isolation_verified,
            "pilot incorrectly certified strict isolation"
        );
        backend.configure_offline(&first.name, CRATES_DESTINATIONS)?;
        eprintln!(
            "PASS: production pilot packaging checks accept Docker services and retain resource/mount checks."
        );
    }
    let script = "import os,sys; assert not any(os.path.exists(p) for p in sys.argv[1:]); open('/workspace/freshness-sentinel','w').write('CONTROLLED'); os.mkdir('/home/agent/.claude'); open('/home/agent/.claude/freshness-sentinel','w').write('CONTROLLED')";
    let args = vec![
        "python3".into(),
        "-c".into(),
        script.into(),
        sentinels.path().display().to_string(),
        sibling.path().display().to_string(),
    ];
    backend.control(backend.exec(&first.name, &args, false)?)?;
    check_abort()?;
    backend.configure_crates(&first.name)?;
    backend.control(backend.exec(
        &first.name,
        &[
            "python3".into(),
            "-c".into(),
            include_str!("../probes/crates_connectivity.py").into(),
        ],
        false,
    )?)?;
    eprintln!(
        "PASS: crates sparse index and checksum-verified crate download reachable only with dependency policy; provider/web/code-host requests remain denied."
    );
    first.destroy()?;
    check_abort()?;
    let second = Guest::new(backend, Role::Diagnostic)?;
    backend.inspect(&second.name)?.validate_identity(
        &second.name,
        Role::Diagnostic,
        backend.image(Role::Diagnostic)?,
        &backend.config.limits,
    )?;
    backend
        .probe(&second.name)?
        .validate_machine(&backend.config.limits, Role::Diagnostic)?;
    backend.verify_offline_policy(&second.name)?;
    backend.control(backend.exec(
        &second.name,
        &[
            "test".into(),
            "!".into(),
            "-e".into(),
            "/workspace/freshness-sentinel".into(),
        ],
        false,
    )?)?;
    eprintln!(
        "PASS: repository/sibling sentinels absent; replacement VM has no previous workspace or agent state."
    );
    check_abort()?;
    let work = tempfile::tempdir_in(state)?;
    let input = work.path().join("fixture.tar");
    archive::pack(
        &repo.join("harness/tests/fixtures/registry"),
        &input,
        1024 * 1024,
        100,
    )?;
    backend.import(&second.name, &input, "/workspace")?;
    let check = include_str!("../probes/offline_connectivity.py");
    let observations: serde_json::Value =
        serde_json::from_slice(&backend.control(backend.exec(
            &second.name,
            &["python3".into(), "-c".into(), check.into()],
            false,
        )?)?)?;
    ensure!(
        observations["proxy_http_denials"] == 7,
        "offline proxy probe incomplete"
    );
    eprintln!(
        "PASS: explicit backend policy and HTTP 403 denials for web, code host, provider, crates and host/metadata destinations."
    );
    eprintln!(
        "Direct route observations: {} DNS denials, {} HTTP denials, {} closed/refused connections (transport failures alone do not certify policy enforcement).",
        observations["direct_dns_denials"]
            .as_u64()
            .context("missing DNS observations")?,
        observations["direct_http_denials"]
            .as_u64()
            .context("missing HTTP observations")?,
        observations["direct_transport_failures"]
            .as_u64()
            .context("missing connection observations")?
    );
    backend.configure_crates(&second.name)?;
    eprintln!("Vendoring registry fixture: {}", second.name);
    process::guest_job(
        backend.exec(
            &second.name,
            &[
                "sh".into(),
                "-c".into(),
                "cargo vendor --locked --versioned-dirs /replay/vendor > /replay/config.toml"
                    .into(),
            ],
            false,
        )?,
        &backend.diagnostics,
        abort,
    )?;
    backend.configure_offline(&second.name, CRATES_DESTINATIONS)?;
    check_abort()?;
    let mut terminal = process::host_command(Path::new("python3"));
    terminal
        .arg("-c")
        .arg(include_str!("../probes/terminal_transport.py"))
        .arg(&backend.config.sbx)
        .arg(&second.name)
        .arg("bench registry fixture: 42");
    backend.control(terminal)?;
    eprintln!(
        "PASS: frozen fixture build in a 120x40 guest PTY, Ctrl-C forwarding, and transport terminal restoration."
    );
    backend.stop(&second.name)?;
    eprintln!(
        "Validating stopped-VM snapshot and source hashes (compressed image layers remain private)."
    );
    backend.export(
        &second.name,
        "workspace",
        &work.path().join("exported"),
        work.path(),
        1024 * 1024,
        100,
    )?;
    ensure!(
        archive::inventory(&work.path().join("exported"), 1024 * 1024, 100)?
            == archive::inventory(
                &repo.join("harness/tests/fixtures/registry"),
                1024 * 1024,
                100
            )?,
        "fixture export differs from submitted source"
    );
    eprintln!(
        "PASS: source exported without restart and hashes unchanged. Exporting the separate replay package."
    );
    check_abort()?;
    backend.export(
        &second.name,
        "replay",
        &work.path().join("replay"),
        work.path(),
        1024 * 1024,
        100,
    )?;
    second.destroy()?;
    crate::replay::validate_vendor_config(&archive::read_regular(
        &work.path().join("replay/config.toml"),
        65536,
    )?)?;
    let preserved = work.path().join("preserved.tar");
    let bundle = work.path().join("bundle.tar");
    archive::pack(&work.path().join("exported"), &preserved, 1024 * 1024, 100)?;
    archive::pack(&work.path().join("replay"), &bundle, 1024 * 1024, 100)?;
    let checksum = archive::file_hash(&bundle, 2 * 1024 * 1024)?;
    check_abort()?;
    eprintln!("Playing the preserved source and separate package in a third clean VM.");
    let third = Guest::new(backend, Role::Diagnostic)?;
    if backend.config.mode.is_pilot() {
        backend.preflight(Role::Playback)?;
        let identity = backend.verify(&third.name, Role::Playback)?;
        ensure!(
            !identity.isolation_verified,
            "pilot incorrectly certified strict isolation"
        );
    }
    backend
        .probe(&third.name)?
        .validate_machine(&backend.config.limits, Role::Playback)?;
    backend.verify_offline_policy(&third.name)?;
    backend.import(&third.name, &preserved, "/workspace")?;
    backend.import(&third.name, &bundle, "/replay")?;
    eprintln!("Building replay fixture: {}", third.name);
    let output = process::guest_job(
        backend.exec(
            &third.name,
            &[
                "cargo".into(),
                "--config".into(),
                "/replay/config.toml".into(),
                "run".into(),
                "--release".into(),
                "--frozen".into(),
            ],
            false,
        )?,
        &backend.diagnostics,
        abort,
    )?;
    ensure!(
        output == b"bench registry fixture: 42\n",
        "offline replay fixture failed"
    );
    ensure!(
        archive::file_hash(&bundle, 2 * 1024 * 1024)? == checksum,
        "replay bundle changed during playback"
    );
    backend.stop(&third.name)?;
    third.destroy()?;
    eprintln!(
        "PASS: locked vendoring/package export, checksum verification, clean-VM frozen playback and owned guest cleanup."
    );
    check_abort()?;
    ensure!(
        backend.config.mode.is_pilot() || services.iter().all(Result::is_ok),
        "real integration check FAILED required host-service isolation; successful fixture checks do not certify official runs. See docs/VERIFICATION.md"
    );
    backend.evidence_gate()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_export_keeps_snapshot_and_private_diagnostic() {
        let raw = tempfile::tempdir().unwrap();
        let image = raw.path().join("stopped-image.tar");
        fs::write(&image, b"original stopped filesystem").unwrap();
        let error = finish_export(
            &image,
            raw.path(),
            Err(anyhow::anyhow!("source layer size limit")),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "source layer size limit");
        assert_eq!(fs::read(&image).unwrap(), b"original stopped filesystem");
        let diagnostic = raw.path().join("export-error.log");
        assert_eq!(
            fs::read_to_string(&diagnostic).unwrap(),
            "source layer size limit\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&image, &diagnostic] {
                assert_eq!(
                    fs::metadata(path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
    }
    #[test]
    fn successful_export_removes_full_private_snapshot() {
        let raw = tempfile::tempdir().unwrap();
        let image = raw.path().join("stopped-image.tar");
        fs::write(&image, b"stopped filesystem").unwrap();
        finish_export(&image, raw.path(), Ok(())).unwrap();
        assert!(!image.exists());
        assert!(!raw.path().join("export-error.log").exists());
    }
    use serde_json::{Value, json};

    #[test]
    fn pilot_does_not_unlock_strict_certification() {
        let mut backend = Sbx {
            config: Config::default(),
            lock: EnvironmentLock {
                schema_version: 1,
                environment: "linux-rust-v2".into(),
                rust_version: "1.97.0".into(),
                images: Default::default(),
            },
            diagnostics: PathBuf::new(),
            abort: Arc::new(AtomicBool::new(false)),
        };
        assert!(backend.evidence_gate().is_err());
        assert_eq!(backend.accepted_status(), Status::Blocked);
        backend.config.mode = crate::config::ExecutionMode::DockerPilot;
        backend.evidence_gate().unwrap();
        assert_eq!(backend.accepted_status(), Status::Warning);
    }

    fn diagnostic_fixture() -> Value {
        json!({"checks": DIAGNOSTIC_CHECKS.iter().map(|name| json!({
            "name": name, "status": "pass", "message": "private@example.invalid",
            "detail": "secret-value\u{1b}[31m", "hint": "upload private logs"
        })).collect::<Vec<_>>()})
    }

    #[test]
    fn diagnostics_report_auth_setup_without_account_or_raw_details() {
        let mut data = diagnostic_fixture();
        data["checks"][12]["status"] = json!("fail");
        let report = diagnostic_summary(Some(1), &serde_json::to_vec(&data).unwrap()).unwrap();
        assert_eq!(report.len(), 1);
        assert!(report[0].contains("sbx login"));
        for private in ["private@", "secret-value", "upload", "\u{1b}"] {
            assert!(!report[0].contains(private));
        }
    }

    #[test]
    fn diagnostics_reject_missing_duplicate_unknown_or_conflicting_checks() {
        let good = diagnostic_fixture();
        assert!(diagnostic_summary(Some(0), &serde_json::to_vec(&good).unwrap()).is_ok());
        assert!(diagnostic_summary(Some(1), &serde_json::to_vec(&good).unwrap()).is_err());
        let mut missing = good.clone();
        missing["checks"].as_array_mut().unwrap().pop();
        let mut duplicate = good.clone();
        duplicate["checks"][12]["name"] = json!("Daemon");
        let mut unknown = good;
        unknown["checks"][0]["status"] = json!("maybe");
        for bad in [missing, duplicate, unknown] {
            assert!(diagnostic_summary(Some(0), &serde_json::to_vec(&bad).unwrap()).is_err());
        }
    }

    #[test]
    fn evaluated_settings_require_the_expected_boolean_not_just_a_key() {
        let key = "ssh.agentForwardingEnabled";
        for data in [
            json!({"key":key,"type":"bool","value":true}),
            json!({"key":key,"type":"bool","value":"false"}),
            json!({"key":"other","type":"bool","value":false}),
            json!({"key":key,"type":"bool"}),
        ] {
            assert!(disabled_setting(&serde_json::to_vec(&data).unwrap(), key).is_err());
        }
        disabled_setting(
            &serde_json::to_vec(&json!({"key":key,"type":"bool","value":false})).unwrap(),
            key,
        )
        .unwrap();
    }

    #[test]
    fn doctor_checks_actual_image_identity_not_only_a_tag_or_lock_entry() {
        let image = Image {
            reference: "spaceship-bench-base:linux-rust-v2".into(),
            image_id: format!("sha256:{}", "ab".repeat(32)),
        };
        let data = json!({"images":[{"repository":"docker.io/library/spaceship-bench-base", "tag":"linux-rust-v2", "id":"abababababab"}]});
        serde_json::from_value::<Templates>(data.clone())
            .unwrap()
            .contains(&image)
            .unwrap();
        for (field, value) in [
            ("id", "cdcdcdcdcdcd"),
            ("id", "ab"),
            ("tag", "linux-rust-v1"),
            ("repository", "other"),
        ] {
            let mut changed = data.clone();
            changed["images"][0][field] = json!(value);
            assert!(
                serde_json::from_value::<Templates>(changed)
                    .unwrap()
                    .contains(&image)
                    .is_err()
            );
        }
    }
}
