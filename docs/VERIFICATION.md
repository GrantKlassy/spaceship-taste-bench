# Evidence and remaining integration work

Updated 2026-09-22 on Ubuntu 24.04 WSL2 amd64, using installed CLI help, official documentation and real local microVMs. Docker sign-in and the deny-all baseline were already configured. No provider login, model request, diagnostic upload or official attempt has been performed.

**The installation and trusted fixture workflow work. Official attempts still fail required backend isolation.** The remaining failures are explicit checks, not configurable trust assertions. `bench doctor`, `bench check-runtimes` and `bench check-integration` return 1 while their required checks fail.

## Completed in the readiness pass

- Installed `bench` on PATH. Rust 1.97.0, Docker Engine 29.8.1, Buildx 0.37.1 and local `sbx` 0.45.0 are available. Backend diagnostics passed all 13 checks during inspection; later checks sometimes report an optional update-lookup warning. No backend upgrade was performed.
- Disabled `ssh.agentForwardingEnabled` with the supported setting and restarted the daemon after checking that there were no sandboxes. Fresh shell, Claude and Codex guests have **no SSH-agent socket**. `SSH_AUTH_SOCK` can still be present as an inert variable; its name alone is not evidence of forwarding. Clipboard image paste and Claude remote control are disabled too. Harness commands inspect these settings without changing them.
- Built and loaded `linux-rust-v2`, preserving the exact v1 toolchain/package layers. Claude's built-in runtime failed when its settings directory was not pre-created with agent ownership. The v2 image creates that empty directory with the correct owner and both agent images have their correct flavor labels. Fresh Claude 2.1.278 and Codex 0.155.1 runtimes now start and report their pinned versions. Original v1 images and archives remain preserved.
- Image preparation now checks candidate startup/version inside a real microVM before publishing a resolved environment lock. It verifies each predecessor's image identity and common base layers, and never compiles a submission on the host.
- Added `bench check-runtimes [--agent claude|codex]`: fixed inspection, version and help probes only. Both actual built-in runtimes are tested independently of the agent-free replay fixture. Successful CLI startup does not certify managed configuration or isolation.
- Replaced the CLI's unconditional `doctor` error with a structured readiness report. `bench doctor --agent codex --json` checks the evaluated settings, actual local image store and account prerequisites. Unknown or failed queries remain blocked; account values are not printed or logged. Source-defined unresolved backend requirements remain blocking.
- Added a registry fixture pinned to `itoa` 1.0.15. Its lockfile was generated in a guest. Live checks exercised real `cargo vendor --locked`, verified unchanged source/lockfile hashes, exported the separate vendor bundle from a stopped VM, and built/ran the preserved project with `--frozen` in a third clean VM under deny-all. Actual 120×40 PTY dimensions, Ctrl-C forwarding and terminal restoration passed.
- Administrative setup and guest vendoring/build jobs now support cancellation. Guest jobs have no 30-second build deadline. Output keeps draining after the capture limit, and cancellation kills transport descendants retaining pipes. Snapshot reads, blob hashing and decompression also check cancellation. Stop/destruction/template cleanup are not suppressed by the abort flag. Creation and snapshot RPCs settle within the administrative timeout before cleanup to avoid racing daemon-side work.
- Fixed Claude argument transport: `sbx exec` rejects empty argv entries, so the adapter now uses `--setting-sources=` to deliver an empty setting-source selection. A regression test covers the backend argument contract.
- Accounted for Claude's five extra writable ext4 volumes, which are absent from `sbx inspect`'s `runtime_mounts` list. Block devices are exactly 2 GiB for projects and 512 MiB each for sessions, todos, shell-snapshots and statsig. The harness reserves their combined 4 GiB within `disk_mib`, reduces the requested root disk accordingly and verifies every device and filesystem capacity. Concurrent guests did not share test markers; the repeatable runtime diagnostic also verifies a replacement guest inherits none after deletion. Both agents pass resource checks.
- Codex invocation explicitly selects Docker's subscription broker endpoint and sentinel, including when user configuration is ignored. The 2026-09-22 follow-up corrected the incompatible native `forced_login_method="chatgpt"` override. Fresh-guest checks now reject missing OAuth mode, unexpected auth-file contents and API-key overrides. This does not establish broker login or refresh.

