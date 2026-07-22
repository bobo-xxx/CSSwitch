# Linux Headless Codex OAuth Design

Date: 2026-07-22

Baseline: CSSwitch v0.8.1 (`c93c7e64d75703d38f08c385ed94460b5057831b`)

Target branch: `linux-headless-oauth`

## Purpose

Port CSSwitch's existing Codex subscription bridge to a headless Linux host without adding a Linux desktop UI. The result must let the user authenticate one CSSwitch-owned Codex account through an SSH-forwarded OAuth callback, run the Rust Gateway as the only inference backend, and keep using the existing headless Claude Science runtime and data directory.

This is a private Linux adaptation of CSSwitch's experimental Codex bridge. It does not claim official OpenAI or Anthropic support.

## Confirmed environment

- Linux x86_64 host with no graphical display.
- Claude Science executable: `/home/bio-13/.local/bin/claude-science`.
- Claude Science data directory: `/work/run/projects/bio-13/.claude-science`.
- Claude Science port: `9002`.
- CSSwitch Gateway port: `11434`.
- HTTP and HTTPS egress proxy: `http://127.0.0.1:2999`.
- The user can forward local port `1455` to server loopback port `1455` over SSH.
- The old v0.3.6/Kimi checkout is preserved only as a backup and is not a runtime fallback.

## Scope

### Included

- A Linux-only headless OAuth entry point built on the existing Authorization Code + PKCE implementation.
- Linux use of CSSwitch-owned auth status, logout, refresh, dynamic model discovery, and Codex inference.
- Direct headless startup of `csswitch-gateway --provider codex`.
- Script-based `start`, `stop`, and `status` operations for the Gateway and Claude Science.
- Reuse of the existing Science data directory without database rewriting.
- Dynamic exposure of only the models returned by the authenticated Codex account.
- One redacted status check, model-catalog check, and user-authorized minimal live inference after login.

### Excluded

- Tauri or any graphical Linux UI.
- `xdg-open`, X11 forwarding, Wayland forwarding, or a bundled browser.
- Device-code authentication.
- Manual pasting of OAuth callback URLs or authorization codes.
- Reading, importing, or modifying native `~/.codex` credentials.
- Kimi/provider switching or automatic fallback.
- A watchdog, systemd unit, or automatic process restart.
- Collaborator accounts, roles, invitations, or per-user Science instances.
- A wrapper for `claude-science url`; the existing command remains authoritative.
- Hardcoded or fabricated Codex models.

## Considered approaches

### 1. Dedicated headless OAuth command — selected

Add an explicit headless command that binds a selected loopback callback port, prints the authorization URL only when the user opts in, and waits for the existing verified callback and token exchange flow.

This keeps desktop behavior stable, works over SSH port forwarding, and reuses the current PKCE, state, private storage, refresh, and atomic commit logic.

### 2. Reuse `login-browser` with `xdg-open` — rejected

The server has no graphical session. Making browser launching depend on `DISPLAY`, desktop packages, or X forwarding would add unrelated failure modes and still would not provide a reliable headless workflow.

### 3. Device code or manual callback pasting — rejected

Device-code authentication is outside the existing v0.8.1 contract. Copying the final callback URL would expose the authorization code and state to shell history or logs and would weaken the established callback validation path.

## User-facing commands

### Authenticate

```bash
csswitch-gateway codex-auth login-headless \
  --callback-port 1455 \
  --show-url
```

Requirements:

- `--callback-port` is mandatory and accepts only `1455` or `1457`.
- Headless mode binds exactly the requested port and never falls back silently.
- `--show-url` is mandatory. It is explicit consent to print the ephemeral authorization URL once to stderr.
- stdout remains machine-readable and secret-free.
- The command waits at most five minutes for the callback.
- `Ctrl-C` cancels before credential commit. If atomic commit has begun, commit finishes rather than leaving partial state.

The user runs this command personally in an SSH terminal. The agent must not run it because doing so would capture the authorization URL in tool or conversation logs.

### Operate the runtime

```bash
csswitch-codex start
csswitch-codex stop
csswitch-codex status
```

