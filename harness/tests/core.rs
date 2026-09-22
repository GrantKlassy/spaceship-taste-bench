use bench::{
    agents::{Agent, Completion, Events},
    archive,
    config::{Cli, Config, Limits},
    protocol::{self, RunStore, Task},
    replay,
};
use chrono::TimeZone;
use clap::Parser;
use std::{fs, io::Cursor, path::Path};

fn repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("tasks/spaceship-v1")).unwrap();
    fs::create_dir(tmp.path().join("templates")).unwrap();
    fs::write(
        tmp.path().join("tasks/spaceship-v1/prompt.md"),
        include_bytes!("../../tasks/spaceship-v1/prompt.md"),
    )
    .unwrap();
    fs::write(
        tmp.path().join("tasks/spaceship-v1/task.toml"),
        include_bytes!("../../tasks/spaceship-v1/task.toml"),
    )
    .unwrap();
    fs::write(
        tmp.path().join("templates/review.md"),
        include_bytes!("../../templates/review.md"),
    )
    .unwrap();
    tmp
}
fn tar(entries: &[(&str, u8, &[u8])]) -> Vec<u8> {
    let mut out = tar::Builder::new(Vec::new());
    for (name, kind, data) in entries {
        let mut h = tar::Header::new_ustar();
        // Construct hostile names directly; Builder::append_data would reject them first.
        h.as_mut_bytes()[..100].fill(0);
        h.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
        h.set_entry_type(tar::EntryType::new(*kind));
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        out.append(&h, Cursor::new(data)).unwrap();
    }
    out.into_inner().unwrap()
}
#[test]
fn arguments_require_model_and_reject_traversal_or_shell_syntax() {
    assert!(Cli::try_parse_from(["bench", "run", "--agent", "codex"]).is_err());
    for model in ["", "--foo", "x;touch /tmp/x", "$(id)", "a\nsecret"] {
        assert!(
            Cli::try_parse_from(["bench", "run", "--agent", "codex", "--model", model]).is_err()
        );
    }
    for id in ["../run", "/absolute", "a/b", "a\\b", "-x", "foo\x1b[0m"] {
        assert!(Cli::try_parse_from(["bench", "play", id]).is_err());
    }
    let cli = Cli::try_parse_from([
        "bench",
        "run",
        "--agent",
        "claude",
        "--model",
        "claude-exact-1",
    ])
    .unwrap();
    assert!(
        matches!(cli.command, bench::config::Commands::Run { task, .. } if task == "spaceship-v1")
    );
}
#[test]
fn configuration_rejects_unknown_or_unsafe_settings() {
    assert!(toml::from_str::<Config>("deadline_seconds = 30").is_err());
    assert!(
        toml::from_str::<Config>("backend = 'host'")
            .unwrap()
            .validate()
            .is_err()
    );
    assert!(
        toml::from_str::<Config>("sbx = 'sbx.exe'")
            .unwrap()
            .validate()
            .is_err()
    );
    for cpus in [0, 65] {
        assert!(
            Limits {
                cpus,
                ..Limits::default()
            }
            .validate()
            .is_err()
        );
    }
    assert!(Config::default().validate().is_ok());
}
#[test]
fn ids_are_utc_safe_unique_and_bounded() {
    let now = chrono::Utc.with_ymd_and_hms(2026, 9, 21, 19, 0, 0).unwrap();
    let a = protocol::run_id(Agent::Codex, "Test/Version:1", now);
    let b = protocol::run_id(Agent::Codex, "Test/Version:1", now);
    assert!(a.starts_with("20260921T190000Z--codex--test-version-1--"));
    assert_ne!(a, b);
    assert!(bench::config::parse_component(&a).is_ok());
}
#[test]
fn task_contract_and_prompt_bytes_are_not_rewritten() {
    let repo = repo();
    let task = Task::load(repo.path(), "spaceship-v1").unwrap();
    assert_eq!(
        task.prompt,
        include_bytes!("../../tasks/spaceship-v1/prompt.md")
    );
    assert_eq!(
        task.prompt_sha256,
        archive::hash(include_bytes!("../../tasks/spaceship-v1/prompt.md"))
    );
    assert!(Task::load(repo.path(), "../spaceship-v1").is_err());
    let changed = String::from_utf8(task.contract_bytes)
        .unwrap()
        .replace("columns = 120", "columns = 80");
    fs::write(repo.path().join("tasks/spaceship-v1/task.toml"), changed).unwrap();
    assert!(Task::load(repo.path(), "spaceship-v1").is_err());
}
#[test]
fn freeze_rejects_corrupt_archived_metadata_instead_of_ignoring_it() {
    let repo = repo();
    let task = Task::load(repo.path(), "spaceship-v1").unwrap();
    let store = RunStore::open(repo.path()).unwrap();
    fs::create_dir(store.root.join("20260921T190000Z--bad")).unwrap();
    assert!(store.check_frozen(&task).is_err());
}
#[test]
fn adapters_send_exact_model_once_without_budgets_or_resume() {
    for agent in [Agent::Claude, Agent::Codex] {
        let args = agent.invocation("an-exact-model-id");
        assert_eq!(
            args.iter()
                .filter(|v| v.as_str() == "an-exact-model-id")
                .count(),
            1
        );
        assert!(!args.iter().any(|a| {
            [
                "resume",
                "--max-turns",
                "--max-budget-usd",
                "--fallback-model",
            ]
            .contains(&a.as_str())
        }));
        assert!(
            !args
                .iter()
                .any(|a| a.contains("spaceship") || a.contains("tasteful"))
        );
    }
    assert_eq!(Agent::Codex.invocation("id").last().unwrap(), "-");
}

