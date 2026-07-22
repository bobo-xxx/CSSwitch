# Linux Headless Codex OAuth Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a secure, headless Linux OAuth workflow and an owned-process launcher that connects the existing Claude Science instance exclusively to a CSSwitch Codex subscription.

**Architecture:** Keep the desktop `login-browser` NDJSON protocol macOS-only and add a separate one-shot `login-headless` CLI path. Reuse the existing PKCE listener and atomic CSSwitch auth repository through an injected authorization-URL sink, then manage Gateway and Science through a Linux shell controller with a private path secret and strict PID/port ownership checks.

**Tech Stack:** Rust 2021, Tokio, Reqwest, Serde JSON, POSIX Bash, curl, Linux `/proc`, `ss`, Cargo tests, repository shell test layers.

## Global Constraints

- Baseline is CSSwitch v0.8.1 commit `c93c7e64d75703d38f08c385ed94460b5057831b` on local branch `linux-headless-oauth`.
- Linux is headless; do not add Tauri UI, `xdg-open`, X11/Wayland forwarding, device-code login, or callback pasting.
- `login-browser` and its desktop NDJSON protocol remain macOS-only and behaviorally unchanged.
- `login-headless` requires `--callback-port 1455|1457` and `--show-url`; it binds exactly that port with no fallback.
- Print the authorization URL exactly once to stderr only after binding succeeds; structured stdout must contain no URL, state, code, token, account identifier, or path secret.
- The callback deadline is five minutes. Ctrl-C cancels before commit; once atomic commit starts, it is allowed to finish.
- OAuth state is CSSwitch-owned under absolute `$HOME/.csswitch`, preserving directory mode `0700`, file mode `0600`, symlink/non-file rejection, and atomic same-directory writes. Never read or modify `~/.codex`.
- The agent must never execute the real `login-headless` command. The user performs OAuth personally through the existing SSH tunnel on port `1455`.
- Runtime defaults are Gateway `/home/bio-13/.local/bin/csswitch-gateway` on `127.0.0.1:11434`, Science `/home/bio-13/.local/bin/claude-science` on `127.0.0.1:9002`, and Science data `/work/run/projects/bio-13/.claude-science`.
- Preserve the existing Science database and history; do not migrate or rewrite stored model identifiers.
- Codex is the sole backend. Do not add Kimi, provider switching, fallback, a watchdog, systemd, or automatic restarts.
- Models are discovered dynamically from the authenticated account; do not hardcode or fabricate model names.
- Store the generated Gateway path secret only in `$HOME/.csswitch/headless/runtime.env` mode `0600`; never print it or the Science bearer URL.
- Stop only recorded processes that still match UID, executable, expected command arguments, and listener port. Never use broad `pkill -f`; unknown owners fail closed.
- Use the current `HTTP_PROXY`/`HTTPS_PROXY` route (`http://127.0.0.1:2999` in this deployment) for OAuth exchange, refresh, discovery, and inference.
- Local commits are authorized. Do not push, publish, or open a pull request.
- Report mock/source verification separately from the one user-authorized minimal live Science -> Gateway -> Codex request.

---

## File map

- `desktop/gateway/src/codex_auth/login_async.rs`: platform-neutral async login engine, exact-port options, and injected headless URL sink.
- `desktop/gateway/src/codex_auth/cli.rs`: Linux state-root enablement, headless argument contract, stderr URL writer, Ctrl-C orchestration, and final secret-free JSON.
- `desktop/gateway/src/codex_auth/mod.rs`: narrow public wrapper used by the CLI; no UI policy.
- `desktop/gateway/src/main.rs`: command routing order only.
- `desktop/gateway/Cargo.toml`: Tokio signal support for Ctrl-C.
- `scripts/csswitch-codex`: Linux runtime controller and its process/secret invariants.
- `test/test_headless_codex.sh`: isolated fake-process tests for start, stop, status, redaction, and failure cleanup.
- `test/run-scripts.sh`: registers the new shell test in the repository test layer.
- `docs/features/codex-science-bridge.md`: documents the supported headless command without changing desktop contracts.
- `docs/operations/development.md`: records Linux build/install and manual OAuth/live-verification steps.

### Task 1: Permit CSSwitch-Owned Auth State Commands on Linux

**Files:**
- Modify: `desktop/gateway/src/codex_auth/cli.rs`
- Test: `desktop/gateway/src/codex_auth/cli.rs`

