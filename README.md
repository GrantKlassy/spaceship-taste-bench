# Spaceship taste bench

A chronological showcase of individual coding-agent attempts. Each agent gets the same frozen prompt, creates a Rust terminal spaceship game, and leaves an unedited submission. A person plays it and writes a subjective review. There are no scores, evaluator models, rankings, repeated-trial machinery, or generated reviews.

**Status: no runs recorded.** The harness supports Codex and Claude Code subscription attempts in Docker pilot mode. The pilot records the accepted gateway/configuration/storage limitations; strict attempts remain blocked by certification requirements. Docker Sandboxes `sbx` 0.45.0 is the sole backend. Follow the [pilot workflow](docs/PILOT.md) for authentication, readiness checks and the first smoke task. See [verification status](docs/VERIFICATION.md) for harness and environment checks.

The development host is Ubuntu 24.04 under WSL2, x86-64, with Rust 1.97.0, Docker Engine 29.8.1, Buildx 0.37.1, `sbx` 0.45.0, KVM access, Docker sign-in and a deny-all baseline. The default environment is `linux-rust-v3`: it preserves v2's Linux/Rust packages and Codex 0.155.1 runtime, and upgrades Claude Code to 2.1.280 for Opus 5.5. V1/v2 images remain preserved. The default task is `spaceship-v2`, whose prompt is byte-for-byte identical to `spaceship-v1`; its contract selects v3.

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
python3 environments/linux-rust-v3/prepare.py
```

This requires an independently installed Docker image builder and local `sbx` 0.45.0. It builds only trusted environment Dockerfiles from the preserved v2 images, loads their saved images into the sandbox store, and checks fresh built-in runtime startup/version before recording actual image IDs. It does not run a task, publish images, or authenticate. The lock now records the real amd64 images built on the development machine; their tar bundles remain local and ignored by Git. Both agent images inherit the same verified base layers. Preparation is refused after an attempt using that environment or after its image identities have been resolved; transfer preserved image bundles to another machine, or prepare a new environment version.

The requested authentication commands are:

```sh
bench auth codex
bench auth claude
```

`auth codex` delegates to the documented host-side `sbx secret set openai --oauth` flow. `auth claude` opens Claude's browser login in a disposable mountless guest, destroys it, and checks subscription reuse in another fresh guest. Run authentication in your own terminal; its output is not logged. Neither command copies your normal agent home or silently changes to API billing. [Credential details and alternatives](docs/SETUP.md#authentication).

## First pilot

Follow [PILOT.md](docs/PILOT.md). Select `mode = "docker-pilot"` in ignored
`bench.local.toml`, authenticate with `bench auth codex`, and check readiness with
`bench doctor --agent codex`. The separate `smoke-v2` task can exercise a real request and replay
without freezing your game prompt. Pilot records explicitly state their accepted
limitations and never claim strict isolation certification.

For an Opus 5.5 attempt, use `bench auth claude`, then `bench doctor --agent claude`
and `bench run --agent claude --model claude-opus-5-5 --task smoke-v2`.
After the smoke test, omit `--task smoke-v2` to use the spaceship prompt.

## Strict attempts, once certification is complete

The spaceship prompts and task contracts can be edited before their first attempt. Allocation freezes the selected task version. Prompt bytes, including the final newline, are delivered exactly once through stdin. Once frozen, make a new version for changes:

```sh
cp -r tasks/spaceship-v2 tasks/spaceship-v3
# Edit spaceship-v3/prompt.md and change version in spaceship-v3/task.toml.
```

In strict mode, after the blockers in [VERIFICATION.md](docs/VERIFICATION.md) are resolved:

```sh
bench run --agent claude --model '<exact-model-id>'
bench run --agent codex --model '<exact-model-id>'
# Optional alternative prompt version:
bench run --agent codex --model '<exact-model-id>' --task spaceship-v3
```

Strict-mode commands currently fail **before prompt delivery**. The separate Docker pilot changes the recorded protocol rather than certifying strict isolation. There is no host/container fallback. Failed prerequisite checks are not attempts. A failure after allocation retains a distinct run directory; an explicitly started retry gets a new ID.

Once a task finishes, the controller stops the guest, exports validated source, destroys the guest, and attempts locked dependency packaging in another guest. Normal completion can produce a broken game. Packaging does not fix it. A question in the final response receives no answer. [Full protocol](PROTOCOL.md).

Source exports allow up to 512 MiB, including dependencies bundled by the agent. If an export fails, its stopped snapshot and `export-error.log` remain in private state for diagnosis; cleanup still removes the guest and temporary template.

Recovery also selects the archived environment. After correcting an export problem, recover the same stopped snapshot with `bench recover '<run-id>'`. This makes no model request and preserves the previous run metadata and snapshot checksum in `export-recovery.json`. It refuses to overwrite an existing solution and prepares locked dependencies from the recovered source.

## Play and review

With a prepared replay bundle, resize your actual terminal to the dimensions in the run's archived `task.toml` (**124 columns × 69 rows** for both spaceship task versions, 120×40 for both smoke task versions), then:

```sh
bench play '<run-id>'
```

The play controller selects the archived environment and verifies hashes, requests a fresh shell guest, checks the selected boundary, supplies source and vendored dependencies separately, verifies the PTY size, and runs `cargo --config /replay/config.toml run --release --frozen`. Strict playback remains blocked by backend-injected services. Pilot playback accepts and records those services while enforcing guest deny-all networking and frozen dependencies. A `TERM` value alone is not accepted as evidence of dimensions.

Write your own `runs/<run-id>/review.md` using the included questions. Screenshots and clips go in `media/`; large media are ignored by default. Build/launch status is operational information, not a game-quality score.

## Artifacts

```text
runs/<UTC>--<agent>--<model-slug>--<random-64-bit-suffix>/
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