#[test]
fn adapters_fit_the_backend_argument_contract_and_require_subscription_billing() {
    for agent in [Agent::Claude, Agent::Codex] {
        let args = agent.invocation("exact-model");
        assert!(args.iter().all(|arg| !arg.is_empty()));
        match agent {
            Agent::Claude => assert!(args.iter().any(|arg| arg == "--setting-sources=")),
            Agent::Codex => {
                let overrides = args
                    .windows(2)
                    .filter(|pair| pair[0] == "-c")
                    .map(|pair| pair[1].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                let config: toml::Value = toml::from_str(&overrides).unwrap();
                assert_eq!(config["model_provider"].as_str(), Some("sandboxd"));
                let provider = &config["model_providers"]["sandboxd"];
                assert_eq!(
                    provider["base_url"].as_str(),
                    Some("https://chatgpt.com/backend-api/codex")
                );
                assert_eq!(
                    provider["experimental_bearer_token"].as_str(),
                    Some("oai-oat01-proxy-managed")
                );
                assert_eq!(provider["requires_openai_auth"].as_bool(), Some(false));
                assert!(config.get("forced_login_method").is_none());
                assert!(config["mcp_servers"].as_table().unwrap().is_empty());
            }
        }
    }
}
#[test]
fn normal_completion_requires_a_native_terminal_event_and_zero_exit() {
    let mut events = Events::default();
    events.consume(Agent::Codex, br#"{"type":"turn.completed","usage":{"input_tokens":7,"output_tokens":3,"account":"secret"}}"#);
    let (out, model) = events.finish(Some(0), false, false, false);
    assert_eq!(out.completion, Completion::Normal);
    assert_eq!(out.usage.unwrap()["input_tokens"], 7);
    assert!(model.is_none());
    assert!(out.cost_usd.is_none());
    assert_eq!(
        Events::default()
            .finish(Some(0), false, false, false)
            .0
            .completion,
        Completion::Interrupted
    );
}
#[test]
fn lifecycle_distinguishes_abort_interruption_and_infrastructure() {
    assert_eq!(
        Events::default()
            .finish(None, true, true, false)
            .0
            .completion,
        Completion::UserAbort
    );
    assert_eq!(
        Events::default()
            .finish(None, false, true, false)
            .0
            .completion,
        Completion::InfrastructureFailure
    );
    let mut events = Events::default();
    events.consume(
        Agent::Codex,
        br#"{"type":"turn.failed","error":{"message":"api-key-must-never-be-published"}}"#,
    );
    let out = events.finish(Some(1), false, false, false).0;
    assert_eq!(out.completion, Completion::Interrupted);
    assert!(!serde_json::to_string(&out).unwrap().contains("api-key"));
}
#[test]
fn claude_completion_records_only_reliable_fields() {
    let mut e = Events::default();
    e.consume(
        Agent::Claude,
        br#"{"type":"system","subtype":"init","model":"model-1","session_id":"private-account"}"#,
    );
    e.consume(Agent::Claude, br#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.03,"usage":{"input_tokens":42},"result":"What should I make?"}"#);
    let (out, model) = e.finish(Some(0), false, false, true);
    assert_eq!(out.completion, Completion::Normal); // A question is still normal completion. No answer.
    assert_eq!(model.as_deref(), Some("model-1"));
    assert_eq!(out.cost_usd, Some(0.03));
    assert!(out.transcript_truncated);
    assert!(
        !serde_json::to_string(&out)
            .unwrap()
            .contains("private-account")
    );
}
#[test]
fn malformed_events_are_not_claimed_as_normal() {
    let mut e = Events::default();
    e.consume(Agent::Codex, b"\x1b[31mnot json");
    e.consume(Agent::Codex, br#"{"type":"turn.completed"}"#);
    assert_eq!(
        e.finish(Some(0), false, false, false).0.completion,
        Completion::Interrupted
    );
}
#[test]
fn ordinary_archive_is_staged_hashed_and_excludes_infrastructure() {
    let tmp = tempfile::tempdir().unwrap();
    let bytes = tar(&[
        ("./src/", b'5', b""),
        ("./src/main.rs", b'0', b"source\n"),
        (".git/config", b'0', b"private"),
        ("target/a", b'0', b"binary"),
        (".env", b'0', b"secret"),
    ]);
    let dest = tmp.path().join("solution");
    let files = archive::extract(Cursor::new(bytes), &dest, 10000, 20).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files["src/main.rs"].sha256, archive::hash(b"source\n"));
    assert!(!dest.join(".git").exists());
    assert!(archive::extract(Cursor::new(tar(&[])), &dest, 10000, 20).is_err());
}
#[test]
fn export_rejects_hostile_paths_atomically() {
    for name in [
        "../escape",
        "/abs",
        "a/../../escape",
        "C:/escape",
        "a\\b",
        "a//b",
        "a/./b",
        "a\x1b",
        "a/NUL",
        "a/..",
        "x.",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("solution");
        let bytes = tar(&[("good", b'0', b"good"), (name, b'0', b"bad")]);
        assert!(
            archive::extract(Cursor::new(bytes), &dest, 10000, 20).is_err(),
            "{name}"
        );
        assert!(!dest.exists());
    }
}
#[test]
fn export_rejects_links_devices_fifo_sparse_and_extended_metadata() {
    for kind in *b"123467SxgLK" {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            archive::extract(
                Cursor::new(tar(&[("evil", kind, b"")])),
                &tmp.path().join("out"),
                100,
                20
            )
            .is_err(),
            "kind {kind}"
        );
    }
}
#[test]
fn excluded_entries_are_still_validated() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(
        archive::extract(
            Cursor::new(tar(&[(".git/evil", b'2', b"")])),
            &tmp.path().join("out"),
            100,
            20
        )
        .is_err()
    );
}
#[test]
fn export_rejects_duplicates_case_collisions_and_limits() {
    for entries in [
        vec![
            ("a", b'0', b"data".as_slice()),
            ("a", b'0', b"data".as_slice()),
        ],
        vec![
            ("A", b'0', b"data".as_slice()),
            ("a", b'0', b"data".as_slice()),
        ],
    ] {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            archive::extract(Cursor::new(tar(&entries)), &tmp.path().join("out"), 100, 20).is_err()
        );
    }
    let tmp = tempfile::tempdir().unwrap();
    assert!(
        archive::extract(
            Cursor::new(tar(&[("a", b'0', b"too large")])),
            &tmp.path().join("out"),
            2,
            20
        )
        .is_err()
    );
    assert!(
        archive::extract(
            Cursor::new(tar(&[("a", b'0', b""), ("b", b'0', b"")])),
            &tmp.path().join("out"),
            100,
            1
        )
        .is_err()
    );
}
#[test]
fn export_rejects_special_modes_truncation_and_trailing_payload() {
    let tmp = tempfile::tempdir().unwrap();
    let mut t = tar(&[("a", b'0', b"a")]);
    let mut header = tar::Header::new_ustar();
    header.as_mut_bytes().copy_from_slice(&t[..512]);
    header.set_mode(0o4755);
    header.set_cksum();
    t[..512].copy_from_slice(header.as_bytes());
    assert!(archive::extract(Cursor::new(t), &tmp.path().join("mode"), 100, 10).is_err());
    let mut t = tar(&[("a", b'0', b"a")]);
    t.truncate(512);
    assert!(archive::extract(Cursor::new(t), &tmp.path().join("short"), 100, 10).is_err());
    let mut t = tar(&[("a", b'0', b"a")]);
    t.extend_from_slice(b"hidden payload");
    assert!(archive::extract(Cursor::new(t), &tmp.path().join("tail"), 100, 10).is_err());
}
#[test]
fn missing_lockfile_and_git_dependencies_are_not_repaired() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname='x'\nversion='0.1.0'\n",
    )
    .unwrap();
    assert!(replay::validate_submission(tmp.path()).is_err());
    assert!(!tmp.path().join("Cargo.lock").exists());
    fs::write(
        tmp.path().join("Cargo.lock"),
        "version=4\n[[package]]\nname='x'\nversion='0.1.0'\nsource='git+https://example.com/x'\n",
    )
    .unwrap();
    assert!(replay::validate_submission(tmp.path()).is_err());
}
#[test]
fn fixture_validates_without_running_cargo_on_the_host() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny");
    replay::validate_submission(&fixture).unwrap();
}
#[test]
fn vendoring_configuration_is_restricted_to_exact_sources() {
    let good = b"[source.crates-io]\nreplace-with='vendored-sources'\n[source.vendored-sources]\ndirectory='/replay/vendor'\n";
    replay::validate_vendor_config(good).unwrap();
    replay::validate_vendor_config(b"").unwrap();
    let mut evil = good.to_vec();
    evil.extend_from_slice(b"[build]\nrustc-wrapper='/tmp/evil'\n");
    assert!(replay::validate_vendor_config(&evil).is_err());
}
#[cfg(unix)]
#[test]
fn local_artifacts_reject_symlinks_and_hardlinks() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("file"), "a").unwrap();
    symlink("file", tmp.path().join("sym")).unwrap();
    assert!(archive::inventory(tmp.path(), 100, 100).is_err());
    fs::remove_file(tmp.path().join("sym")).unwrap();
    fs::hard_link(tmp.path().join("file"), tmp.path().join("hard")).unwrap();
    assert!(archive::inventory(tmp.path(), 100, 100).is_err());
}
#[test]
fn docker_snapshot_layers_can_be_selected_without_host_execution() {
    let tmp = tempfile::tempdir().unwrap();
    let first = tar(&[
        ("workspace/src/main.rs", b'0', b"old"),
        ("workspace/deleted", b'0', b"gone"),
        ("home/agent/auth.json", b'0', b"not exported"),
    ]);
    let second = tar(&[
        ("workspace/src/main.rs", b'0', b"new"),
        ("workspace/.wh.deleted", b'0', b""),
    ]);
    let manifest = br#"[{"Layers":["first.tar","second.tar"]}]"#;
    let image = tar(&[
        ("manifest.json", b'0', manifest),
        ("first.tar", b'0', &first),
        ("second.tar", b'0', &second),
    ]);
    let path = tmp.path().join("image.tar");
    fs::write(&path, image).unwrap();
    let dest = tmp.path().join("solution");
    let files =
        archive::extract_image_workspace(&path, "workspace", &dest, 100000, 1000, 100).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(fs::read(dest.join("src/main.rs")).unwrap(), b"new");
}

