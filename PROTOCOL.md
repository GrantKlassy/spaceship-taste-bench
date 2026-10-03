# Attempt protocol: single-attempt-v1

## Docker pilot variant

`mode = "docker-pilot"` selects `single-attempt-docker-pilot-v1` for Codex or Claude.
Pilot metadata records the mode and accepted limitations; `isolation_verified`
remains false. Docker-managed MCP/credential services, generated agent
configuration, unverified subscription refresh/exhaustive network coverage, and
unbounded host snapshot/cache growth do not block this variant. Guest resources,
image identities, mount/socket checks, explicit network rules, exact prompt
delivery, immutable exports and cleanup still apply. Replay must use the same
mode and does not claim the strict credential-free service boundary.
Generation records the selected agent's observed Docker network defaults plus
crates.io.
See [PILOT.md](docs/PILOT.md) for the executable workflow.

The isolation and certification requirements below describe strict mode. The
single-attempt and preservation rules also apply to the pilot unless overridden
above. Neither mode runs submissions or their Cargo build scripts on the host.

Each record is one particular attempt at one release/model, followed by a person's observations. Hardware and elapsed time are contextual information. There are no scores, deadline budgets, repeated trials, selection of a best result, or statistical claims.

## Input snapshots

The host controller selects an explicit agent, model identifier, task name and environment identity. It archives the task's exact UTF-8 prompt bytes and hashes them with SHA-256. No adapter adds design advice or rewrites the prompt. Tool-enabling settings are recorded separately.

Run directories use `<agent>-<model-slug>-YYYY-MM-DD`, with a UTC date and single
hyphens. The model slug is lowercase, collapses punctuation to a single hyphen,
and is capped at 64 characters with trailing hyphens removed. Each agent/model
slug can have one run per UTC date across all tasks, including smoke tasks.
Allocation rejects an existing ID and preserves its artifacts.

Tasks and environments are updated in place under stable names. At allocation, a host file lock serializes archive validation and creation of the input snapshot. Each run preserves its exact prompt, task contract, environment lock and settings with SHA-256 checksums. Later source edits affect future attempts only; existing snapshots are never rewritten. Malformed or modified archived inputs block allocation. Playback and recovery load the run's archived environment lock and require the recorded image identities. Machine-protection settings may change and are recorded per attempt.

No parent `.git`, instructions, starter game, earlier solution, review, transcript, or run metadata is sent to a guest. A fresh neutral `/workspace` is initialized as an empty Git repository without a remote. Backend guest names are independent random identifiers, not the public run ID.

## The request

Generation requires a new externally isolated guest and a new agent session. A reused clean image is permitted; a reused writable guest is not. CLI versions are pinned per environment, checked against the reviewed lock, and verified before task delivery. `linux-rust` selects Claude Code 2.1.280 and Codex 0.155.1. The model argument is passed exactly as selected; there is no fallback model or harness retry. Reported model identity is recorded only when the native stream supplies it. A provider can still reject an identifier; unavailable identity is null.

The task is written once to stdin, then stdin is closed. The agent can edit, execute commands, compile, test, and revise during its natural tool loop inside the VM. The controller imposes no wall-clock, token, or cost deadline. It does not send follow-up messages, repair prompts, clarification answers, automatic resumes, or a second request. Provider/CLI internal transport recovery is part of that CLI's behavior, not an additional harness attempt.

If the agent returns a question and exits normally, that is its outcome. It receives no answer. A normal native terminal event plus successful process exit establishes completion; game launchability is not inferred from it. Missing or malformed lifecycle events are not guessed to be success.

## Outcomes

| Field | Meaning |
| --- | --- |
| `not_started` | Prompt has not been delivered |
| `normal` | Native completion event and successful transport exit |
| `interrupted` | Provider/auth/network/agent error or missing successful terminal event; precise cause may be unknown |
| `user_abort` | User signal stopped this attempt |
| `infrastructure_failure` | Controller/guest setup or transport failure |

Export, cleanup, replay preparation and playback are separate fields. A completed agent can have failed export or a game that does not build. Post-completion infrastructure failure does not rewrite natural agent completion as an agent failure.

Start/end timestamps use UTC. Agent elapsed time uses a monotonic clock and is observational. Usage and cost are copied only from recognized numeric native fields. Absent values are JSON null. Raw provider text and arbitrary native JSON fields do not become public metadata.

CPU, memory, disk, export count/size, replay count/size and raw-log size limits protect machines. They are not benchmark budgets. Raw-log overflow truncates capture while the agent request continues and the event parser keeps draining; metadata records truncation. Administrative control operations have a 30-second liveness timeout; it is not applied to agent requests or guest vendoring/build jobs. Those jobs remain cancellable. Creation and snapshot RPCs are allowed to settle within the administrative bound before cleanup, avoiding a race with daemon-side creation. Other setup commands stop their transport process group promptly on cancellation; cleanup still runs.

## Preservation

After completion or abort, all remaining guest processes must stop before a filesystem snapshot is taken. Export must not restart a modified guest or run its hooks. Stopped snapshot semantics and source preservation have been verified with trusted fixtures. Host-side growth during backend snapshot/cache creation remains unbounded by a verified capability and blocks production.

The controller validates transport paths and entries, applies documented infrastructure exclusions, bounds extraction, and atomically publishes the source directory. It does not fix source, rewrite manifests, generate a missing lockfile, or edit the README. Checksums cover file bytes, executable flags, and directory records. Infrastructure/auth paths are excluded as described in SECURITY.md; rejected archives retain meaningful failure metadata instead of partial published source.

Failed exports preserve the stopped image and a bounded error log in private state. Successful exports remove the full image after source validation. Retained images are recovery inputs, never published submissions or permission to execute the original guest again.

`bench recover <run-id>` can reprocess a retained stopped image after an export fix. It validates the archived inputs, mode, environment and original resource bounds, refuses an existing solution, and records the previous run metadata plus the snapshot checksum in `export-recovery.json`. Recovery updates export and replay status while retaining the original generation outcome, timestamps and usage. It neither makes a model request nor restarts the generation guest or edits source.

Cleanup guards cover partially created guests and normal error unwinding. Ctrl-C/termination requests are handled by the controller. SIGKILL and host power loss require the exact-name recovery procedure in README. Explicit retries with the same agent/model must use a later UTC date and never overwrite prior attempts.

## Replay and review

Locked vendoring is attempted in a different disposable agent-free VM. Only crates.io lockfile sources are accepted; Git/custom-registry dependencies and escaping paths are rejected. `cargo vendor --locked --versioned-dirs /replay/vendor` must leave the source tree and lockfile unchanged. Packaging sets `CARGO_NET_OFFLINE=false` for registry access even when a submission defaults to offline mode; playback remains frozen and offline. Cargo's printed source configuration remains separate from the original submission. Failure is recorded without repair.

The local bundle is outside Git history and has a checksum, environment identity and source-tree binding in `replay.json`. Play selects the archived environment and verifies these before importing anything. The replay VM receives the original source, preserved dependencies, and runtime; no agent or credentials, and no networking. It builds with frozen/offline resolution in a real UTF-8/256-color PTY at the dimensions recorded in the archived task contract. Both host and guest PTY dimensions are checked.

A person writes `review.md`. There is no automatic subjective assessment. One result does not establish a general ranking of models or tools.