## Codex-first follow-up

The user selected Codex with a ChatGPT subscription for the first attempt. The installed sbx 0.45.0 embeds an OAuth setup that uses a custom `sandboxd` model provider and a placeholder API-key-shaped auth file. The former adapter both ignored that generated provider and forced native ChatGPT login. The adapter now supplies the exact subscription endpoint and bearer sentinel explicitly, and production verification requires the observed OAuth mode and expected placeholder credentials. No authentication or model request was used to make this correction.

A separate trusted minimal `schemaVersion: "2"` sandbox kit, with no inherited agent kit, no credential declarations, no workspace mount, skills off, and explicit deny-all networking, created successfully from the preserved base image. It removed the unrelated declared provider services and generated `.codex`/`.claude`/`.agents` directories. It still had `mcp_gateway: true`, the `mcpgateway` service, gateway environment bindings and `GH_TOKEN`. Passing `--static-mcp=` did not disable the gateway or dynamic discovery. A fixed `initialize` and `tools/list` probe received HTTP 200 under deny-all and listed `code-mode`, `mcp-add`, `mcp-config-set`, `mcp-exec` and `mcp-find`. No tool was invoked. The temporary guests were destroyed and no attempt was allocated.

The local MCP registry and governance profile list were empty. The installed settings have no gateway-disable switch, and local `policy deny` exposes only network rules. These results rule out an empty kit or an empty MCP registry as sufficient isolation. See Docker's [MCP modes](https://docs.docker.com/ai/sandboxes/mcp-gateway/#choose-an-mcp-mode), [custom kits](https://docs.docker.com/ai/sandboxes/customize/kits/) and [Codex authentication](https://docs.docker.com/ai/sandboxes/agents/codex/#authentication).

A labeled pilot using Docker's normal gateway-bearing microVM boundary would change the current project contract and is awaiting the user's decision. It is not implemented as a bypass. Otherwise, gateway isolation still needs supported backend/governance enforcement. Host filesystem quotas also still require administrator setup: `sudo -n true` reports that a password is required, and the current ext4 mount has no configured quota.

## Remaining backend and authentication blockers