- `start` verifies redacted auth status, starts the owned Gateway, checks Gateway health and dynamic model discovery, and only then replaces the current headless Science process with the Codex-backed process. A failed catalog check stops the newly started Gateway.
- Authentication does not start or restart Science automatically.
- `stop` terminates only processes that the script previously started and can still identify.
- `status` reports owned PID state, listener state, Gateway health, and Science status without printing OAuth material, the local path secret, or the Science bearer URL.
- Users obtain and share the Science URL only with `claude-science url`.

## Architecture and data flow

```text
Local browser
  -> OpenAI authorization endpoint
  -> http://localhost:1455/auth/callback
  -> SSH local forwarding
  -> Linux csswitch-gateway callback listener on 127.0.0.1:1455
  -> CSSwitch-owned OAuth files under ~/.csswitch

Trusted collaborator browser
  -> SSH/reverse access to Claude Science on 127.0.0.1:9002
  -> Claude Science Anthropic request
  -> http://127.0.0.1:11434/<local-path-secret>/v1/messages
  -> CSSwitch Gateway Anthropic-to-Responses translation
  -> Codex backend through the configured environment proxy
  -> CSSwitch Gateway Responses-to-Anthropic translation
  -> Claude Science
```

The authorization URL and the Science URL are distinct bearer URLs. Only the user receives the authorization URL. Trusted collaborators may receive the Science URL and thereby share the same Science workspace, tools, history visible to that instance, and Codex subscription usage authority.

## OAuth platform changes

The current macOS `login-browser` command and desktop NDJSON protocol remain unchanged.

The Linux port will:

1. Add `login-headless` as a separate command.
2. Reuse the async callback listener, PKCE generation, state validation, token exchange, mutation lock, atomic credential commit, and cancellation state machine.
3. Replace the browser-launch dependency with a headless authorization-URL sink used only by `login-headless`.
4. Permit CSSwitch production private-file storage on Linux using the same absolute-HOME and file-hardening rules.
5. Permit `status`, `logout`, refresh, inference snapshots, model discovery, and Gateway serving on Linux.
6. Keep `login-browser` rejected on Linux so desktop assumptions cannot activate accidentally.

The headless command's stdout contract must remain bounded, structured, and free of authorization URLs, state, codes, tokens, and account identifiers. The opted-in URL is written once to stderr and is never copied into a persisted log by CSSwitch.

## Credential and local endpoint security

OAuth credentials remain in CSSwitch-owned files beneath `~/.csswitch/`. The implementation must preserve:

- directory mode `0700` and file mode `0600`;
- rejection of symlinks and non-regular files;
- atomic same-directory writes and generation/epoch consistency;
- no token values in stdout, stderr after the initial authorization URL, process arguments, logs, status output, or configuration;
- no access to native `~/.codex` authentication.

The headless runtime generates a random local Gateway path secret on first start and stores it in `~/.csswitch/headless/runtime.env` with mode `0600`. The Gateway receives it as `CSSWITCH_AUTH_TOKEN`; Science receives it only as part of `ANTHROPIC_BASE_URL`. Scripts never print it.

The Science access URL remains a bearer invitation. The user intentionally shares it only with trusted collaborators. This project does not add collaborator isolation or revocation. Stopping Science is the emergency access cutoff.

## Runtime management

Installation layout:

- `~/.local/bin/csswitch-gateway`: built Rust Gateway.
- `~/.local/bin/csswitch-codex`: headless control script.
- `~/.csswitch/headless/`: runtime environment, PID records, and non-sensitive status metadata.
- Existing CSSwitch OAuth files remain directly under the established `~/.csswitch` namespace.

The control script must fail closed when:

- authentication is missing or unavailable;
- no compatible Codex model can be discovered;
- ports `11434` or `9002` are owned by unknown processes;
- a recorded PID no longer matches the expected UID, executable, port, or Science data directory;
- Gateway health or model discovery fails;
- the Science executable or data directory is unavailable.

The script may stop only a process it launched and can verify. It must not use broad `pkill -f` matching. There is no automatic fallback to Kimi and no automatic restart loop.

