# Local setup

The development machine has the required toolchain and images. The default `linux-rust` environment pins Claude Code 2.1.280 and Codex 0.155.1. Follow [PILOT.md](PILOT.md) for subscription authentication, readiness checks and the first smoke task. Strict attempts remain blocked by their backend isolation requirements. See [VERIFICATION.md](VERIFICATION.md) for harness and environment checks and accepted pilot limitations.

## Current machine

Checked again on 2026-09-22:

- Ubuntu 24.04.4 LTS, WSL2, amd64; Linux-local backend and Docker Unix socket.
- Rust/Cargo 1.97.0, Docker Engine 29.8.1, Buildx 0.37.1, standalone `sbx` 0.45.0.
- Working `docker`/`kvm` group access; KVM API 12; Docker sign-in and deny-all baseline initialized by the user.
- All 13 sbx diagnostics have passed. Some checks return 12 passes and an optional update-lookup warning; the installed CLI remains 0.45.0. Base, Claude 2.1.278 and Codex 0.155.1 images are built, loaded and recorded in `environment.lock.json`.
- Real guest creation, trusted file copying, stopped export, dependency connectivity, 120×40 PTY/Ctrl-C and clean-VM fixture replay work.
- SSH forwarding was disabled during the 2026-09-22 maintenance pass after verifying there were no existing sandboxes. Fresh shell, Claude and Codex guests have no SSH-agent socket. MCP services and unrelated provider bindings remain attached; strict mode rejects them and the accepted pilot records them. Broker authentication does not copy a normal agent home.

## Prerequisites on another machine