**Interfaces:**
- Consumes: `state_root_from_home(home: &Path) -> PathBuf`, `production_status(PathBuf)`, and logout functions from `codex_auth/mod.rs`.
- Produces: `production_state_root_from(home: Option<OsString>) -> Result<PathBuf, StorageError>` on macOS and Linux; `run_cli` supports `status|logout` on both platforms.

- [ ] **Step 1: Write Linux state-root and platform-policy tests**

Add imports and tests in `cli.rs`:

```rust
use std::ffi::OsString;

#[test]
fn production_state_root_requires_absolute_home() {
    assert!(production_state_root_from(None).is_err());
    assert!(production_state_root_from(Some(OsString::from("relative"))).is_err());
    assert_eq!(
        production_state_root_from(Some(OsString::from("/srv/user"))).unwrap(),
        PathBuf::from("/srv/user").join(super::super::CODEX_STATE_DIR_NAME)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_status_reaches_csswitch_repository_instead_of_platform_rejection() {
    let run = run_cli(&["status".into()]);
    let value: Value = serde_json::from_str(&run.json).unwrap();
    assert_ne!(value["error"]["code"], "unsupported_platform");
}
```

- [ ] **Step 2: Run the focused tests and confirm red**

Run: `cargo test --manifest-path desktop/gateway/Cargo.toml codex_auth::cli::tests::production_state_root_requires_absolute_home -- --exact`

Expected: compile failure because `production_state_root_from` does not exist.

- [ ] **Step 3: Broaden only private-state operations to macOS and Linux**

Replace the current platform branches and state-root function with:

```rust
#[cfg(any(target_os = "macos", target_os = "linux"))]
{
    let state_root = match production_state_root() {
        Ok(root) => root,
        Err(error) => return oauth_error_run(command, error.into()),
    };
    let logout_local_only = command == Command::Logout
        && std::env::var("CSSWITCH_CODEX_LOGOUT_SKIP_REVOKE").as_deref()
            == Ok("proxy_config_invalid");
    return run_cli_with(
        command,
        now_seconds(),
        &ProductionCommands { state_root, logout_local_only },
        logout_local_only.then_some(WarningView {
            code: "revoke_skipped",
            reason: "proxy_config_invalid",
        }),
    );
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
oauth_error_run(command, OAuthFlowError::from(StorageError::UnsupportedPlatform))

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn production_state_root_from(home: Option<std::ffi::OsString>) -> Result<PathBuf, StorageError> {
    let home = home
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| StorageError::InvalidState("HOME is unavailable or not absolute".into()))?;
    Ok(super::state_root_from_home(&home))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn production_state_root() -> Result<PathBuf, StorageError> {
    production_state_root_from(std::env::var_os("HOME"))
}
```

Keep `run_streaming_cli`'s `#[cfg(not(target_os = "macos"))]` rejection unchanged.

- [ ] **Step 4: Run focused and storage tests**

Run: `cargo test --manifest-path desktop/gateway/Cargo.toml codex_auth::cli -- --nocapture`

Expected: all CLI tests pass on Linux; `linux_status...` may report missing state but never `unsupported_platform`.

Run: `cargo test --manifest-path desktop/gateway/Cargo.toml codex_auth::storage -- --nocapture`

Expected: all private-file permission, atomic-write, and symlink tests pass.

- [ ] **Step 5: Commit**

```bash
git add desktop/gateway/src/codex_auth/cli.rs
git commit -m "feat: enable Codex auth state commands on Linux"
```

### Task 2: Add an Exact-Port Headless Login Engine

**Files:**
- Modify: `desktop/gateway/src/codex_auth/login_async.rs`
- Modify: `desktop/gateway/src/codex_auth/mod.rs`
- Test: `desktop/gateway/src/codex_auth/login_async.rs`

**Interfaces:**
- Consumes: existing `run_browser_login_with_launcher`, `LoginControl`, `LoginProgress`, `AuthRepository`, and production `CodexHttpClientFactory`.
- Produces: `run_production_login_headless(state_root: PathBuf, callback_port: u16, control: &LoginControl, progress: F, show_url: U) -> Result<AuthStatus, OAuthFlowError>` where `F: Fn(LoginProgress)` and `U: Fn(&str) -> Result<(), OAuthFlowError>`.

- [ ] **Step 1: Write tests for the port allow-list, exact binding, and one URL emission**

Add tests beside existing async-login tests:

```rust
#[test]
fn headless_port_policy_is_closed() {
    assert!(validate_headless_callback_port(1455).is_ok());
    assert!(validate_headless_callback_port(1457).is_ok());
    for port in [0, 80, 1456, 3000, u16::MAX] {
        let error = validate_headless_callback_port(port).unwrap_err();
        assert_eq!(error.code, OAuthErrorCode::OAuthProtocol);
        assert_eq!(error.stage, "callback_bind");
    }
}

#[tokio::test]
async fn occupied_exact_port_emits_no_authorization_url() {
    let occupied = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = occupied.local_addr().unwrap().port();
    let options = AsyncLoginOptions::headless_for_test(port, Duration::from_millis(50));
    let urls = Arc::new(Mutex::new(Vec::<String>::new()));
    let launcher = HeadlessUrlLauncher::new({
        let urls = urls.clone();
        move |url| { urls.lock().unwrap().push(url.to_string()); Ok(()) }
    });
    let result = run_browser_login_with_launcher(
        &test_repository(), &test_client(), false, &options,
        &LoginControl::default(), &|_| {}, &launcher,
    ).await;
    assert_eq!(result.unwrap_err().code, OAuthErrorCode::CallbackUnavailable);
    assert!(urls.lock().unwrap().is_empty());
}
```

Extend an existing successful callback test to assert `urls.len() == 1`, its redirect URI uses the requested port, and neither the repository's status nor terminal error contains the URL.

- [ ] **Step 2: Run the tests and confirm red**

Run: `cargo test --manifest-path desktop/gateway/Cargo.toml codex_auth::login_async::tests::headless_port_policy_is_closed -- --exact`

Expected: compile failure for `validate_headless_callback_port` and `HeadlessUrlLauncher`.

- [ ] **Step 3: Implement the allow-list, URL sink, and production wrapper**

Add this policy and launcher in `login_async.rs`:

```rust
pub(crate) fn validate_headless_callback_port(port: u16) -> Result<(), OAuthFlowError> {
    if CALLBACK_PORTS.contains(&port) {
        Ok(())
    } else {
        Err(OAuthFlowError::new(
            OAuthErrorCode::OAuthProtocol,
            false,
            "The headless callback port must be 1455 or 1457",
        ).at_stage("callback_bind"))
    }
}

struct HeadlessUrlLauncher<U> { show_url: U }

impl<U> HeadlessUrlLauncher<U> {
    fn new(show_url: U) -> Self { Self { show_url } }
}

impl<U> BrowserLauncher for HeadlessUrlLauncher<U>
where U: Fn(&str) -> Result<(), OAuthFlowError> {
    fn open<'a>(&'a self, url: &'a str, _: &'a LoginControl)
        -> Pin<Box<dyn Future<Output = Result<(), OAuthFlowError>> + 'a>> {
        Box::pin(async move { (self.show_url)(url) })
    }
}
```

Add `AsyncLoginOptions::headless(port)` with `callback_ports: vec![port]` and the existing five-minute timeout. Extract the existing production HTTP-client construction into `production_client() -> Result<(reqwest::Client, bool), OAuthFlowError>` so browser and headless wrappers share proxy/TLS behavior. Call `validate_headless_callback_port` before client construction, mutation locking, listener binding, or URL generation.

Expose from `mod.rs`:

```rust
pub async fn run_production_login_headless<F, U>(
    state_root: PathBuf,
    callback_port: u16,
    control: &LoginControl,
    progress: F,
    show_url: U,
) -> Result<AuthStatus, OAuthFlowError>
where
    F: Fn(LoginProgress),
    U: Fn(&str) -> Result<(), OAuthFlowError>,
{
    let repository = storage::AuthRepository::production(state_root);
    login_async::run_production_login_headless(
        &repository, callback_port, control, progress, show_url,
    ).await
}
```

Both production wrappers must call `control.finish()` exactly once after the shared login future returns.

- [ ] **Step 4: Run all async OAuth tests**

Run: `cargo test --manifest-path desktop/gateway/Cargo.toml codex_auth::login_async -- --nocapture`

Expected: existing denial, state mismatch, timeout, cancellation, and atomic-commit tests plus the new exact-port tests all pass.

- [ ] **Step 5: Commit**

```bash
git add desktop/gateway/src/codex_auth/login_async.rs desktop/gateway/src/codex_auth/mod.rs
git commit -m "feat: add exact-port headless Codex OAuth engine"
```

### Task 3: Add the Secret-Free `login-headless` CLI