`start` uses the existing `HTTP_PROXY` and `HTTPS_PROXY` environment route for OAuth exchange, refresh, model discovery, and inference. Browser networking stays on the user's local machine.

## Science behavior

The existing Science database and conversation history are reused without direct modification. CSSwitch does not migrate or rewrite model identifiers in stored conversations. Dynamic models are served using the existing `Codex / ...` Science-compatible aliases, and requests are accepted only when the alias resolves to a model in the current authenticated account catalog.

The only supported inference backend in this new runtime is Codex. Authentication, catalog, translation, or upstream failures are reported as failures; they never trigger another provider.

## Error handling

- Invalid headless arguments exit with the existing invalid-arguments code.
- Unsupported callback ports fail before opening a listener or generating an authorization URL.
- An occupied requested callback port fails explicitly; no fallback occurs.
- OAuth denial, timeout, cancellation, proxy failure, TLS failure, challenge responses, storage failures, and inconsistent auth generations retain structured error codes.
- The authorization URL is emitted only after the callback listener is successfully bound.
- Failed or cancelled login before commit does not create authenticated state.
- Errors after HTTP streaming begins remain protocol-safe and do not retry inference POST requests.
- Start failures leave the runtime stopped rather than falling back to a different provider.

## Test strategy

Implementation follows red-green-refactor. Production changes require a failing test first.

### OAuth CLI tests

- `login-headless` is accepted on Linux while `login-browser` remains rejected.
- `--callback-port` is required and restricted to `1455` or `1457`.
- `--show-url` is required.
- The authorization URL is written once to the injected stderr sink and never to structured stdout.
- URL emission occurs only after exact-port binding succeeds.
- Callback state mismatch, denial, timeout, and cancellation remain fail closed.
- Atomic commit and status output retain the existing secret-free contract.

### Platform/storage tests

- Linux production state root resolves only from an absolute HOME.
- Private files use the required modes and reject symlinks/non-files.
- Linux status, logout, refresh, and inference snapshots use the CSSwitch namespace and never native `~/.codex`.

### Runtime-script tests

- Start order is auth -> Gateway -> Gateway health -> catalog -> Science; a catalog failure stops the newly started Gateway.
- Unknown port owners are never killed.
- Stale or mismatched PID records fail closed.
- Runtime secrets and Science URLs never appear in status output or logs.
- No Kimi fallback path exists.

### Regression and live verification

- Focused Gateway tests for each red-green cycle.
- Gateway formatting, clippy, and complete Rust tests.
- Repository `bash test/run_all.sh`, reported using the repository's `current-env clean` versus `release-ready green` vocabulary.
- The user personally performs OAuth through the SSH tunnel.
- After user confirmation, run redacted auth status and dynamic model discovery.
- With explicit authorization already granted, send one minimal prompt through the complete Science -> Gateway -> Codex path and report the live-provider result separately from mock/source tests.

## Rollout and rollback

The original `/work/run/projects/bio-13/CSswitch` checkout remains untouched. Its complete dirty v0.3.6 backup is retained at:

`/work/run/projects/bio-13/test/claude-science/backups/CSswitch-v0.3.6-dirty-20260722`

The implementation is developed on local branch `linux-headless-oauth` from v0.8.1. Local commits are authorized; pushing, publishing, or opening a pull request is not authorized.

Rollback of this experiment means stopping the owned Codex Gateway and Science processes and reinstalling or invoking a separately chosen prior setup manually. The new control script itself provides no Kimi fallback or provider switch.

## Completion criteria

The work is complete only when:

1. All new behavior has failing-then-passing automated tests.
2. Existing macOS browser login and desktop protocol tests remain unchanged and passing.
3. A Linux Gateway binary is installed in the approved layout.
4. The user completes SSH-forwarded OAuth personally.
5. Redacted status and dynamic model discovery succeed.
6. The owned Gateway and existing Science runtime start on ports `11434` and `9002`.
7. One authorized minimal live request succeeds through the complete path.
8. No credential, OAuth URL, path secret, or Science bearer URL is captured in project logs or the agent transcript.
