use crate::{
    archive,
    config::Config,
    process,
    protocol::{ArtifactStatus, Run, atomic_json},
    sandbox::{Guest, Role, Sandbox},
    terminal::Terminal,
};
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replay {
    pub schema_version: u32,
    pub preparation: ArtifactStatus,
    pub reason: Option<String>,
    pub source_tree_sha256: Option<String>,
    pub environment_image: Option<String>,
    /// A basename beneath the private state's bundles directory, never an arbitrary path.
    pub local_bundle: Option<String>,
    pub bundle_sha256: Option<String>,
    pub bundle_bytes: Option<u64>,
    pub launch: ArtifactStatus,
    pub launch_exit_code: Option<i32>,
    pub last_played_at: Option<chrono::DateTime<Utc>>,
}
impl Default for Replay {
    fn default() -> Self {
        Self {
            schema_version: 1,
            preparation: ArtifactStatus::Pending,
            reason: None,
            source_tree_sha256: None,
            environment_image: None,
            local_bundle: None,
            bundle_sha256: None,
            bundle_bytes: None,
            launch: ArtifactStatus::Unavailable,
            launch_exit_code: None,
            last_played_at: None,
        }
    }
}

pub fn validate_submission(root: &Path) -> Result<()> {
    ensure!(root.join("Cargo.toml").is_file(), "missing Cargo.toml");
    ensure!(
        root.join("Cargo.lock").is_file(),
        "missing Cargo.lock; resolution will not be regenerated"
    );
    let lock: toml::Value = toml::from_str(std::str::from_utf8(&archive::read_regular(
        &root.join("Cargo.lock"),
        4 * 1024 * 1024,
    )?)?)?;
    for package in lock
        .get("package")
        .and_then(toml::Value::as_array)
        .context("invalid Cargo.lock packages")?
    {
        if let Some(source) = package.get("source") {
            ensure!(
                source.as_str() == Some("registry+https://github.com/rust-lang/crates.io-index"),
                "only crates.io locked dependencies are allowed"
            );
            let checksum = package
                .get("checksum")
                .and_then(toml::Value::as_str)
                .context("registry dependency has no checksum")?;
            ensure!(
                checksum.len() == 64 && checksum.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid crate checksum"
            );
        }
    }
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_file() && entry.file_name() == "Cargo.toml" {
            let value: toml::Value = toml::from_str(std::str::from_utf8(&archive::read_regular(
                entry.path(),
                1024 * 1024,
            )?)?)?;
            validate_manifest_values(
                &value,
                entry.path().parent().context("manifest parent")?,
                root,
            )?;
        }
    }
    Ok(())
}
fn validate_manifest_values(value: &toml::Value, parent: &Path, root: &Path) -> Result<()> {
    match value {
        toml::Value::Table(t) => {
            ensure!(
                !t.contains_key("git"),
                "Git Cargo dependencies are not permitted"
            );
            if let Some(registry) = t.get("registry") {
                ensure!(
                    registry.as_str() == Some("crates-io"),
                    "custom Cargo registry is not permitted"
                );
            }
            if let Some(path) = t.get("path").and_then(toml::Value::as_str) {
                let resolved = parent
                    .join(path)
                    .canonicalize()
                    .context("missing path dependency or target")?;
                ensure!(
                    resolved.starts_with(root.canonicalize()?),
                    "Cargo path escapes submitted source"
                );
            }
            for v in t.values() {
                validate_manifest_values(v, parent, root)?;
            }
        }
        toml::Value::Array(a) => {
            for v in a {
                validate_manifest_values(v, parent, root)?;
            }
        }
        _ => (),
    }
    Ok(())
}

// Constant script: generated paths, manifests and prompt text are never interpolated.
const VENDOR: &str = r#"set -eu
cd /workspace
test -f Cargo.lock
mkdir -p /replay/vendor
before=$(sha256sum Cargo.lock)
cargo vendor --locked --versioned-dirs /replay/vendor > /replay/config.toml
test "$before" = "$(sha256sum Cargo.lock)"
"#;
const PLAY: &str = r#"set -eu
stty rows 40 cols 120
test "$(stty size)" = '40 120'
export LANG=C.UTF-8 LC_ALL=C.UTF-8 TERM=xterm-256color
export CARGO_NET_OFFLINE=true
cd /workspace
exec cargo --config /replay/config.toml run --release --frozen
"#;