**Files:**
- Modify: `desktop/gateway/Cargo.toml`
- Modify: `desktop/gateway/src/codex_auth/cli.rs`
- Modify: `desktop/gateway/src/codex_auth/mod.rs`
- Modify: `desktop/gateway/src/main.rs`
- Test: `desktop/gateway/src/codex_auth/cli.rs`

**Interfaces:**
- Consumes: Task 2 `run_production_login_headless` and existing `LoginControl::cancel()` commit-aware state machine.
- Produces: `run_headless_cli(args: &[String]) -> Option<CliRun>`; `HeadlessArgs { callback_port: u16 }`; exactly one stderr URL; one terminal JSON object on stdout.

- [ ] **Step 1: Write parser and output-contract tests**

Add table-driven tests:

```rust
#[test]
fn headless_arguments_are_explicit_and_bounded() {
    for args in [
        vec!["login-headless"],
        vec!["login-headless", "--show-url"],
        vec!["login-headless", "--callback-port", "1455"],
        vec!["login-headless", "--callback-port", "1456", "--show-url"],
        vec!["login-headless", "--callback-port", "1455", "--show-url", "extra"],
    ] {
        assert!(parse_headless_args(&args.into_iter().map(String::from).collect::<Vec<_>>()).is_err());
    }
    assert_eq!(
        parse_headless_args(&["login-headless".into(), "--callback-port".into(), "1455".into(), "--show-url".into()]).unwrap(),
        HeadlessArgs { callback_port: 1455 }
    );
}

#[test]
fn authorization_url_writer_emits_once_to_only_its_sink() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = AuthorizationUrlWriter::new(bytes.clone());
    writer.show("https://auth.example/authorize?state=private").unwrap();
    assert!(writer.show("https://auth.example/second").is_err());
    let output = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert_eq!(output.matches("https://auth.example/").count(), 1);
    assert!(output.ends_with('\n'));
}
```

Add a test executor seam that returns a fake `AuthStatus`; assert the returned `CliRun.json` contains `"command":"login-headless"` and contains none of `https://`, `state=`, `access_token`, `refresh_token`, or the account hash.

- [ ] **Step 2: Run the focused tests and confirm red**

Run: `cargo test --manifest-path desktop/gateway/Cargo.toml codex_auth::cli::tests::headless_arguments_are_explicit_and_bounded -- --exact`

Expected: compile failure because the parser and writer are absent.

- [ ] **Step 3: Implement parsing and one-shot stderr writing**

Add:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HeadlessArgs { callback_port: u16 }

fn parse_headless_args(args: &[String]) -> Result<HeadlessArgs, ()> {
    match args {
        [command, port_flag, port, show]
            if command == "login-headless"
                && port_flag == "--callback-port"
                && show == "--show-url" => match port.parse::<u16>() {
                    Ok(callback_port @ (1455 | 1457)) => Ok(HeadlessArgs { callback_port }),
                    _ => Err(()),
                },
        _ => Err(()),
    }
}

#[derive(Clone)]
struct AuthorizationUrlWriter<W> {
    inner: Arc<Mutex<W>>,
    emitted: Arc<std::sync::atomic::AtomicBool>,
}
```

Implement `show(&self, url: &str)` with `compare_exchange(false, true, ...)`, one `writeln!`, and flush. On a second call or I/O failure return a redacted `OAuthFlowError` at stage `browser_open`; never embed the URL in an error.

- [ ] **Step 4: Implement Ctrl-C orchestration and final JSON**

Add Tokio's `signal` feature in `desktop/gateway/Cargo.toml`:

```toml
tokio = { version = "1", features = ["io-util", "macros", "net", "rt", "signal", "time"] }
```

Implement `run_headless_cli` only under macOS/Linux state storage. Build a current-thread Tokio runtime, pin the login future, and use:

```rust
let result = runtime.block_on(async {
    let login = run_production_login_headless(
        state_root, parsed.callback_port, &control, |_| {}, move |url| stderr.show(url),
    );
    tokio::pin!(login);
    tokio::select! {
        result = &mut login => result,
        signal = tokio::signal::ctrl_c() => {
            if signal.is_err() {
                Err(OAuthFlowError::new(OAuthErrorCode::OAuthNetwork, false,
                    "The interrupt handler could not be installed").at_stage("callback_wait"))
            } else {
                let _ = control.cancel();
                login.await
            }
        }
    }
});
```

Refactor error serialization through a string-taking helper while preserving the existing wrapper:

```rust
fn oauth_error_run(command: Command, error: OAuthFlowError) -> CliRun {
    oauth_error_run_for(command.as_str(), error)
}

