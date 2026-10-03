# Spaceship taste bench

A chronological showcase of individual coding-agent attempts. Each agent gets the current task prompt, archived exactly for that attempt, creates a Rust terminal spaceship game, and leaves an unedited submission. A person plays it and writes a subjective review. There are no scores, evaluator models, rankings, repeated-trial machinery, or generated reviews.

**Status: no runs recorded.** The harness supports Codex and Claude Code subscription attempts in Docker pilot mode. The pilot records the accepted gateway/configuration/storage limitations; strict attempts remain blocked by certification requirements. Docker Sandboxes `sbx` 0.45.0 is the sole backend. Follow the [pilot workflow](docs/PILOT.md) for authentication, readiness checks and the first smoke task. See [verification status](docs/VERIFICATION.md) for harness and environment checks.

The development host is Ubuntu 24.04 under WSL2, x86-64, with Rust 1.97.0, Docker Engine 29.8.1, Buildx 0.37.1, `sbx` 0.45.0, KVM access, Docker sign-in and a deny-all baseline. The `linux-rust` environment pins Claude Code 2.1.280 and Codex 0.155.1. The default task is `spaceship`; `smoke` provides a small end-to-end check.

Tasks, environments and configs use stable names and are updated in place. Keep one current implementation of each; Git preserves code history, and run directories preserve each attempt's exact inputs.

## Install and check

Run from this repository root:

```sh
rustup toolchain install 1.97.0 --profile minimal --component rustfmt --component clippy
cargo +1.97.0 install --path harness --locked
bench doctor
```

`doctor` reports readiness for the selected mode. It exits 1 while a required check is blocked. Use `bench doctor --agent codex --json` for structured output. Pilot mode verifies an available account in a disposable guest, including its network policy; it never calls a model, logs in, changes global policy, or copies your normal agent configuration. The CLI also accepts `--mode strict|docker-pilot`, `--repo /path/to/checkout` and `--config /path/to/config.toml`.

To work on the harness:

```sh
cargo +1.97.0 fmt --manifest-path harness/Cargo.toml -- --check
cargo +1.97.0 clippy --manifest-path harness/Cargo.toml --all-targets --locked -- -D warnings
cargo +1.97.0 test --manifest-path harness/Cargo.toml --locked
python3 -m unittest discover -s environments/linux-rust -p 'test_*.py'
```

Only the harness is compiled on the host. Submissions and their Cargo build scripts must run inside an external microVM.

## Supported-host status

| Host | Status |
| --- | --- |
| Ubuntu 24.04+ amd64 with usable KVM | Enabled for the Docker pilot; strict certification remains blocked |
| WSL2 Ubuntu amd64 | Linux-local creation/copy/export/PTY transport verified; supports the same pilot mode |
| Fedora / other Linux | Rust CLI may build; backend support and this integration are unverified, so no official runs |
| Apple silicon, macOS 14+ | Vendor-supported backend; this resolved amd64 environment and harness transport are not enabled there |
| Intel macOS / native Windows | Unsupported by this harness |
| Windows-hosted backend invoked from WSL | Unsupported; `sbx.exe`, named pipes, Windows file paths and ConPTY are not treated as Linux-local `sbx` |