Use [Docker's installation instructions](https://docs.docker.com/ai/sandboxes/install/) for the **local** backend. The inspected CLI contract is [sbx 0.45.0](https://github.com/docker/sbx-releases/releases/tag/v0.45.0); other versions require review before this harness accepts them.

The enabled harness host is Ubuntu 24.04+ amd64 with read/write KVM access. Virtualized hosts require nested virtualization. Docker documents an Apple-silicon macOS backend too, but this resolved environment and harness transport are not enabled there. Fedora, other distributions, native Windows and other architectures fail with instructions; Rust compilation alone does not establish backend support.

For WSL, install Linux binaries **inside the same distribution** and reach its Linux-local daemon. File copying and terminal transport have now been exercised on that path. A Windows `sbx.exe`, named-pipe connection, Windows credential store or ConPTY bridge is a different, unsupported integration.

Docker's convenience installer can display a WSL recommendation for Docker Desktop and pause for 20 seconds before continuing. That advisory does not establish the transport this project needs. Follow the vendor's Linux installation and KVM requirements; the harness never installs privileged components or changes groups/security settings. Docker Engine/Buildx are used separately to build trusted images.

On a fresh installation, the user performs the account/global policy steps:

```sh
sbx daemon start --detach
sbx login
sbx policy init deny-all
sbx diagnose
```

Do not reset an existing policy or remove unrelated resources. `policy init` sets a global baseline, and kits can still add allowances. The harness adds and inspects its own per-guest rules. Docker sign-in is distinct from Claude/OpenAI subscription authentication. Use `bench doctor` for non-billable diagnostics; it never launches a model or initializes global policy.

Install the harness from the repository root:

```sh
rustup toolchain install 1.97.0 --profile minimal --component rustfmt --component clippy
cargo +1.97.0 install --path harness --locked --force
bench doctor
```

Host Python 3 is needed for image preparation and the opt-in PTY diagnostic driver. Game compilation and Cargo build scripts never run on the host.

## Trusted images

Before an environment is first resolved:

```sh
python3 environments/linux-rust/prepare.py
```

The script supplies only the three reviewed Dockerfiles as build context. The base builds directly from pinned upstream Rust and shell images. Both agent images inherit that base, verify their pinned npm integrity, and install Claude Code 2.1.280 or Codex 0.155.1. Fresh built-in runtime startup is verified before recording the lock. Claude home-directory ownership and agent flavor labels are set in the current Dockerfiles. No older local image is needed. The context never includes the repository, runs, home, credentials or instructions. Both agents share the Linux/Rust/native terminal toolchain and neutral `/workspace`; no crate choice, game architecture or starter source is included.

Base/index/platform digests and agent package pins were resolved from public registries. Native OS packages are resolved during the first trusted build; `/opt/bench/native-packages.tsv` records their exact versions. The lock records the complete built image identity for each image. Initial package resolution is not claimed reproducible. Docker 29's image IDs can identify OCI indexes rather than image configurations; the lock records actual returned identities.

**The environment lock is already resolved.** Docker images use
`terminal-game-taste-bench-{base,claude,codex}:linux-rust` tags. A fresh clone
contains the lock, but not the ignored archives in `environments/linux-rust/build/`.
Inspect `docker image ls` and `sbx template ls` for existing images before building.
When the images are already loaded in `sbx` with the lock's IDs, no preparation
is needed. To save matching Docker images locally and load them into `sbx`:

```sh
mkdir -p environments/linux-rust/build
for kind in base claude codex; do
  docker image save --output "environments/linux-rust/build/$kind.tar" \
    "terminal-game-taste-bench-$kind:linux-rust"
  chmod 600 "environments/linux-rust/build/$kind.tar"
done
```

Use these commands to load preserved archives, including after transferring them
outside Git to another compatible host:

```sh
sbx template load environments/linux-rust/build/base.tar
sbx template load environments/linux-rust/build/claude.tar
sbx template load environments/linux-rust/build/codex.tar
```

No image/artifact has been uploaded. If neither matching images nor preserved archives are available, or for an intentional update, run `python3 environments/linux-rust/prepare.py --rebuild` with the reviewed Dockerfiles and CLI pins. The script updates the same lock after all candidate runtimes pass. Keep any bundles needed by existing runs before rebuilding: playback and recovery use their archived locks and refuse different image identities. A new build is recorded as new input to future attempts, without creating another environment directory.

The default environment is `linux-rust`. Runs require an explicit task such as `--task spaceship` or `--task smoke`; each task has its own prompt and terminal contract. Playback and recovery select the environment archived with the run. Copy `bench.example.toml` to ignored `bench.local.toml` to adjust machine-protection limits. These are CPU, memory, disk, export/replay and log safety settings, never benchmark time/token/cost budgets. `disk_mib` covers the root disk plus writable runtime volumes. Claude reserves 4096 MiB for its five runtime volumes and requires at least 6144 MiB total; the default 20480 MiB gives it a 16384 MiB root disk. Codex and shell guests use the full allocation for their root disk.

Private state lives at `$XDG_STATE_HOME/terminal-game-taste-bench`, falling back
to `~/.local/state/terminal-game-taste-bench`. `BENCH_STATE_DIR` overrides the
whole path. The directory must be dedicated to the benchmark and outside the
checkout; its `.bench-state-v1` marker contains `terminal-game-taste-bench state v1`
followed by a newline. Raw logs, retained snapshots and replay bundles stay there
across clones. Existing run metadata and input snapshots must retain their exact
bytes; bundle references are relative to this state directory.

## Authentication

Subscriptions remain the intended mode. Authentication is an explicit user action before a task; the game prompt is not used to test it. The accepted Docker pilot can use the supported broker sign-in now; strict mode still requires isolation certification.

### Codex

`bench auth codex` delegates to the documented host-broker OAuth flow:

```sh
bench auth codex
# Equivalent backend flow: sbx secret set openai --oauth
```

The backend's browser/keychain flow is intended to keep the real token outside the VM. Fresh-guest sentinels, refresh, credential scoping and required destinations still need live verification. Docker documents API-key precedence over OAuth; the harness must identify this before any attempt instead of silently switching billing. The auth command does not remove an existing API key.

The adapter now supplies the backend's subscription provider explicitly even with `--ignore-user-config`. `bench check-runtimes --agent codex` checks the fresh guest's OAuth mode and placeholder auth file without a model request. Docker routes its subscription broker through a custom `sandboxd` provider; native `forced_login_method="chatgpt"` is not compatible with the broker's placeholder auth file. A missing account or API-key mode remains blocked. Successful login alone still does not make an official attempt available.

Native `codex login` / `codex login --device-auth` are supported standalone flows, but normal host Codex configuration is not this harness's transfer mechanism. Never copy a normal Codex home or `auth.json` into a run.

### Claude Code

Run `bench auth claude` in your terminal. It creates a disposable mountless guest
from the pinned Claude image and runs `claude auth login --claudeai`; follow the
browser link to sign in with your Claude subscription. It makes no model request
and does not log authentication input or output. The login guest is destroyed,
then a second fresh guest must report OAuth mode, broker placeholder tokens and
a native Claude subscription login. No host or guest agent home is copied.

An existing Anthropic API-key broker entry blocks this flow because Docker gives
API keys precedence. The harness never removes that entry automatically. Local
`sbx` 0.45.0 supports `secret set --oauth` only for OpenAI, so Claude uses the
native login inside its guest instead. After authentication, run
`bench doctor --agent claude`, then the smoke task in [PILOT.md](PILOT.md) before
your selected terminal-game attempt. Long-lived refresh remains unverified; a successful
auth-status check alone does not prove account entitlement or a model response.

Docker also documents `sbx secret set anthropic` and `sbx secret set openai` for API keys. These are separately billed alternatives, not automatically enabled by this release. They do not solve the MCP/credential-isolation failures. Add an explicit billing mode only if subscription reuse cannot work securely and the user chooses that alternative.

References: [Docker credential handling](https://docs.docker.com/ai/sandboxes/configuration/credentials/), [Docker Codex](https://docs.docker.com/ai/sandboxes/agents/codex/), [Docker Claude](https://docs.docker.com/ai/sandboxes/agents/claude-code/), [OpenAI authentication](https://learn.chatgpt.com/docs/auth), [Claude authentication](https://code.claude.com/docs/en/authentication).

## Next actions and remaining blockers

These work now without an agent account or model call:

```sh
bench --mode strict doctor
bench --mode strict check-integration
```

Both strict commands currently return 1 with the documented isolation failures.
Pilot readiness passes for both authenticated agents on this host. Use
`bench doctor --agent claude --json` or `--agent codex` for structured readiness;
`bench check-runtimes --agent claude` or `--agent codex` checks one runtime.
Without an agent selection, runtime checks cover both in strict mode and default
to Codex in pilot mode. The integration diagnostic uses fixed trusted sentinels
and a Rust fixture with the exact `itoa` 1.0.15 dependency. Passing pilot checks
does not certify strict isolation.

Remaining requirements are a supported way to disable the automatic MCP gateway and unrelated credential bindings, bounded backend snapshot/cache creation, verified provider subscription/egress behavior, and completion of the remaining network/runtime acceptance matrix. No verified installation or configuration command currently satisfies the whole contract. Do not remove `Sbx::evidence_gate` as a workaround. SSH forwarding is now disabled on this machine, but this does not remove MCP. Harness commands still do not change global settings or policy automatically.

After those requirements are implemented and verified, the intended user workflow is:

```sh
bench auth claude
bench auth codex
bench run --agent claude --model '<exact-model-id>' --task '<task-name>'
bench run --agent codex --model '<exact-model-id>' --task '<task-name>'
bench play '<run-id>'
```

Those strict-mode attempt/play commands are not claimed to work in this release. They currently stop before task delivery or execution of an archived submission. The [Docker pilot](PILOT.md) is available. Keep the actual review terminal at the archived task's dimensions: 124 columns × 69 rows for `spaceship`, or 120×40 for `smoke`.