fn oauth_error_run_for(command: &'static str, error: OAuthFlowError) -> CliRun {
    let envelope = ErrorEnvelope {
        schema_version: CLI_SCHEMA_VERSION,
        ok: false,
        command: Some(command),
        error: ErrorView {
            code: error.code.as_str(),
            message: error.message,
            retryable: error.retryable,
            stage: Some(error.stage),
            upstream_status: error.upstream_status,
            response_kind: error.response_kind,
            challenge_detected: error.challenge_detected,
            transport_kind: error.transport_kind,
        },
    };
    match serde_json::to_string(&envelope) {
        Ok(json) => CliRun { json, exit_code: exit_code(error.code) },
        Err(_) => internal_serialization_error(),
    }
}
```

Add `headless_success_run(status: &AuthStatus) -> CliRun`, constructing the existing schema-v3 success envelope with `command: "login-headless"` and `account_hash: None`; use `oauth_error_run_for("login-headless", error)` for failures. Both paths produce one-line JSON and expose no authorization URL or account identifier. Export `run_headless_cli` from `mod.rs`.

Route in `main.rs` before `run_streaming_cli`:

```rust
if let Some(run) = csswitch_gateway::codex_auth::run_headless_cli(&args) {
    println!("{}", run.json);
    std::process::exit(run.exit_code);
}
```

Update invalid usage to `login-browser|login-headless|status|logout`. Do not route `login-headless` through `run_streaming_cli` and do not loosen its macOS cfg.

- [ ] **Step 5: Run CLI and regression tests**

Run: `cargo test --manifest-path desktop/gateway/Cargo.toml codex_auth::cli -- --nocapture`

Expected: all tests pass, including Linux rejection of `login-browser`, acceptance of headless syntax, one-shot stderr, and secret-free JSON.

Run: `cargo test --manifest-path desktop/gateway/Cargo.toml codex_auth -- --nocapture`

Expected: all auth lifecycle and storage tests pass.

- [ ] **Step 6: Commit**

```bash
git add desktop/gateway/Cargo.toml desktop/gateway/src/codex_auth/cli.rs desktop/gateway/src/codex_auth/mod.rs desktop/gateway/src/main.rs
git commit -m "feat: expose headless Codex OAuth CLI"
```

### Task 4: Build the Linux Runtime Controller with Ownership Guards

**Files:**
- Create: `scripts/csswitch-codex`
- Create: `test/test_headless_codex.sh`
- Modify: `test/run-scripts.sh`

**Interfaces:**
- Consumes: `csswitch-gateway codex-auth status`, Gateway `--provider codex --port N` with `CSSWITCH_AUTH_TOKEN` supplied only through the environment, `/<secret>/health`, `/<secret>/v1/models`, and Science `serve`/`status`/`stop` commands.
- Produces: `csswitch-codex start|stop|status`; private `runtime.env`, `gateway.pid`, and `science.pid` under `CSSWITCH_HEADLESS_DIR`.

- [ ] **Step 1: Create a fake-runtime shell test harness**

Create `test/test_headless_codex.sh` with a `mktemp -d` fixture, trap cleanup, fake `csswitch-gateway`, fake `claude-science`, fake `curl`, and fake `ss`. Export these supported seams:

```bash
export CSSWITCH_GATEWAY_BIN="$fixture/bin/csswitch-gateway"
export CSSWITCH_SCIENCE_BIN="$fixture/bin/claude-science"
export CSSWITCH_CURL_BIN="$fixture/bin/curl"
export CSSWITCH_SS_BIN="$fixture/bin/ss"
export CSSWITCH_HEADLESS_DIR="$fixture/home/.csswitch/headless"
export CSSWITCH_SCIENCE_DATA_DIR="$fixture/science-data"
export CSSWITCH_GATEWAY_PORT=11434
export CSSWITCH_SCIENCE_PORT=9002
```

The fake commands append only operation names to `$fixture/events`; they must deliberately fail if an auth token, path secret, authorization URL, or Science URL is written there.

- [ ] **Step 2: Add failing behavior tests**

Exercise these exact cases in separate fixture resets:

```bash
assert_events 'auth gateway health catalog science'
assert_mode "$CSSWITCH_HEADLESS_DIR" 700
assert_mode "$CSSWITCH_HEADLESS_DIR/runtime.env" 600
assert_not_contains "$start_output" 'CSSWITCH_AUTH_TOKEN\|/v1/messages\|https://'

