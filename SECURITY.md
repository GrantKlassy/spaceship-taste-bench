# Security and trust boundaries

These boundaries apply to every task in terminal-game-taste-bench, including the
`spaceship` game and the `smoke` workflow check. A task's theme does not change its
execution or preservation rules.

**Strict generation and playback remain blocked by their isolation requirements.** The separately labeled [Docker pilot](docs/PILOT.md) uses the user's accepted existing sandbox boundary. It allows Docker-managed MCP/credential services and generated agent configuration, and accepts unbounded host snapshot/cache growth. These limitations are archived on every pilot; `isolation_verified` stays false. Pilot generation uses Docker's pinned network defaults for the selected agent plus crates.io. Packaging permits only crates.io and playback uses deny-all, but these guest rules do not isolate Docker-managed services. The strict guarantees below do not apply to those accepted exceptions. Read [the evidence ledger](docs/VERIFICATION.md).

## Threat model

Treat the generated project, Cargo manifests/build scripts, filenames, transcripts, and the agent's guest processes as hostile. Protect host files/accounts, other runs, host/LAN services, and machine resources. The host controller, Rust dependencies, reviewed environment build inputs, operating system, hypervisor, and backend credential broker are trusted. This is not protection against a compromised host or hypervisor exploit.

The intended execution boundary is a Docker Sandboxes **microVM**, using local `sbx`, not a worktree, directory change, agent permission setting, or ordinary shared-kernel container. No host workspace path is passed to `sbx create`; no clone mode, host volumes, published ports, Docker socket, home directory, SSH agent, or previous guest filesystem is supplied. The built-in guest may contain a container runtime; that alone is not the host isolation boundary.

Docker's [security model](https://docs.docker.com/ai/sandboxes/security/) describes default shared skills and MCP gateway integrations. `--skills off` is insufficient: actual inspection showed an MCP gateway, and guest probes reached that gateway and connected to an SSH-agent socket. No MCP tool, SSH identity or signing request was made. On 2026-09-22, SSH forwarding was disabled using the documented setting and a daemon restart while no guests existed; subsequent fresh-guest probes found no SSH-agent socket. Typed inspection now rejects these services, unexpected runtime mounts, changed identities and network-policy drift. Guest-only checks cannot prove host-side enforcement.

## Networking and credentials

Creation requests an explicit per-sandbox deny-all rule before execution. No permissive fallback or guessed provider allowlist is installed. Dependency policy construction allows only TCP 443 to `index.crates.io` and `static.crates.io`, verified with real index and checksum-checked crate downloads. Generation still needs observed provider/authentication destinations. Global and organization/kit rules are relevant too; a preset called “Locked Down” is not evidence of the effective policy. Harness commands never alter global host policy.

Docker's [local network policy](https://docs.docker.com/ai/sandboxes/governance/access-controls/local/) allows kit-added exceptions even under a deny-all *preset*. Explicit deny rules take precedence. Verification must cover DNS/proxy paths, direct IPv4/IPv6 traffic, non-HTTP TCP, UDP/ICMP, redirects, host/LAN addresses, code hosts, and unintended provider services. The broker's own host access is part of the trusted backend and is distinct from guest egress.

`web_search="disabled"` and Apps disabled are passed to Codex separately, because provider-hosted tools need not follow a guest firewall. Claude's WebSearch/WebFetch, Chrome and external MCP configuration are disabled using its supported flags; safe mode skips normal customizations. Empty guest agent homes and effective settings still need inspection, including managed settings and backend-generated configuration. No custom credential proxy is implemented.

Prefer subscriptions through Docker's documented OAuth broker. Real provider tokens should remain in the host credential store, with only proxy-managed sentinels entering guests. `bench auth codex` invokes the documented broker OAuth command. `bench auth claude` runs native subscription login in a disposable mountless guest, destroys it and verifies broker reuse in a new guest. Claude attempts require observed OAuth mode, expected access/refresh placeholders, a native Claude subscription login and no authentication/provider overrides. Existing API keys take precedence over OAuth according to Docker's documentation; they block subscription attempts. API billing requires an explicit future configuration, not automatic substitution. Raw auth output is never captured into benchmark artifacts. Never copy normal `~/.claude`, `~/.codex`, skills or memory directories into an image or run.