Solutions are independent Cargo projects, not members of a repository-wide workspace. `solution/` is made read-only and checked against its inventory before playback. Git does not preserve read-only permissions, so checksum verification remains necessary after a clone. Empty directories are preserved in local exports/replay but Git itself does not track empty directories.

Private state defaults to `$XDG_STATE_HOME/spaceship-taste-bench`, or `~/.local/state/spaceship-taste-bench`. `BENCH_STATE_DIR` can select a dedicated directory outside the checkout. It contains `raw/<run-id>/stdout.jsonl`, `stderr.log`, cleanup journals, temporary full snapshots, and `bundles/*.tar`. Backend administrative diagnostics are retained under private `backend/` directories, with 64 KiB per stdout/stderr stream per operation; attempt setup and packaging logs stay under `raw/<run-id>/`. Authentication flows are not logged. Logs use mode 0600; the state directory uses 0700. Transcript size is bounded; truncation is recorded without ending an attempt. Nothing there is automatically published. Bundle references in `replay.json` are basenames relative to the private `bundles/` directory.

## Troubleshooting

- `doctor` reports missing `sbx`: install the **standalone** 0.45.0 CLI/backend from Docker's documented distribution. Legacy `docker sandbox` is a different interface.
- WSL cannot open `/dev/kvm`: nested virtualization and group/device permissions are administrator setup; a Windows Docker installation does not solve the Linux-local transport automatically.
- `sbx diagnose` passes but guest creation reports an uninitialized network policy: on a fresh installation, run `sbx policy init deny-all` yourself. This is a global sandbox policy; do not reset an existing setup. Per-guest effective rules still require verification.
- `check-integration` prints transport passes and then fails on MCP/credential bindings: this is the observed backend limitation. `bench check-runtimes` separately checks each built-in agent image without a model call. It runs only fixed trusted fixtures and returns 1 when any required isolation property fails.
- `required isolation is not established`: follow the engineering checklist in [VERIFICATION.md](docs/VERIFICATION.md). There is intentionally no configuration flag to suppress it.
- Image identities are null: image preparation has not completed. Preserve resolved image tar files outside Git, not just their tags.
- Frozen task/environment mismatch: use a new version; do not rewrite history.
- Missing lockfile, non-crates.io dependency, or failed vendoring: retain the submission and failure metadata. Do not run `cargo generate-lockfile` or repair it.
- Cleanup interrupted by a crash or SIGKILL: inspect private `raw/<run-id>/guest.json`, then `sbx rm --force <exact-recorded-guest-name>`. Never use broad `--all`/`reset` cleanup. SIGKILL/power loss cannot be handled by a userspace destructor.

Read [SECURITY.md](SECURITY.md) before sharing source or any manually sanitized transcript. The record is historical and subjective, not a statistically conclusive comparison of model ability.