[Docker's installation requirements](https://docs.docker.com/ai/sandboxes/install/) are separate from whether Rust compiles on a platform. The harness never falls back to ordinary Docker containers or host execution.

## Setup, images and authentication

Follow [SETUP.md](docs/SETUP.md), then copy `bench.example.toml` to ignored `bench.local.toml` to adjust machine-protection limits. Unknown settings, including time/token budget keys, are rejected.

Prepare images without starting an attempt:

```sh
python3 environments/linux-rust/prepare.py
```

This requires an independently installed Docker image builder and local `sbx` 0.45.0. It builds the trusted Dockerfiles directly from pinned upstream images, checks both agent package integrities, and verifies fresh built-in runtime startup/version before recording actual image IDs. Both agents inherit the same base layers. The current lock is already resolved; load the saved `build/*.tar` bundles to reuse those images. For an intentional update, edit the existing environment and reviewed CLI pins, then run `prepare.py --rebuild`. Retain any image bundles needed by archived attempts before rebuilding. Image preparation makes no model request.

The requested authentication commands are:

```sh
bench auth codex
bench auth claude
```

`auth codex` delegates to the documented host-side `sbx secret set openai --oauth` flow. `auth claude` opens Claude's browser login in a disposable mountless guest, destroys it, and checks subscription reuse in another fresh guest. Run authentication in your own terminal; its output is not logged. Neither command copies your normal agent home or silently changes to API billing. [Credential details and alternatives](docs/SETUP.md#authentication).

## First pilot

Follow [PILOT.md](docs/PILOT.md). Select `mode = "docker-pilot"` in ignored
`bench.local.toml`, authenticate with `bench auth codex`, and check readiness with
`bench doctor --agent codex`. The separate `smoke` task can exercise a real request and replay
before using your game prompt. Pilot records explicitly state their accepted
limitations and never claim strict isolation certification.

For an Opus 5.5 attempt, use `bench auth claude`, then `bench doctor --agent claude`
and `bench run --agent claude --model claude-opus-5-5 --task smoke`.
On a later UTC date, omit `--task smoke` to use the spaceship prompt. Smoke
and game attempts share the one-run-per-agent/model/date limit.

## Strict attempts, once certification is complete

Edit `tasks/spaceship/prompt.md` and `task.toml` in place. Allocation snapshots the selected task and environment lock for that attempt. Later edits apply to future attempts; existing archives remain unchanged and checksum-verified. Prompt bytes, including the final newline, are delivered exactly once through stdin.

In strict mode, after the blockers in [VERIFICATION.md](docs/VERIFICATION.md) are resolved:

```sh
bench run --agent claude --model '<exact-model-id>'
bench run --agent codex --model '<exact-model-id>'
```

Strict-mode commands currently fail **before prompt delivery**. The separate Docker pilot changes the recorded protocol rather than certifying strict isolation. There is no host/container fallback. Failed prerequisite checks are not attempts. A failure after allocation retains its run directory and reserves that agent/model/date; a retry with the same agent/model must use a later UTC date.

Once a task finishes, the controller stops the guest, exports validated source, destroys the guest, and attempts locked dependency packaging in another guest. Normal completion can produce a broken game. Packaging does not fix it. A question in the final response receives no answer. [Full protocol](PROTOCOL.md).

Source exports allow up to 512 MiB, including dependencies bundled by the agent. If an export fails, its stopped snapshot and `export-error.log` remain in private state for diagnosis; cleanup still removes the guest and temporary template.

Recovery also selects the archived environment. After correcting an export problem, recover the same stopped snapshot with `bench recover '<run-id>'`. This makes no model request and preserves the previous run metadata and snapshot checksum in `export-recovery.json`. It refuses to overwrite an existing solution and prepares locked dependencies from the recovered source.

## Play and review

With a prepared replay bundle, resize your actual terminal to the dimensions in the run's archived `task.toml` (**124 columns × 69 rows** for `spaceship`, 120×40 for `smoke`), then:

```sh
bench play '<run-id>'
```

The play controller selects the archived environment and verifies hashes, requests a fresh shell guest, checks the selected boundary, supplies source and vendored dependencies separately, verifies the PTY size, and runs `cargo --config /replay/config.toml run --release --frozen`. Strict playback remains blocked by backend-injected services. Pilot playback accepts and records those services while enforcing guest deny-all networking and frozen dependencies. A `TERM` value alone is not accepted as evidence of dimensions.

Write your own `runs/<run-id>/review.md` using the included questions. Screenshots and clips go in `media/`; large media are ignored by default. Build/launch status is operational information, not a game-quality score.

## Artifacts

```text
runs/<agent>-<model-slug>-YYYY-MM-DD/
  prompt.md                 exact prompt snapshot
  task.toml                 exact task contract snapshot
  settings.json             non-secret input settings
  environment.lock.json     environment input snapshot, when present
  run.json                  controller-owned metadata and checksums
  solution/                 validated source; never repaired
  replay.json               independent preparation/play status and bundle checksum
  export-recovery.json      prior metadata and snapshot checksum, when recovered
  review.md                 manual review template
  media/
```

The date is UTC. For example: `codex-gpt-6-astra-2026-09-24`. Model slugs are
lowercase, use single hyphens between alphanumeric segments, and are capped at
64 characters with trailing hyphens removed. There are no double hyphens or
random suffixes. An existing directory with the same name is never overwritten;
another attempt with that agent/model slug on the same UTC date is rejected,
including smoke tasks and retries.

Solutions are independent Cargo projects, not members of a repository-wide workspace. `solution/` is made read-only and checked against its inventory before playback. Git does not preserve read-only permissions, so checksum verification remains necessary after a clone. Empty directories are preserved in local exports/replay but Git itself does not track empty directories.

Private state defaults to `$XDG_STATE_HOME/spaceship-taste-bench`, or `~/.local/state/spaceship-taste-bench`. `BENCH_STATE_DIR` can select a dedicated directory outside the checkout. It contains `raw/<run-id>/stdout.jsonl`, `stderr.log`, cleanup journals, temporary full snapshots, and `bundles/*.tar`. Backend administrative diagnostics are retained under private `backend/` directories, with 64 KiB per stdout/stderr stream per operation; attempt setup and packaging logs stay under `raw/<run-id>/`. Authentication flows are not logged. Logs use mode 0600; the state directory uses 0700. Transcript size is bounded; truncation is recorded without ending an attempt. Nothing there is automatically published. Bundle references in `replay.json` are basenames relative to the private `bundles/` directory.

## Troubleshooting

- `doctor` reports missing `sbx`: install the **standalone** 0.45.0 CLI/backend from Docker's documented distribution. Legacy `docker sandbox` is a different interface.
- WSL cannot open `/dev/kvm`: nested virtualization and group/device permissions are administrator setup; a Windows Docker installation does not solve the Linux-local transport automatically.
- `sbx diagnose` passes but guest creation reports an uninitialized network policy: on a fresh installation, run `sbx policy init deny-all` yourself. This is a global sandbox policy; do not reset an existing setup. Per-guest effective rules still require verification.
- `check-integration` prints transport passes and then fails on MCP/credential bindings: this is the observed backend limitation. `bench check-runtimes` separately checks each built-in agent image without a model call. It runs only fixed trusted fixtures and returns 1 when any required isolation property fails.
- `required isolation is not established`: follow the engineering checklist in [VERIFICATION.md](docs/VERIFICATION.md). There is intentionally no configuration flag to suppress it.
- Image identities are null: image preparation has not completed. Preserve resolved image tar files outside Git, not just their tags.
- Archived input checksum mismatch: restore the original archived bytes. Make future task/environment changes in their existing source directories.
- Missing lockfile, non-crates.io dependency, or failed vendoring: retain the submission and failure metadata. Do not run `cargo generate-lockfile` or repair it.
- Cleanup interrupted by a crash or SIGKILL: inspect private `raw/<run-id>/guest.json`, then `sbx rm --force <exact-recorded-guest-name>`. Never use broad `--all`/`reset` cleanup. SIGKILL/power loss cannot be handled by a userspace destructor.

Read [SECURITY.md](SECURITY.md) before sharing source or any manually sanitized transcript. The record is historical and subjective, not a statistically conclusive comparison of model ability.