pub fn prepare<B: Sandbox>(
    backend: &B,
    config: &Config,
    run_dir: &Path,
    run: &Run,
    state: &Path,
) -> Result<Replay> {
    prepare_abort(
        backend,
        config,
        run_dir,
        run,
        state,
        &AtomicBool::new(false),
    )
}

pub fn prepare_abort<B: Sandbox>(
    backend: &B,
    config: &Config,
    run_dir: &Path,
    run: &Run,
    state: &Path,
    abort: &AtomicBool,
) -> Result<Replay> {
    let mut replay = Replay {
        source_tree_sha256: run.export.tree_sha256.clone(),
        ..Replay::default()
    };
    let result = (|| -> Result<()> {
        let source = run_dir.join("solution");
        validate_submission(&source)?;
        backend.preflight(Role::Package)?;
        let work = tempfile::Builder::new().prefix("pack-").tempdir_in(state)?;
        let input = work.path().join("source.tar");
        archive::pack(
            &source,
            &input,
            config.limits.export_bytes,
            config.limits.export_files,
        )?;
        let guest = Guest::new(backend, Role::Package)?;
        let identity = backend.verify(&guest.name, Role::Package)?;
        backend.import(&guest.name, &input, "/workspace")?;
        // cargo vendor runs ONLY in the separate agent-free VM, with dependency egress.
        process::guest_job(
            backend.exec(
                &guest.name,
                &["bash".into(), "-c".into(), VENDOR.into()],
                false,
            )?,
            &state.join("raw").join(&run.run_id).join("packaging"),
            abort,
        )?;
        backend.stop(&guest.name)?;
        let exported = work.path().join("replay");
        backend.export(
            &guest.name,
            "replay",
            &exported,
            work.path(),
            config.limits.replay_bytes,
            config.limits.replay_files,
        )?;
        // Confirm vendor did not change the submitted tree in the package guest.
        let after = work.path().join("source-after");
        backend.export(
            &guest.name,
            "workspace",
            &after,
            work.path(),
            config.limits.export_bytes,
            config.limits.export_files,
        )?;
        ensure!(
            archive::inventory(
                &after,
                config.limits.export_bytes,
                config.limits.export_files
            )? == run.export.files,
            "packaging changed source; replay is invalid"
        );
        guest.destroy()?;
        let vendor_config = archive::read_regular(&exported.join("config.toml"), 64 * 1024)?;
        validate_vendor_config(&vendor_config)?;
        let bundle = work.path().join("bundle.tar");
        archive::pack(
            &exported,
            &bundle,
            config.limits.replay_bytes,
            config.limits.replay_files,
        )?;
        let wire_limit =
            config.limits.replay_bytes + (config.limits.replay_files as u64 + 10) * 2048;
        let checksum = archive::file_hash(&bundle, wire_limit)?;
        let bundles = state.join("bundles");
        fs::create_dir_all(&bundles)?;
        archive::ensure_directory(&bundles)?;
        let name = format!("{}-{checksum}.tar", run.run_id);
        let dest = bundles.join(&name);
        ensure!(!dest.exists(), "replay bundle already exists");
        replay.bundle_bytes = Some(fs::metadata(&bundle)?.len());
        fs::rename(bundle, dest)?;
        replay.bundle_sha256 = Some(checksum);
        replay.local_bundle = Some(name);
        replay.environment_image = identity.image_digest;
        replay.preparation = ArtifactStatus::Complete;
        Ok(())
    })();
    if result.is_err() {
        replay.preparation = ArtifactStatus::Failed;
        // Do not publish arbitrary cargo stderr/manifests in metadata.
        replay.reason = Some("locked_dependency_packaging_failed; inspect private setup logs or validate source/lockfile; no source was repaired".into());
    }
    atomic_json(&run_dir.join("replay.json"), &replay)?;
    Ok(replay)
}

