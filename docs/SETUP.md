# Local setup

The development machine has the required toolchain and images. **Codex and Claude subscription sign-in and real pilot smoke tests through offline replay now pass.** The Claude smoke requested and reported `claude-opus-5`. Follow [PILOT.md](PILOT.md) on another host. Strict attempts remain blocked by their backend isolation requirements. See [VERIFICATION.md](VERIFICATION.md) for the evidence and accepted pilot limitations.

## Current machine

Checked again on 2026-09-22:

- Ubuntu 24.04.4 LTS, WSL2, amd64; Linux-local backend and Docker Unix socket.
- Rust/Cargo 1.97.0, Docker Engine 29.8.1, Buildx 0.37.1, standalone `sbx` 0.45.0.
- Working `docker`/`kvm` group access; KVM API 12; Docker sign-in and deny-all baseline initialized by the user.
- All 13 sbx diagnostics have passed. Some checks return 12 passes and an optional update-lookup warning; the installed CLI remains 0.45.0. Base, Claude 2.1.278 and Codex 0.155.1 images are built, loaded and recorded in `environment.lock.json`.
- Real guest creation, trusted file copying, stopped export, dependency connectivity, 120×40 PTY/Ctrl-C and clean-VM fixture replay work.
- SSH forwarding was disabled during the 2026-09-22 maintenance pass after verifying there were no existing sandboxes. Fresh shell, Claude and Codex guests have no SSH-agent socket. MCP services and unrelated provider bindings remain attached; strict mode rejects them and the accepted pilot records them. The user completed broker OAuth, and the pilot smoke test succeeded without copying a normal agent home.

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
cargo +1.97.0 install --path harness --locked
bench doctor
```

Host Python 3 is needed for image preparation and the opt-in PTY diagnostic driver. Game compilation and Cargo build scripts never run on the host.

## Trusted images

Before an environment is first resolved:

```sh
python3 environments/linux-rust-v2/prepare.py
```

The v2 script supplies only the three reviewed Dockerfiles as build context. It checks the exact v1 predecessor image IDs, reuses their packages and base layers, corrects Claude home-directory ownership and agent flavor labels, and verifies fresh built-in runtime startup before recording the new lock. It never supplies the repository, runs, home, credentials or instructions. Both agents inherit the same Linux/Rust/native terminal toolchain and neutral `/workspace`; no crate choice, game architecture or starter source is included.

Base/index/platform digests and agent package pins were resolved from public registries. Native OS packages are resolved during the first trusted build; `/opt/bench/native-packages.tsv` records their exact versions. The complete built image identity then freezes that environment. Initial package resolution is not claimed reproducible. Docker 29's image IDs can identify OCI indexes rather than image configurations; the lock records actual returned identities.

**This checkout is already resolved.** Its local image archives are in ignored `environments/linux-rust-v2/build/`. To use those exact images on another compatible host, transfer the preserved archives outside Git and load them:

```sh
sbx template load environments/linux-rust-v2/build/base.tar
sbx template load environments/linux-rust-v2/build/claude.tar
sbx template load environments/linux-rust-v2/build/codex.tar
```

No image/artifact has been uploaded. Preparation refuses an already resolved environment or a checkout with archived attempts. Intentional tool updates require a new environment version and supported contract in the harness; preserve old image archives for old attempts. Do not delete lock identities to pretend a rebuild is the historical environment.

Copy `bench.example.toml` to ignored `bench.local.toml` to adjust machine-protection limits. These are CPU, memory, disk, export/replay and log safety settings, never benchmark time/token/cost budgets. `disk_mib` covers the root disk plus writable runtime volumes. Claude reserves 4096 MiB for its five runtime volumes and requires at least 6144 MiB total; the default 20480 MiB gives it a 16384 MiB root disk. Codex and shell guests use the full allocation for their root disk.

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
your Opus spaceship attempt. Long-lived refresh remains unverified; a successful
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
bench run --agent claude --model '<exact-model-id>'
bench run --agent codex --model '<exact-model-id>'
bench play '<run-id>'
```

Those strict-mode attempt/play commands are not claimed to work in this release. They currently stop before task delivery or execution of an archived submission. The [Docker pilot](PILOT.md) is available. Keep the actual review terminal at the archived task's dimensions: 124 columns × 69 rows for `spaceship-v1`, or 120×40 for `smoke-v1`.