1. **MCP gateway and credential scope.** `sbx inspect` still reports an MCP gateway and an attached `mcpgateway` service. Shell guests still receive unrelated provider/integration bindings. A previous fixed HTTP probe reached the gateway under explicit deny-all and received HTTP 400; no MCP tool was invoked. Docker documents that every sandbox starts a gateway, and MCP governance is separate from network policy. The installed local `policy deny` interface only supports networking. [MCP access policies](https://docs.docker.com/ai/sandboxes/governance/access-controls/mcp/) require organization governance; there is no local MCP preset. A supported removal mechanism or independently verified external denial is still needed. An empty server registry or an agent flag does not provide that isolation.
2. **Actual agent runtime configuration.** Codex gets generated `.codex/config.toml` gateway settings; Claude gets generated `.claude` settings and gateway configuration. The effective neutral settings and broker state still require verification after backend service isolation is resolved. CLI help/version success does not establish which managed settings take effect. Claude's additional storage is now accounted for and passes separate live checks.
3. **Provider subscription authentication and egress.** The backend currently has no OpenAI or Anthropic provider credential entries. Codex's documented host OAuth command is wired; Claude's subscription subcommand is present in the pinned CLI, but fresh-guest broker reuse remains unverified and `bench auth claude` remains unavailable. Complete the authentication-only flow after service isolation is established, observe token masking/reuse/refresh, prove API-key precedence cannot change billing, and derive the required generation destinations. No guessed production allowlist is enabled. A user's interactive provider sign-in cannot be completed unattended.
4. **Host snapshot/cache growth.** Guest root disk limits and parser/extraction limits are verified. `sbx template save --output` can create backend storage/cache before the parser sees the file. Neither an output-size check nor a free-space check is an enforced host-side growth limit. The installed interface has no verified bounded export option. A supported backend limit or administrator-provided, verified filesystem quota is required; passwordless administrator access is unavailable on this host. No storage directories were moved and no filesystems or quotas were changed.
5. **Remaining network coverage.** Explicit authorizer checks cover hostnames, host/metadata destinations, IPv4, IPv6, loopback/LAN and UDP decisions. Live HTTP probes cover normal proxy use and bypassing proxy environment variables. Under dependency policy, both proxy and direct requests fetch the real sparse index; the downloaded crate checksum matches that index. Denied direct hostnames can fail at DNS, while some raw host/metadata routes close/refuse connections. Those transport failures are recorded as observations, not proof that policy caused them. Complete controlled raw IPv4/IPv6, non-HTTP TCP, UDP/ICMP and redirect acceptance before claiming all paths are certified.

These requirements remain enforced in production. There is no user-editable certificate, unsafe switch, host-execution path or ordinary-container fallback. Provider login alone cannot make an official run pass the current preflight.

## Export and cancellation evidence

Stopped exports retain the stopped state before and after saving. The exporter reads the observed Docker-save manifest with OCI digest-addressed gzip blobs, checks hashes and bounds compressed and decompressed data. It reconstructs only `/workspace` or `/replay`, preserves byte/executable/directory inventories and rejects unsafe entries. No guest code is run after stop to collect source.

A trusted source with a real vendored dependency completed export and replay on 2026-09-22. No source or lockfile was repaired. The new cancellation tests exercise SIGINT during provisioning, after guest creation, during snapshot export and at the guest build phase; check the local results below for their latest status. Cancellation cleanup checks both exact guest names and exported template names. SIGKILL, power loss, daemon timeouts and every possible timing race are not claimed covered.

## Reproduce

```sh
bench doctor --agent codex --json
bench check-runtimes
bench check-integration
cargo +1.97.0 test --manifest-path harness/Cargo.toml --locked --test real_backend real_ctrl_c -- --ignored --nocapture --test-threads=1
```

The first three commands deliberately return 1 while isolation remains blocked. Diagnostics continue only with fixed trusted probes/fixtures; they cannot receive a task, arbitrary source, archived submission or model request. They do not allocate an attempt or freeze the prompt.

Raw administrative output remains in private state under bounded `backend/` directories. Attempt setup and packaging diagnostics go under private `raw/<run-id>/`. Authentication input/output is never logged. Saved image archives and full temporary snapshots stay outside Git.

## Local results

- Ordinary Rust suite: **62 passed**, six real-backend tests ignored by default.
- Formatting and Clippy with `-D warnings`: passed.
- Fresh v2 Claude/Codex startup, pinned versions, adapter flags, CPU/memory/disk limits and Claude volume freshness: passed. Full runtime checks still fail the configuration/service requirements above.
- Real dependency vendoring, stopped source/package export and clean-VM frozen replay: passed. Integration still fails required host-service isolation.
- Live cancellation: all four checks passed: provisioning, post-creation, snapshot export and the guest build phase.

Only the harness, reviewed image preparation and trusted test drivers execute on the host. All fixture Cargo commands execute inside microVMs. There are no archived attempts. The task contract now selects `linux-rust-v2`; prompt bytes were not changed.

## References

- [Docker MCP gateway](https://docs.docker.com/ai/sandboxes/mcp-gateway/), [MCP policy reference](https://docs.docker.com/ai/sandboxes/governance/reference/mcp-policy/), [credentials and SSH forwarding](https://docs.docker.com/ai/sandboxes/configuration/credentials/), [local network policy](https://docs.docker.com/ai/sandboxes/governance/access-controls/local/), [root disk sizing](https://docs.docker.com/ai/sandboxes/troubleshooting/#sandbox-runs-out-of-disk-space).
- [Docker Claude authentication](https://docs.docker.com/ai/sandboxes/agents/claude-code/), [Docker Codex authentication](https://docs.docker.com/ai/sandboxes/agents/codex/), [OpenAI authentication and forced login method](https://learn.chatgpt.com/docs/auth).
- [Cargo vendor](https://doc.rust-lang.org/cargo/commands/cargo-vendor.html), [Cargo registry protocols](https://doc.rust-lang.org/cargo/reference/registries.html#registry-protocols).
