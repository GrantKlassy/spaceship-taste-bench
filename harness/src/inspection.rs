//! Schemas observed on local sbx 0.45.0, before any agent/task executes.
//! Raw fields are never rendered or copied into public run metadata.
use crate::{
    agents::Agent,
    config::Limits,
    sandbox::{CLAUDE_VOLUMES, Image, Role},
};
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Deserialize)]
pub(crate) struct Inspect {
    pub name: String,
    pub agent: String,
    pub state: String,
    pub image: String,
    pub image_digest: String,
    pub cpus: u32,
    pub memory: String,
    kits: Vec<Value>,
    runtime_mounts: Vec<Value>,
    mcp_gateway: bool,
    secrets: Vec<Value>,
    sessions: u64,
    daemon_version: String,
    // Built-in provider runtimes add this descriptive field. It is not proof
    // of authentication or subscription billing.
    #[serde(rename = "auth_mode", default)]
    _auth_mode: Option<String>,
    // Known descriptive fields. They convey no isolation proof.
    #[serde(rename = "uptime", default)]
    _uptime: Option<String>,
    #[serde(rename = "daemon_uptime")]
    _daemon_uptime: String,
    #[serde(rename = "network")]
    _network: String,
    #[serde(rename = "network_policy")]
    _network_policy: Value,
    #[serde(rename = "proxy")]
    _proxy: String,
    #[serde(flatten)]
    unknown: BTreeMap<String, Value>,
}
impl Inspect {
    pub fn validate_identity(
        &self,
        name: &str,
        role: Role,
        image: &Image,
        limits: &Limits,
    ) -> Result<()> {
        ensure!(
            self.unknown.is_empty(),
            "unrecognized backend inspection fields"
        );
        ensure!(
            self.name == name && self.state == "running" && self.agent == role.agent(),
            "guest identity/state mismatch"
        );
        ensure!(
            self.image == image.reference && self.image_digest == image.image_id,
            "guest image identity mismatch"
        );
        ensure!(
            self.daemon_version == "v0.45.0",
            "unverified backend daemon version"
        );
        ensure!(
            self.cpus == limits.cpus
                && memory_bytes(&self.memory) == Some(limits.memory_mib * 1024 * 1024),
            "effective CPU/memory allocation mismatch"
        );
        ensure!(
            self.kits.is_empty() && self.runtime_mounts.is_empty() && self.sessions == 0,
            "unexpected kit, runtime mount or active agent session"
        );
        Ok(())
    }
    pub fn validate_services(&self) -> Result<()> {
        ensure!(
            !self.mcp_gateway,
            "backend exposes an MCP gateway; agent configuration cannot disable this external service"
        );
        ensure!(
            self.secrets.is_empty(),
            "backend attached credentials/services; per-provider scoping is not verified"
        );
        Ok(())
    }
}
fn memory_bytes(value: &str) -> Option<u64> {
    let (number, unit) = value.split_at_checked(value.len().checked_sub(1)?)?;
    let multiplier = match unit {
        "m" => 1024 * 1024,
        "g" => 1024 * 1024 * 1024,
        _ => return None,
    };
    number.parse::<u64>().ok()?.checked_mul(multiplier)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CodexBroker {
    mode: String,
    placeholder_auth: bool,
    no_api_key_override: bool,
}
impl CodexBroker {
    pub fn validate_subscription(&self) -> Result<()> {
        ensure!(
            self.mode == "oauth",
            "Codex requires the backend's OAuth subscription mode; complete bench auth codex and create a fresh guest"
        );
        ensure!(
            self.placeholder_auth && self.no_api_key_override,
            "Codex broker credentials are not the expected placeholders, or an API-key override is present"
        );
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GuestProbe {
    workspace_empty: bool,
    personal_state_absent: bool,
    ssh_socket_present: bool,
    host_socket_present: bool,
    environment_keys: Vec<String>,
    cpus: u32,
    memory_bytes: u64,
    storage: Vec<Storage>,
    runtime_storage: BTreeMap<String, RuntimeStorage>,
    pub rust: String,
    pub architecture: String,
    mounts: Mounts,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Storage {
    device: u64,
    bytes: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeStorage {
    device: u64,
    bytes: u64,
    device_bytes: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mounts {
    filesystems: Vec<Mount>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mount {
    target: String,
    source: String,
    fstype: String,
    options: String,
    #[serde(default)]
    children: Vec<Mount>,
}
impl GuestProbe {
    pub fn validate_machine(&self, limits: &Limits, role: Role) -> Result<()> {
        ensure!(
            self.personal_state_absent,
            "fresh guest contains agent state; backend-generated configuration needs separate verification"
        );
        self.validate_resources(limits, role)
    }
    pub fn validate_resources(&self, limits: &Limits, role: Role) -> Result<()> {
        ensure!(self.workspace_empty, "fresh guest workspace is not empty");
        ensure!(
            self.rust == "rustc 1.97.0 (2d8144b78 2026-07-07)" && self.architecture == "x86_64",
            "guest toolchain/architecture mismatch (only the resolved amd64 environment is enabled)"
        );
        let memory = limits.memory_mib * 1024 * 1024;
        ensure!(
            self.cpus == limits.cpus
                && self.memory_bytes <= memory
                && self.memory_bytes >= memory * 9 / 10,
            "guest CPU/memory observations disagree with allocation"
        );
        let disk = role.root_disk_mib(limits.disk_mib)? * 1024 * 1024;
        ensure!(
            self.storage.len() == 4
                && self
                    .storage
                    .iter()
                    .all(|s| s.device == self.storage[0].device
                        && s.bytes <= disk
                        && s.bytes >= disk * 9 / 10),
            "guest writable root/workspace/home/tmp storage bound mismatch"
        );
        let mut mounts = BTreeMap::new();
        fn collect<'a>(
            items: &'a [Mount],
            result: &mut BTreeMap<&'a str, &'a Mount>,
        ) -> Result<()> {
            for item in items {
                ensure!(
                    result.insert(&item.target, item).is_none(),
                    "duplicate mount target"
                );
                collect(&item.children, result)?;
            }
            Ok(())
        }
        collect(&self.mounts.filesystems, &mut mounts)?;
        let mut expected = vec![
            ("/", "overlay"),
            ("/proc", "proc"),
            ("/dev", "tmpfs"),
            ("/dev/pts", "devpts"),
            ("/dev/shm", "tmpfs"),
            ("/dev/mqueue", "mqueue"),
            ("/sys", "sysfs"),
            ("/sys/fs/cgroup", "cgroup2"),
            ("/run", "tmpfs"),
            ("/run/secrets", "tmpfs"),
            ("/etc/resolv.conf", "virtiofs"),
        ];
        let volumes = if role == Role::Generation(Agent::Claude) {
            &CLAUDE_VOLUMES[..]
        } else {
            &[]
        };
        ensure!(
            self.runtime_storage.len() == volumes.len(),
            "unexpected writable runtime storage"
        );
        let mut devices = std::collections::BTreeSet::from([self.storage[0].device]);
        for &(target, mib) in volumes {
            let volume = self
                .runtime_storage
                .get(target)
                .ok_or_else(|| anyhow::anyhow!("missing runtime storage measurement"))?;
            let bytes = mib * 1024 * 1024;
            ensure!(
                devices.insert(volume.device)
                    && volume.device_bytes == bytes
                    && volume.bytes <= bytes
                    && volume.bytes >= bytes * 9 / 10,
                "runtime volume is not a distinct device with its expected capacity"
            );
            expected.push((target, "ext4"));
        }
        ensure!(mounts.len() == expected.len(), "unexpected guest mount");
        for (target, kind) in expected {
            let item = mounts
                .get(target)
                .ok_or_else(|| anyhow::anyhow!("missing expected guest mount"))?;
            ensure!(item.fstype == kind, "unexpected guest filesystem type");
            if kind == "ext4" {
                ensure!(
                    item.source.starts_with("/dev/vd")
                        && !item.source.contains('[')
                        && item.options.split(',').any(|o| o == "rw"),
                    "unexpected runtime volume source or mode"
                );
            }
            if target == "/etc/resolv.conf" {
                ensure!(
                    item.options.split(',').any(|o| o == "ro")
                        && item.source.starts_with("bind-")
                        && item.source.ends_with("[/resolv.conf]"),
                    "unexpected backend DNS-file mount"
                );
            }
        }
        Ok(())
    }
    pub fn validate_services(&self) -> Result<()> {
        ensure!(
            !self.ssh_socket_present && !self.host_socket_present,
            "guest exposes an SSH-agent or host-service socket"
        );
        ensure!(
            !self.environment_keys.iter().any(|k| k.ends_with("_API_KEY")
                || k.ends_with("_TOKEN")
                || k.starts_with("SBX_CRED_")
                || k.starts_with("MCP_")),
            "guest has provider/integration credential bindings; removal and scoping are unverified"
        );
        Ok(())
    }
    pub fn ssh_socket_absent(&self) -> bool {
        !self.ssh_socket_present
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policies {
    rules: Vec<Rule>,
}
#[derive(Deserialize)]
struct Rule {
    scope: String,
    applies_to: String,
    resource_type: String,
    decision: String,
    resources: Vec<String>,
    status: String,
    actions: Vec<String>,
}
pub(crate) fn validate_offline_policy(bytes: &[u8], name: &str) -> Result<()> {
    let policy: Policies = serde_json::from_slice(bytes)?;
    let network: Vec<_> = policy
        .rules
        .iter()
        .filter(|r| r.resource_type == "network")
        .collect();
    ensure!(network.len() == 1, "unexpected or inherited network rules");
    let rule = network[0];
    let scope = format!("sandbox:{name}");
    ensure!(
        rule.scope == scope
            && rule.applies_to == scope
            && rule.status == "active"
            && rule.decision == "deny"
            && rule.resources == ["**"]
            && rule.actions == ["net:connect:tcp", "net:connect:udp"],
        "no explicit effective per-guest deny-all rule"
    );
    Ok(())
}
pub(crate) fn validate_denial(
    exit: Option<i32>,
    bytes: &[u8],
    name: &str,
    target: &str,
) -> Result<()> {
    validate_protocol_denial(exit, bytes, name, target, "tcp")
}

pub(crate) fn validate_protocol_denial(
    exit: Option<i32>,
    bytes: &[u8],
    name: &str,
    target: &str,
    protocol: &str,
) -> Result<()> {
    ensure!(
        ["tcp", "udp"].contains(&protocol),
        "unsupported policy protocol"
    );
    let check: Value = serde_json::from_slice(bytes)?;
    ensure!(
        exit == Some(1)
            && check["allowed"] == false
            && check["deny_kind"] == "explicit"
            && check["context"] == format!("sandbox:{name}")
            && check["target"] == target
            && check["action"] == format!("net:connect:{protocol}"),
        "backend authorizer did not explicitly deny the expected destination"
    );
    Ok(())
}

pub(crate) fn validate_crates_policy(bytes: &[u8], name: &str) -> Result<()> {
    let policy: Policies = serde_json::from_slice(bytes)?;
    let network: Vec<_> = policy
        .rules
        .iter()
        .filter(|r| r.resource_type == "network")
        .collect();
    ensure!(
        network.len() == 2,
        "dependency policy has unexpected network rules"
    );
    let scope = format!("sandbox:{name}");
    for target in ["index.crates.io:443", "static.crates.io:443"] {
        ensure!(
            network
                .iter()
                .filter(|r| r.scope == scope
                    && r.applies_to == scope
                    && r.decision == "allow"
                    && r.status == "active"
                    && r.resources == [target]
                    && r.actions == ["net:connect:tcp"])
                .count()
                == 1,
            "dependency policy does not allow exactly the observed registry destinations"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codex_subscription_requires_oauth_and_only_broker_placeholders() {
        let accepted = serde_json::json!({
            "mode": "oauth", "placeholder_auth": true, "no_api_key_override": true
        });
        serde_json::from_value::<CodexBroker>(accepted.clone())
            .unwrap()
            .validate_subscription()
            .unwrap();
        for mode in ["none", "apikey", "unknown", ""] {
            let mut rejected = accepted.clone();
            rejected["mode"] = mode.into();
            assert!(
                serde_json::from_value::<CodexBroker>(rejected)
                    .unwrap()
                    .validate_subscription()
                    .is_err()
            );
        }
        for key in ["placeholder_auth", "no_api_key_override"] {
            let mut rejected = accepted.clone();
            rejected[key] = false.into();
            assert!(
                serde_json::from_value::<CodexBroker>(rejected)
                    .unwrap()
                    .validate_subscription()
                    .is_err()
            );
        }
        assert!(serde_json::from_str::<CodexBroker>(r#"{"mode":"oauth"}"#).is_err());
    }
    fn inspect() -> Inspect {
        serde_json::from_str(include_str!("../tests/fixtures/sbx/inspect.json")).unwrap()
    }
    #[test]
    fn observed_inspection_rejects_automatic_mcp() {
        let observed = inspect();
        let image = Image {
            reference: observed.image.clone(),
            image_id: observed.image_digest.clone(),
        };
        let limits = Limits {
            cpus: 2,
            memory_mib: 2048,
            ..Limits::default()
        };
        observed
            .validate_identity("bench-observed", Role::Diagnostic, &image, &limits)
            .unwrap();
        assert!(
            observed
                .validate_services()
                .unwrap_err()
                .to_string()
                .contains("MCP")
        );
    }
    #[test]
    fn effective_inspection_rejects_extra_mounts_identity_drift_and_unknown_fields() {
        for (key, value) in [
            (
                "runtime_mounts",
                serde_json::json!([{"source":"private-path"}]),
            ),
            ("cpus", serde_json::json!(8)),
            ("unexpected_forwarder", serde_json::json!(true)),
        ] {
            let mut data: Value =
                serde_json::from_str(include_str!("../tests/fixtures/sbx/inspect.json")).unwrap();
            let original = inspect();
            data[key] = value;
            let changed: Inspect = serde_json::from_value(data).unwrap();
            assert!(
                changed
                    .validate_identity(
                        "bench-observed",
                        Role::Diagnostic,
                        &Image {
                            reference: original.image,
                            image_id: original.image_digest
                        },
                        &Limits {
                            cpus: 2,
                            memory_mib: 2048,
                            ..Limits::default()
                        }
                    )
                    .is_err()
            );
        }
    }
    #[test]
    fn observed_policy_requires_effective_deny_not_a_preset_name() {
        let data = include_bytes!("../tests/fixtures/sbx/policy.json");
        validate_offline_policy(data, "bench-observed").unwrap();
        assert!(validate_offline_policy(data, "other").is_err());
        let mut policy: Value = serde_json::from_slice(data).unwrap();
        policy["rules"][2]["decision"] = Value::from("allow");
        assert!(
            validate_offline_policy(&serde_json::to_vec(&policy).unwrap(), "bench-observed")
                .is_err()
        );
    }
    #[test]
    fn denial_is_valid_structured_output_with_a_nonzero_exit() {
        let bytes = br#"{"allowed":false,"deny_kind":"explicit","context":"sandbox:bench-test","target":"example.com:443","action":"net:connect:tcp"}"#;
        validate_denial(Some(1), bytes, "bench-test", "example.com:443").unwrap();
        for exit in [Some(0), Some(2), None] {
            assert!(validate_denial(exit, bytes, "bench-test", "example.com:443").is_err());
        }
    }
    #[test]
    fn dependency_policy_rejects_additional_destinations() {
        let data = include_bytes!("../tests/fixtures/sbx/crates-policy.json");
        validate_crates_policy(data, "bench-observed").unwrap();
        let mut policy: Value = serde_json::from_slice(data).unwrap();
        policy["rules"][2]["resources"] = serde_json::json!(["**"]);
        assert!(
            validate_crates_policy(&serde_json::to_vec(&policy).unwrap(), "bench-observed")
                .is_err()
        );
    }
    #[test]
    fn guest_observations_reject_forwarding_extra_mounts_and_storage_drift() {
        let data = include_bytes!("../tests/fixtures/sbx/guest.json");
        let probe: GuestProbe = serde_json::from_slice(data).unwrap();
        let limits = Limits {
            cpus: 2,
            memory_mib: 2048,
            disk_mib: 2048,
            ..Limits::default()
        };
        probe.validate_machine(&limits, Role::Diagnostic).unwrap();
        assert!(probe.validate_services().is_err());
        let mut data: Value = serde_json::from_slice(data).unwrap();
        data["storage"][1]["device"] = Value::from(123);
        assert!(
            serde_json::from_value::<GuestProbe>(data.clone())
                .unwrap()
                .validate_machine(&limits, Role::Diagnostic)
                .is_err()
        );
        data["storage"][1]["device"] = Value::from(0);
        data["mounts"]["filesystems"][0]["children"].as_array_mut().unwrap().push(serde_json::json!({
            "target":"/host", "source":"private-host-workspace", "fstype":"virtiofs", "options":"rw"
        }));
        assert!(
            serde_json::from_value::<GuestProbe>(data)
                .unwrap()
                .validate_machine(&limits, Role::Diagnostic)
                .is_err()
        );
    }
    #[test]
    fn claude_runtime_volumes_count_against_the_total_disk_budget() {
        let mut data: Value =
            serde_json::from_slice(include_bytes!("../tests/fixtures/sbx/guest.json")).unwrap();
        data["personal_state_absent"] = false.into();
        for (index, (path, mib)) in CLAUDE_VOLUMES.iter().enumerate() {
            data["runtime_storage"][path] = serde_json::json!({
                "device": 65072 + 16 * index,
                "bytes": if index == 0 { 2040373248_u64 } else { 510873600 },
                "device_bytes": mib * 1024 * 1024,
            });
            data["mounts"]["filesystems"][0]["children"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({
                    "target": path,
                    "source": format!("/dev/vd{}", (b'd' + index as u8) as char),
                    "fstype": "ext4", "options": "rw,relatime"
                }));
        }
        let limits = Limits {
            cpus: 2,
            memory_mib: 2048,
            disk_mib: 6144,
            ..Limits::default()
        };
        let role = Role::Generation(Agent::Claude);
        let probe: GuestProbe = serde_json::from_value(data.clone()).unwrap();
        probe.validate_resources(&limits, role).unwrap();
        assert!(probe.validate_machine(&limits, role).is_err());
        assert!(probe.validate_resources(&limits, Role::Playback).is_err());
        assert!(
            probe
                .validate_resources(
                    &Limits {
                        disk_mib: 4096,
                        ..limits.clone()
                    },
                    role
                )
                .is_err()
        );
        for (key, value) in [
            ("device", 0_u64),
            ("bytes", 3 * 1024 * 1024 * 1024),
            ("device_bytes", 3 * 1024 * 1024 * 1024),
        ] {
            let mut changed = data.clone();
            changed["runtime_storage"][CLAUDE_VOLUMES[0].0][key] = value.into();
            assert!(
                serde_json::from_value::<GuestProbe>(changed)
                    .unwrap()
                    .validate_resources(&limits, role)
                    .is_err()
            );
        }
        let volumes = data["runtime_storage"].as_object_mut().unwrap();
        let removed = volumes.remove(CLAUDE_VOLUMES[0].0).unwrap();
        volumes.insert("/unrecognized".into(), removed);
        assert!(
            serde_json::from_value::<GuestProbe>(data)
                .unwrap()
                .validate_resources(&limits, role)
                .is_err()
        );
    }
}
