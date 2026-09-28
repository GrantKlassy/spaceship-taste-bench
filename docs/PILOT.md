# Docker pilot

The `docker-pilot` mode uses the installed Docker Sandboxes microVM boundary. It
accepts Docker's managed gateway, credential bindings and generated configuration,
plus the lack of a host snapshot/cache quota. These are recorded limitations,
not claims that the strict protocol has been certified. This mode supports Codex
with a ChatGPT subscription and Claude Code with a Claude subscription.

Select it once in ignored `bench.local.toml`:

```toml
mode = "docker-pilot"
```

Other settings retain their defaults. `--mode docker-pilot` or `--mode strict`
overrides the configuration for one command. Strict remains the default for a
checkout without local configuration, and its certification gate remains closed.

Authenticate the Docker broker for the agent you want in your own terminal:

```sh
bench auth codex
bench doctor --agent codex

# Or Claude Code:
bench auth claude
bench doctor --agent claude
```

Codex uses Docker's host-side OAuth command. Claude opens `claude auth login
--claudeai` in a disposable mountless guest. Complete the browser sign-in in your
own terminal. The harness destroys that guest, then checks that Docker supplies
subscription credentials independently to a new guest. Authentication output is
not captured in harness logs.

Fresh-guest checks require OAuth mode, the expected placeholder credentials, and
no API-key override. Claude must also report a Claude subscription login through
its native auth-status command. Existing native login files are not copied.
Account entitlement, token refresh and a real
model response are only established when exercised; doctor makes no model request.
OpenAI distinguishes [subscription login from API-key billing](https://learn.chatgpt.com/docs/auth).

With an exact model ID, use the separate small smoke task before the game:

```sh
bench run --agent codex --model '<exact-model-id>' --task smoke-v1
bench play '<smoke-run-id>'
```

For Opus 5 with the frozen `linux-rust-v2` image:

```sh
bench run --agent claude --model claude-opus-5 --task smoke-v1
bench play '<smoke-run-id>'
# Then the same frozen spaceship task used by the Codex runs:
bench run --agent claude --model claude-opus-5
```

Use the full model ID from [Claude's model configuration](https://code.claude.com/docs/en/model-config),
not the moving `opus` alias, to preserve which release you requested. The native
stream's reported model is archived separately. The harness does not set a fallback model.
This image pins Claude Code 2.1.278. Opus 5.5 requires 2.1.280 or later; testing
it requires a new environment and task version, rather than rebuilding v2.

Smoke playback needs a 120-column by 40-row terminal. A successful smoke test generates
and packages a tiny Rust program, then prints `bench smoke: 42` during playback.
Its archive freezes only `smoke-v1`, leaving `spaceship-v1` editable. Once your
game prompt is ready, use `bench run --agent codex --model '<exact-model-id>'`.
The current `spaceship-v1` prompt and contract use 124 columns by 69 rows.
Playback always uses each run's archived contract dimensions.

Pilot records use protocol `single-attempt-docker-pilot-v1`, `mode: docker-pilot`,
an explicit `accepted_limitations` list and `isolation_verified: false`. Replay
records carry the same mode, and playback refuses a mode mismatch. Prompt bytes,
task freezing, immutable source hashes, isolated packaging and cleanup retain the
same behavior as strict attempts.

Guests still have no host workspace mount, no shared skills, no published ports,
and no SSH-agent or host Docker socket. Image versions, empty workspaces, CPU,
memory and writable disk limits remain checked. Generation uses Docker's pinned
defaults for the selected agent: Codex's OpenAI/code/package hosts or Claude's
Anthropic/Claude service hosts, plus HTTPS to `index.crates.io` and
`static.crates.io`. Claude's observed built-in rules permit `api.anthropic.com`,
`platform.claude.com`, `downloads.claude.ai`, `claude.com`, `code.claude.com`,
`mcp-proxy.anthropic.com` and `bridge.claudeusercontent.com`, all on port 443.
These effective destinations are
checked and archived; packaging permits only the two registry hosts, and playback
uses explicit deny-all and frozen dependencies.
Docker-managed gateway and broker services are outside that guest-network claim,
including during playback. Full configuration neutrality, credential-free replay,
exhaustive network isolation and enforced host-storage quotas are not promised.

`bench check-integration` exercises fixed fixtures without a provider login or a
model call. In pilot mode it also exercises generation policy transitions and the
production package/playback verification path. `bench check-runtimes` defaults
to Codex in this mode; use `--agent claude` to check Claude. Both require a real
OAuth broker configuration to pass.
