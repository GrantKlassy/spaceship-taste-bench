# Spaceship taste bench

A chronological showcase of individual coding-agent attempts. Each agent gets the same frozen prompt, creates a Rust terminal spaceship game, and leaves an unedited submission. A person plays it and writes a subjective review. There are no scores, evaluator models, rankings, repeated-trial machinery, or generated reviews.

**Status: trusted fixture replay and agent startup work; official attempts remain blocked by backend isolation and authentication integration.** Docker Sandboxes `sbx` 0.45.0 is the sole backend. SSH forwarding has been disabled on this machine and fresh guests have no SSH-agent socket. MCP gateway access, unrelated credential bindings, managed agent configuration and host snapshot/cache bounds remain unresolved. No model request or official attempt has been made. See [verification status](docs/VERIFICATION.md).

The development host is Ubuntu 24.04 under WSL2, x86-64, with Rust 1.97.0, Docker Engine 29.8.1, Buildx 0.37.1, `sbx` 0.45.0, KVM access, Docker sign-in and a deny-all baseline. The default environment is now `linux-rust-v2`: it preserves v1's packages and fixes Claude startup permissions and agent image flavor labels. The original v1 images remain preserved.

## Install and check

Run from this repository root:

```sh
rustup toolchain install 1.97.0 --profile minimal --component rustfmt --component clippy
cargo +1.97.0 install --path harness --locked
bench doctor
```

`doctor` reports measured host/settings/image/account checks separately from unresolved integration requirements. It exits 1 while any required check is blocked. Use `bench doctor --agent codex --json` for a structured report scoped to the intended agent. It never calls a model, logs in, initializes network policy, or copies normal agent configuration. The CLI also accepts `--repo /path/to/checkout` and `--config /path/to/config.toml`.

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
| Ubuntu 24.04+ amd64 with usable KVM | Enabled for trusted diagnostics; official attempts blocked by the service-isolation failures |
| WSL2 Ubuntu amd64 | Linux-local creation/copy/export/PTY transport verified on the development machine; the same isolation failures apply |
| Fedora / other Linux | Rust CLI may build; backend support and this integration are unverified, so no official runs |
| Apple silicon, macOS 14+ | Vendor-supported backend; this resolved amd64 environment and harness transport are not enabled there |
| Intel macOS / native Windows | Unsupported by this harness |
| Windows-hosted backend invoked from WSL | Unsupported; `sbx.exe`, named pipes, Windows file paths and ConPTY are not treated as Linux-local `sbx` |

[Docker's installation requirements](https://docs.docker.com/ai/sandboxes/install/) are separate from whether Rust compiles on a platform. The harness never falls back to ordinary Docker containers or host execution.

## Setup, images and authentication

Follow [SETUP.md](docs/SETUP.md), then copy `bench.example.toml` to ignored `bench.local.toml` to adjust machine-protection limits. Unknown settings, including time/token budget keys, are rejected.

Prepare images without starting an attempt:

```sh
python3 environments/linux-rust-v2/prepare.py
```

This requires an independently installed Docker image builder and local `sbx` 0.45.0. It builds only trusted environment Dockerfiles from the preserved v1 images, loads their saved images into the sandbox store, and checks fresh built-in runtime startup/version before recording actual image IDs. It does not run a task, publish images, or authenticate. The lock now records the real amd64 images built on the development machine; their tar bundles remain local and ignored by Git. Both agent images inherit the same verified base layers. Preparation is refused after any archived attempt or after the environment has been resolved; transfer preserved image bundles to another machine, or prepare a new environment version.

The requested authentication commands are:

```sh
bench auth codex
bench auth claude
```

`auth codex` delegates to the documented host-side `sbx secret set openai --oauth` flow when that CLI is installed. `auth claude` currently fails with the documented subscription setup and the unresolved fresh-guest broker limitation. Neither copies your normal agent home or silently changes to API billing. No provider authentication was performed during development. [Credential details and alternatives](docs/SETUP.md#authentication).

## First attempt, once certification is complete

You may edit `tasks/spaceship-v1/prompt.md` before its first allocated attempt. Its bytes, including the final newline, are delivered exactly once through stdin. The task contract is frozen too. Once there is an archived attempt, make a new version for changes:

```sh
cp -r tasks/spaceship-v1 tasks/spaceship-v2
# Edit spaceship-v2/prompt.md and change version in spaceship-v2/task.toml.
```

After the blockers in [VERIFICATION.md](docs/VERIFICATION.md) are resolved:

```sh
bench run --agent claude --model '<exact-model-id>'
bench run --agent codex --model '<exact-model-id>'
# Optional alternative prompt version:
bench run --agent codex --model '<exact-model-id>' --task spaceship-v2
```

These commands currently fail **before prompt delivery**. There is no `--unsafe`, mock-backend, fallback, or certification-bypass switch. Failed prerequisite checks are not attempts. A failure after allocation retains a distinct run directory; an explicitly started retry gets a new ID.

Once a task finishes, the controller stops the guest, exports validated source, destroys the guest, and attempts locked dependency packaging in another guest. Normal completion can produce a broken game. Packaging does not fix it. A question in the final response receives no answer. [Full protocol](PROTOCOL.md).

## Play and review

With a prepared replay bundle, resize your actual terminal to **120 columns × 40 rows**, then:

```sh
bench play '<run-id>'
```

The play controller verifies hashes, requests a fresh agent-free guest, checks isolation, supplies source and vendored dependencies separately, verifies the PTY size, and runs `cargo --config /replay/config.toml run --release --frozen`. A trusted fixture with an exact crates.io dependency completed locked vendoring, export/package and fresh-VM offline replay. PTY dimensions, Ctrl-C and terminal restoration passed live checks. Production play still fails before importing a submission because backend-injected services violate its credential-free, offline contract. A `TERM` value alone is not accepted as evidence of dimensions.

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