CATALOG_RESULT=fail "$controller" start && fail 'catalog failure was accepted'
assert_events 'auth gateway health catalog gateway-stop'

SS_OWNER=unknown "$controller" start && fail 'unknown owner was accepted'
assert_events ''

PID_MATCH=mismatch "$controller" stop && fail 'mismatched recorded PID was killed'
assert_events 'verify-failed'

status_output="$($controller status 2>&1)"
assert_not_contains "$status_output" 'CSSWITCH_AUTH_TOKEN\|/v1/messages\|https://'
```

Also scan the controller source for forbidden fallback/process patterns:

```bash
! grep -Eiq 'kimi|pkill[[:space:]]+-f|watchdog|systemctl' "$controller"
```

- [ ] **Step 3: Run the shell test and confirm red**

Run: `bash test/test_headless_codex.sh`

Expected: failure because `scripts/csswitch-codex` does not exist.

- [ ] **Step 4: Implement configuration, private secret creation, and redacted auth**

Create `scripts/csswitch-codex` with `#!/usr/bin/env bash` and `set -euo pipefail`. Use these exact production defaults while preserving the test seams:

```bash
gateway_bin=${CSSWITCH_GATEWAY_BIN:-/home/bio-13/.local/bin/csswitch-gateway}
science_bin=${CSSWITCH_SCIENCE_BIN:-/home/bio-13/.local/bin/claude-science}
curl_bin=${CSSWITCH_CURL_BIN:-curl}
ss_bin=${CSSWITCH_SS_BIN:-ss}
state_dir=${CSSWITCH_HEADLESS_DIR:-"${HOME:?}/.csswitch/headless"}
science_data=${CSSWITCH_SCIENCE_DATA_DIR:-/work/run/projects/bio-13/.claude-science}
gateway_port=${CSSWITCH_GATEWAY_PORT:-11434}
science_port=${CSSWITCH_SCIENCE_PORT:-9002}
```

Set `umask 077`, create and validate `state_dir` as a non-symlink directory mode `0700`, then atomically create `runtime.env` from 32 random bytes:

```bash
secret=$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')
tmp="$state_dir/runtime.env.$$"
printf 'CSSWITCH_AUTH_TOKEN=%s\n' "$secret" >"$tmp"
chmod 600 "$tmp"
mv "$tmp" "$state_dir/runtime.env"
```

Load it without exporting or printing it. Validate the line against `^CSSWITCH_AUTH_TOKEN=[0-9a-f]{64}$`. Auth succeeds only when one-line `codex-auth status` JSON contains both `"ok":true` and `"authenticated":true`; preserve the JSON only in a mode-600 temporary file and delete it after parsing.

- [ ] **Step 5: Implement listener and recorded-PID verification**

Implement `listener_pid(port)` by invoking `ss -H -ltnp "sport = :$port"`, requiring zero or one `pid=N` result. Implement `verify_pid(kind, pid, port)` with all of:

```bash
test "$pid" -eq "$pid" 2>/dev/null
test -r "/proc/$pid/status" -a -r "/proc/$pid/cmdline" -a -e "/proc/$pid/exe"
test "$(awk '/^Uid:/{print $2}' "/proc/$pid/status")" = "$(id -u)"
test "$(readlink -f "/proc/$pid/exe")" = "$(readlink -f "$expected_bin")"
tr '\0' '\n' <"/proc/$pid/cmdline" | grep -Fx -- "$expected_argument"
test "$(listener_pid "$port")" = "$pid"
```

For Gateway, `expected_argument` is `codex` plus the exact port argument. For Science it is the exact `science_data` path plus the exact port. Reject symlink/non-regular PID files and require mode `0600`. A nonempty listener without a matching verified PID record is an unknown owner and must abort without signals.

- [ ] **Step 6: Implement ordered start and failure cleanup**

`start` performs this order and no other provider path:

1. Verify binaries are regular executable files and `science_data` is an existing non-symlink directory.
2. Verify redacted Codex auth.
3. Refuse unknown owners on ports `11434` and `9002`.
4. Start Gateway with inherited `HTTP_PROXY`/`HTTPS_PROXY`, `CSSWITCH_AUTH_TOKEN` in its environment, `--provider codex --port "$gateway_port"`, redirecting logs to mode-600 files; write its PID atomically.
5. Poll `http://127.0.0.1:$gateway_port/$secret/health` without verbose curl.
6. Fetch `/$secret/v1/models` to a mode-600 temporary file; require a nonempty `data` array and at least one `Codex / ` alias, then delete it.
7. Stop a currently recorded and verified Science process only after catalog success.
8. Start Science with `ANTHROPIC_BASE_URL=http://127.0.0.1:$gateway_port/$secret`, `HTTPS_PROXY=http://127.0.0.1:$gateway_port`, matching lowercase variables, loopback `NO_PROXY`, and:

```bash
"$science_bin" serve --data-dir "$science_data" --host 127.0.0.1 \
  --port "$science_port" --sandbox-port 9003 --no-browser --no-auto-update --detached
```

Resolve the detached listener PID through `ss`, verify it, and then write `science.pid`. Any failure after Gateway launch sends TERM only to the newly recorded and verified Gateway, waits a bounded interval, and removes its PID file. It never starts Science after health/catalog failure.

- [ ] **Step 7: Implement stop and status**

`stop` verifies each PID before TERM, waits up to ten seconds, uses KILL only on the same still-verified PID, then removes its record. Stop Science before Gateway. If either record is stale/mismatched, return nonzero without signaling it.

`status` emits only labels such as:

```text
auth=ready
gateway=running health=ready catalog=ready
science=running listener=ready
```

It must not print command lines, environment, request URLs, model payloads, OAuth JSON, log contents, or the output of `claude-science url`.

- [ ] **Step 8: Register and run the shell tests**

Append to `test/run-scripts.sh` using the file's existing `fail` accumulator pattern:

```bash
bash test/test_headless_codex.sh || fail=1
```

Run: `bash test/test_headless_codex.sh`

Expected: PASS for ordered start, catalog cleanup, unknown owners, stale PIDs, modes, and redaction.

Run: `bash test/run-scripts.sh`

Expected: `current-env clean` or the repository's equivalent successful script-layer result; record environment-only skips separately.

- [ ] **Step 9: Commit**

```bash
git add scripts/csswitch-codex test/test_headless_codex.sh test/run-scripts.sh
git commit -m "feat: manage headless Codex Science runtime"
```

### Task 5: Document, Build, and Verify the Linux Distribution

**Files:**
- Modify: `docs/features/codex-science-bridge.md`
- Modify: `docs/operations/development.md`

**Interfaces:**
- Consumes: Tasks 1-4 CLI and controller contracts.
- Produces: exact user-run OAuth instructions, build/install commands, rollback boundary, and test evidence.

- [ ] **Step 1: Add documentation contract checks to the shell test**

Add assertions that the feature docs include the exact headless command, explicitly say the user runs it, use `claude-science url` directly, and contain no Kimi fallback instruction:

```bash
grep -Fq 'codex-auth login-headless --callback-port 1455 --show-url' docs/features/codex-science-bridge.md
grep -Fq 'Run this OAuth command yourself' docs/features/codex-science-bridge.md
grep -Fq 'claude-science url' docs/features/codex-science-bridge.md
! grep -Eiq 'fallback.*kimi|kimi.*fallback' docs/features/codex-science-bridge.md
```

- [ ] **Step 2: Run the test and confirm red**

Run: `bash test/test_headless_codex.sh`

Expected: failure because the headless documentation is absent.

- [ ] **Step 3: Write the operator documentation**

Add a `Headless Linux OAuth` section documenting:

```bash
ssh -L 1455:127.0.0.1:1455 user@server
csswitch-gateway codex-auth login-headless --callback-port 1455 --show-url
csswitch-codex start
csswitch-codex status
claude-science url
```

State that only the OAuth user opens the first URL; only trusted collaborators receive the Science URL; collaborators share that Science instance's workspace/history and the Codex subscription's usage authority. Explain that OAuth alone never starts Science, `stop` is the emergency access cutoff, and rollback requires manually choosing the backed-up prior setup because this controller has no provider fallback.

In development docs add:

```bash
cargo build --release --manifest-path desktop/gateway/Cargo.toml
install -m 0755 desktop/gateway/target/release/csswitch-gateway ~/.local/bin/csswitch-gateway
install -m 0755 scripts/csswitch-codex ~/.local/bin/csswitch-codex
```

- [ ] **Step 4: Run formatting, lint, focused, and full repository tests**

Run:

```bash
cargo fmt --manifest-path desktop/gateway/Cargo.toml -- --check
cargo clippy --manifest-path desktop/gateway/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path desktop/gateway/Cargo.toml
bash test/run_all.sh
```

Expected: formatting, clippy, and Cargo tests pass. Report `run_all.sh` with the repository's `current-env clean` versus `release-ready green` vocabulary and list any documented machine-only gates instead of calling them failures.

- [ ] **Step 5: Audit outputs and source for secret/fallback regressions**

Run:

```bash
rg -n 'pkill -f|systemctl|watchdog|kimi|\.codex' scripts/csswitch-codex desktop/gateway/src/codex_auth docs/features/codex-science-bridge.md
rg -n 'authorization_url|access_token|refresh_token|CSSWITCH_AUTH_TOKEN' desktop/gateway/src/codex_auth/cli.rs scripts/csswitch-codex
git diff --check
git status --short
```

Expected: the first command finds no prohibited runtime behavior (`.codex` may occur only in an explicit prohibition in docs); the second finds only internal variable handling/tests and no printing; `git diff --check` is silent.

- [ ] **Step 6: Commit documentation and any formatting-only changes**

```bash
git add docs/features/codex-science-bridge.md docs/operations/development.md desktop/gateway
git commit -m "docs: add Linux headless Codex operations"
```

### Task 6: Install and Perform the User-Controlled Live Handoff

**Files:**
- Install outside repository: `/home/bio-13/.local/bin/csswitch-gateway`
- Install outside repository: `/home/bio-13/.local/bin/csswitch-codex`
- Runtime creation by user command: `/home/bio-13/.csswitch` and `/home/bio-13/.csswitch/headless`

**Interfaces:**
- Consumes: release binary and controller from Tasks 3-5.
- Produces: authenticated Codex-only Science runtime and redacted live verification evidence.

- [ ] **Step 1: Build and install without touching the backup checkout**

Run:

```bash
cargo build --release --manifest-path desktop/gateway/Cargo.toml
install -m 0755 desktop/gateway/target/release/csswitch-gateway /home/bio-13/.local/bin/csswitch-gateway
install -m 0755 scripts/csswitch-codex /home/bio-13/.local/bin/csswitch-codex
```

Expected: both installed files are owned by the current user and mode `0755`. Do not modify `/work/run/projects/bio-13/CSswitch` or its backup.

- [ ] **Step 2: Hand the OAuth command to the user and stop execution**

Tell the user to run personally, in their own SSH terminal with the `1455` tunnel active:

```bash
/home/bio-13/.local/bin/csswitch-gateway codex-auth login-headless \
  --callback-port 1455 --show-url
```

Expected: the URL appears once in that terminal's stderr; the user opens it locally and completes login; terminal stdout ends with one secret-free JSON result. The implementing agent must not execute, capture, quote, or ask the user to paste that URL.

- [ ] **Step 3: After the user confirms completion, verify redacted auth**

Run:

```bash
/home/bio-13/.local/bin/csswitch-gateway codex-auth status
```

Expected: schema-v3 one-line JSON with `"ok":true`, `"authenticated":true`, a valid/expiring expiry state, and no token or URL fields. Do not display account hashes in the handoff summary.

- [ ] **Step 4: Start and verify the owned runtime**

Run:

```bash
/home/bio-13/.local/bin/csswitch-codex start
/home/bio-13/.local/bin/csswitch-codex status
```

Expected: auth, Gateway health, dynamic catalog, and Science listener report ready on ports `11434` and `9002`. Do not print the local path secret, model-catalog payload, or Science URL.

- [ ] **Step 5: Perform the already authorized minimal live path test**

Use the normal Science request interface with the smallest prompt that proves a response, such as `Reply with exactly: OK`, selecting one alias returned by the live dynamic catalog. Record only pass/fail, the selected public alias, HTTP/result status, and whether the response matched; do not record bearer URLs, headers, OAuth material, account identifiers, or raw logs.

Expected: one successful Science -> Gateway -> Codex response. Any Codex/auth/catalog/translation failure remains a failure with no Kimi retry.

- [ ] **Step 6: Final repository and rollback report**

Run:

```bash
git status --short
git log --oneline --decorate -8
```

Report local commits, tests, installation paths, owned runtime state, and live-provider evidence separately. State that no push occurred and rollback is `csswitch-codex stop` followed by a manually selected prior setup; do not alter or delete the preserved backup.