#[test]
fn empty_directories_survive_pack_export_and_hashing() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    fs::create_dir_all(source.join("assets/empty")).unwrap();
    let packed = tmp.path().join("source.tar");
    archive::pack(&source, &packed, 100, 20).unwrap();
    let dest = tmp.path().join("dest");
    let records = archive::extract(fs::File::open(packed).unwrap(), &dest, 100, 20).unwrap();
    assert!(dest.join("assets/empty").is_dir());
    assert!(records["assets/empty"].directory);
    assert_eq!(records, archive::inventory(&source, 100, 20).unwrap());
}

#[test]
fn opaque_whiteouts_do_not_delete_new_entries_when_listed_last() {
    let tmp = tempfile::tempdir().unwrap();
    let lower = tar(&[("workspace/assets/old", b'0', b"old")]);
    let upper = tar(&[
        ("workspace/assets/new", b'0', b"new"),
        ("workspace/assets/.wh..wh..opq", b'0', b""),
    ]);
    let manifest = br#"[{"Layers":["lower.tar","upper.tar"]}]"#;
    let image = tar(&[
        ("manifest.json", b'0', manifest),
        ("lower.tar", b'0', &lower),
        ("upper.tar", b'0', &upper),
    ]);
    let path = tmp.path().join("image.tar");
    fs::write(&path, image).unwrap();
    let dest = tmp.path().join("solution");
    archive::extract_image_workspace(&path, "workspace", &dest, 100000, 1000, 100).unwrap();
    assert!(dest.join("assets/new").is_file());
    assert!(!dest.join("assets/old").exists());
}