pub fn validate_vendor_config(bytes: &[u8]) -> Result<()> {
    // Cargo may output nothing for projects with no registry dependencies.
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(());
    }
    let config: toml::Value = toml::from_str(std::str::from_utf8(bytes)?)?;
    let source = config
        .get("source")
        .and_then(toml::Value::as_table)
        .context("vendor config has no source table")?;
    ensure!(
        config.as_table().is_some_and(|t| t.len() == 1) && source.len() == 2,
        "unexpected vendoring configuration"
    );
    ensure!(
        source
            .get("crates-io")
            .and_then(|v| v.get("replace-with"))
            .and_then(toml::Value::as_str)
            == Some("vendored-sources"),
        "missing crates.io replacement"
    );
    ensure!(
        source
            .get("vendored-sources")
            .and_then(|v| v.get("directory"))
            .and_then(toml::Value::as_str)
            == Some("/replay/vendor"),
        "vendor directory mismatch"
    );
    ensure!(
        source
            .values()
            .all(|v| v.as_table().is_some_and(|t| t.len() == 1)),
        "extra vendor settings"
    );
    Ok(())
}

pub fn play<B: Sandbox>(
    backend: &B,
    config: &Config,
    run_dir: &Path,
    run: &Run,
    state: &Path,
    abort: &AtomicBool,
) -> Result<()> {
    let path = run_dir.join("replay.json");
    let mut replay: Replay = serde_json::from_slice(&archive::read_regular(&path, 1024 * 1024)?)?;
    ensure!(
        replay.schema_version == 1 && replay.preparation == ArtifactStatus::Complete,
        "no prepared replay bundle; a broken or unpackaged submission is preserved as-is"
    );
    ensure!(
        replay.source_tree_sha256 == run.export.tree_sha256 && replay.source_tree_sha256.is_some(),
        "replay/source mismatch"
    );
    let name = replay
        .local_bundle
        .as_deref()
        .context("missing local replay reference")?;
    let relative = archive::safe_path(name.as_bytes())?;
    ensure!(
        relative.components().count() == 1 && name.ends_with(".tar"),
        "invalid replay reference"
    );
    let bundle = state.join("bundles").join(relative);
    let wire_limit = config.limits.replay_bytes + (config.limits.replay_files as u64 + 10) * 2048;
    ensure!(
        Some(archive::file_hash(&bundle, wire_limit)?) == replay.bundle_sha256
            && Some(fs::metadata(&bundle)?.len()) == replay.bundle_bytes,
        "replay bundle checksum/size mismatch"
    );
    backend.preflight(Role::Playback)?;
    let _terminal = Terminal::require_120x40()?;
    let result = (|| -> Result<Option<i32>> {
        let work = tempfile::tempdir_in(state)?;
        let unpacked = work.path().join("unpacked");
        archive::extract(
            File::open(&bundle)?,
            &unpacked,
            config.limits.replay_bytes,
            config.limits.replay_files,
        )?;
        validate_vendor_config(&archive::read_regular(
            &unpacked.join("config.toml"),
            65536,
        )?)?;
        let source_tar = work.path().join("source.tar");
        archive::pack(
            &run_dir.join("solution"),
            &source_tar,
            config.limits.export_bytes,
            config.limits.export_files,
        )?;
        let guest = Guest::new(backend, Role::Playback)?;
        let identity = backend.verify(&guest.name, Role::Playback)?;
        ensure!(
            identity.image_digest == replay.environment_image && identity.image_digest.is_some(),
            "replay runtime identity mismatch"
        );
        ensure!(!abort.load(Ordering::SeqCst), "play aborted before launch");
        backend.import(&guest.name, &source_tar, "/workspace")?;
        let replay_tar = work.path().join("replay.tar");
        archive::pack(
            &unpacked,
            &replay_tar,
            config.limits.replay_bytes,
            config.limits.replay_files,
        )?;
        backend.import(&guest.name, &replay_tar, "/replay")?;
        let exit = process::interactive_abort(
            backend.exec(
                &guest.name,
                &["bash".into(), "-c".into(), PLAY.into()],
                true,
            )?,
            abort,
        )?;
        backend.stop(&guest.name)?;
        guest.destroy()?;
        Ok(exit)
    })();
    replay.last_played_at = Some(Utc::now());
    match &result {
        Ok(code) => {
            replay.launch_exit_code = *code;
            replay.launch = if *code == Some(0) {
                ArtifactStatus::Complete
            } else {
                ArtifactStatus::Failed
            };
        }
        Err(_) => {
            replay.launch = ArtifactStatus::Failed;
            replay.reason = Some("playback_infrastructure_or_build_failure".into());
        }
    }
    atomic_json(&path, &replay)?;
    result?;
    Ok(())
}