The Codex adapter explicitly reconstructs the pinned backend's OAuth provider routing while ignoring generated user configuration: provider `sandboxd`, endpoint `https://chatgpt.com/backend-api/codex`, and the non-secret `oai-oat01-proxy-managed` bearer. It does not force native ChatGPT login: Docker's broker uses a placeholder `OPENAI_API_KEY` auth file even for subscription sessions, so native `forced_login_method="chatgpt"` is incompatible with that integration. A fresh guest must report `SBX_CRED_OPENAI_MODE=oauth`, have exactly the expected placeholder auth file, and have no API-key override. Those observations are necessary but do not establish refresh, egress or service isolation; the production gate remains closed.

For Claude, sbx 0.45.0 leaves `SBX_CRED_ANTHROPIC_MODE=none` even after the broker
seeds OAuth credentials. The harness instead requires backend inspection to
report `oauth · anthropic`, the guest credential file to contain the exact
`sk-ant-oat01-proxy-managed` and `sk-ant-ort01-proxy-managed` placeholders, and
native `claude auth status` to report a first-party Claude subscription login.
These checks emit only booleans; tokens and account details are not logged.
The backend's descriptive mode field alone never establishes subscription use.

## Resources and images

CPU/memory allocations and Docker's documented [`DOCKER_SANDBOXES_ROOT_SIZE`](https://docs.docker.com/ai/sandboxes/troubleshooting/#sandbox-runs-out-of-disk-space) host-side creation setting were checked in real guests. Root, home, workspace and temporary files share the bounded guest root filesystem. The built-in Claude runtime also attaches five writable ext4 volumes under `.claude`: projects (2 GiB), sessions, todos, shell-snapshots and statsig (512 MiB each). The harness subtracts their combined 4 GiB from `disk_mib` before allocating Claude's root disk. It checks their distinct devices, exact block capacities, filesystem capacities and mount types; extra mounts remain rejected. Live sentinel checks found no sharing between concurrent guests or inheritance by a replacement guest after deletion. `runtime_mounts: []` from backend inspection alone does not account for these devices. Tmpfs storage consumes bounded VM memory.

**Host-side snapshot/cache growth is not yet bounded by a verified backend capability.** Guest disk accounting and parser limits cannot prevent backend disk consumption during snapshot creation. Production remains gated for this reason too.

Image base/index/platform digests and agent package versions were resolved from registries. Trusted preparation has now built and loaded all three amd64 images, recorded their actual Docker identities and checked that both agent images inherit identical base layers. Native package versions are recorded inside the base image. No fabricated digest or `latest` image is used. Base packages are resolved during first preparation, then their versions and complete image identity are frozen. Rebuilding requires explicit `--rebuild` and updates the same environment lock only after all candidate runtimes pass; archived attempts retain their original image identities. The initial package resolution is not claimed reproducible. No benchmark prompt or starter solution is baked in. Image build success does not establish microVM isolation.

## Export and filesystem handling

A stopped generation guest is snapshotted through `sbx template save --output`. The adapter never uses `sbx cp` to unpack an untrusted directory onto the host and never invokes guest code after stop to collect source. Actual snapshots remained stopped before and after saving. The parser supports the observed Docker-save manifest plus OCI digest-addressed gzip blobs, and legacy uncompressed layers used by unit fixtures. The full image is private local data, never a public artifact. Normal successful exports remove it; failed exports retain it with private diagnostics for explicit recovery. Template cleanup uses the exact per-run name.

The host reads tar data as data. It never runs a generated build script, executable, installer, Git hook, archive command, Cargo command, or imported container image. The extractor:

- Stages into a fresh private directory, then renames on full success.
- Accepts bounded ordinary files/directories; preserves file bytes and executable/non-executable distinction, plus empty directories.
- Rejects absolute paths, traversal, backslashes, drive-like paths, controls, ambiguous names, case collisions, duplicate paths, non-ASCII filenames, links, devices, FIFOs, sparse entries, PAX/GNU extension records and setuid/setgid/sticky metadata for selected source entries.
- Does not restore ownership, timestamps, xattrs, ACLs or privileged modes. Normalizes ordinary modes to 0644/0755, then makes archived source read-only.
- Bounds entry count, source bytes, transport bytes and replay size. Excluded entries in direct source-tar input are still validated and counted. Snapshot projection discards excluded paths without extracting their contents or applying their file types. This is deliberately strict and may reject otherwise buildable submissions containing unusual generated metadata.
- Excludes components `.git`, `target`, `.claude`, `.codex`, `.ssh`, `.aws`, `.azure`, `.gnupg`, `.bench`, and files `auth.json`, `.credentials.json`, `.env`, `.env.*` (except `.env.example`). No blanket exclusion of `.cargo`, source dotfiles, or lockfiles.

Image transport parsing ignores filesystem content outside `/workspace` or `/replay`, without extracting it. Native image layers have unrelated Linux filenames and PAX/GNU path metadata; bounded parsing accounts for path overrides so they cannot conceal source entries. Extended metadata on selected source is rejected. Excluded build-cache paths may carry such metadata, including Rust incremental hard links; the effective path is validated before exclusion and links are never followed. Blob hashes are checked; compressed bytes and aggregate decompressed bytes are each capped. Only one expanded layer is stored at a time. Temporary parser storage can therefore exceed the compressed snapshot size; these bounds are distinct from backend snapshot creation.

Whiteouts represent deletions in layer reconstruction, not exported files. Identical repeated directories emitted around opaque whiteouts are accepted; duplicate files or case aliases are rejected. Source-layer byte accounting is conservative and includes superseded content. This may refuse large snapshots instead of losing protection.

Artifact IDs and replay references are validated relative components. Private state and the checkout must be disjoint. Symlinked artifact roots and non-regular metadata are rejected. SHA-256 inventories detect ordinary edits, but are not signatures: someone who can rewrite both source and metadata can forge a local historical record. A trusted release history is still necessary for publication.

## Replay and terminals

Vendoring and game compilation happen only inside separate external sandboxes. A missing lockfile is a recorded failure. Locked vendoring cannot change the source tree; harness configuration stays in `/replay`, not inside `solution/`. Replay requires the preserved base image identity, not a mutable image tag. Provider credentials and agents must be absent, including backend-injected placeholder auth/configuration.

Interactive game output can control the terminal. Use a modern terminal in a disposable window and avoid exposing a terminal with host clipboard integrations. Transcript progress never renders raw output; only interactive playback intentionally displays program output. A trusted fixture verified 120×40 PTY dimensions, Ctrl-C forwarding and transport termios restoration; a separate test verified cleanup after SIGINT to the harness. Live cancellation checks cover provisioning, guest creation, snapshot export and the guest build phase. Snapshot parsing, blob hashing and decompression are cancellable too. Creation/snapshot RPCs settle before cleanup; backend administrative timeouts still cannot prove that a wedged daemon has ceased all work. SIGKILL or a terminal emulator crash can prevent restoration; use the terminal's reset feature if needed.

## Public sharing

`run.json` is host-owned; guest files cannot supply authoritative metadata. Native streams are reduced to known lifecycle, model and numeric usage/cost fields. Unknown information stays null. Raw transcripts, temporary full images, local state, replay bundles, credentials, build outputs and large media are ignored or kept outside the checkout. Never automatically upload them.

Before publishing an attempt, inspect source and the manual review for accidental secrets, identifying information and unsafe links. If publishing a transcript, make a separate reviewed/sanitized copy manually. Do not edit the immutable solution to make it look better. This harness does not promise automatic secret redaction of arbitrary source.
