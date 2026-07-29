# Provider Failure Contract Seam Prototype Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and interactively validate a throwaway Rust state-machine prototype for the Provider Failure Contract seam, then record the human verdict in the Wayfinder map.

**Architecture:** A pure `AttemptController` consumes sanitized `FailureObservation` values and returns `AttemptDirective` values while owning retry, repair, and response-started state. A thin terminal shell lets the user inject observations and inspect state and the Anthropic-compatible error envelope; it performs no network, OAuth, filesystem persistence, or production gateway mutation.

**Tech Stack:** Rust 2021, existing `serde_json`, Cargo example target, ANSI terminal rendering, local Markdown Wayfinder tracker.

## Global Constraints

- Execute prototype work in an isolated worktree on branch `prototype/provider-failure-contract-seam`, created with the `using-git-worktrees` skill from commit `6ae73409c06c3fdbd73c090b31dfabcefcbe52d0` or its reviewed descendant.
- Do not modify production gateway modules, dependencies, OAuth state, proxy configuration, or live Provider Routes.
- Do not add automated tests to the throwaway prototype; validate with compilation plus the exact interactive scenarios in Task 3.
- Provider adapters retain I/O; the prototype models deterministic policy and state only.
- Failure Observation and Provider Failure values must not accept request bodies, response bodies, prompts, credentials, account identifiers, cookies, arbitrary upstream strings, or private URLs.
- Existing Anthropic-compatible `type`, `error.type`, and `error.message` fields remain present.
- A Safe Repair is a closed enum value, explicitly enabled, semantics-preserving, and usable at most once.
- No retry or repair is legal after response bytes begin.
- Numeric backoff values are prototype inputs, not production defaults.

---

## File structure

- Create `desktop/gateway/examples/provider_failure_prototype/README.md` — prototype question, run command, keyboard actions, scenario script, and accepted verdict.
- Create `desktop/gateway/examples/provider_failure_prototype/controller.rs` — pure data types, state machine, classification, retry/repair budgets, and envelope serialization.
- Create `desktop/gateway/examples/provider_failure_prototype/main.rs` — terminal rendering and input dispatch only.
- Modify `.scratch/provider-failure-contract/issues/02-choose-provider-contract-seam.md` — record the human-reviewed answer and prototype branch pointer after acceptance.
- Modify `.scratch/provider-failure-contract/map.md` — append the one-line decision pointer after the ticket resolves.

### Task 1: Scaffold the isolated state viewer

**Files:**
- Create: `desktop/gateway/examples/provider_failure_prototype/README.md`
- Create: `desktop/gateway/examples/provider_failure_prototype/controller.rs`
- Create: `desktop/gateway/examples/provider_failure_prototype/main.rs`

**Interfaces:**
- Consumes: the approved design in `docs/superpowers/specs/2026-07-29-provider-failure-contract-seam-design.md`.
- Produces: `AttemptController::begin_post`, `AttemptController::mark_response_started`, `AttemptController::reset`, and read-only state rendering used by Task 2.

- [ ] **Step 1: Create the isolated worktree**

Invoke the `using-git-worktrees` skill, then create or select a clean worktree for `prototype/provider-failure-contract-seam`. Verify:

```bash
git status --short --branch
git rev-parse HEAD
```

Expected: clean branch `prototype/provider-failure-contract-seam`, based on reviewed commit `6ae73409c06c3fdbd73c090b31dfabcefcbe52d0` or a descendant containing only reviewed documentation changes.

- [ ] **Step 2: Record the prototype question and one-command entrypoint**

Create `desktop/gateway/examples/provider_failure_prototype/README.md`:

````markdown
# Provider Failure Contract Seam Prototype

PROTOTYPE — throwaway terminal shell; never ship this example as gateway runtime code.

## Question

Does a pure Attempt Controller provide a sufficiently small interface to centralize Provider Failure classification, retry and Safe Repair budgets, response-started rules, and Anthropic-compatible serialization while leaving Provider Route adapters in control of protocol-specific normalization and I/O?

## Run

```bash
cargo run --offline --manifest-path desktop/gateway/Cargo.toml --example provider_failure_prototype
```

The prototype is in-memory only. It performs no network, OAuth, proxy, or filesystem-persistence operations.
````

- [ ] **Step 3: Create the initial pure controller state**

Create `desktop/gateway/examples/provider_failure_prototype/controller.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_posts: u8,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub retry_after_cap_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_posts: 3,
            base_delay_ms: 500,
            max_delay_ms: 2_000,
            retry_after_cap_ms: 60_000,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttemptState {
    pub posts_started: u8,
    pub repairs_used: u8,
    pub response_started: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteContext {
    pub provider: String,
    pub route: String,
    pub correlation_id: String,
    pub policy: RetryPolicy,
}

#[derive(Clone, Debug)]
pub struct AttemptController {
    context: RouteContext,
    state: AttemptState,
}

impl AttemptController {
    pub fn new(context: RouteContext) -> Self {
        Self {
            context,
            state: AttemptState::default(),
        }
    }

    pub fn context(&self) -> &RouteContext {
        &self.context
    }

    pub fn state(&self) -> &AttemptState {
        &self.state
    }

    pub fn begin_post(&mut self) -> bool {
        if self.state.response_started
            || self.state.posts_started >= self.context.policy.max_posts
        {
            return false;
        }
        self.state.posts_started += 1;
        true
    }

    pub fn mark_response_started(&mut self) -> bool {
        if self.state.posts_started == 0 {
            return false;
        }
        self.state.response_started = true;
        true
    }

    pub fn reset(&mut self) {
        self.state = AttemptState::default();
    }
}
```

- [ ] **Step 4: Create the initial terminal shell**

Create `desktop/gateway/examples/provider_failure_prototype/main.rs`:

```rust
mod controller;

use std::io::{self, Write};

use controller::{AttemptController, RouteContext, RetryPolicy};

fn render(controller: &AttemptController) {
    print!("\x1b[2J\x1b[H");
    println!("\x1b[1mProvider Failure Contract Prototype\x1b[0m");
    println!("\x1b[2mNo network, OAuth, or persistence\x1b[0m\n");
    println!("\x1b[1mprovider\x1b[0m: {}", controller.context().provider);
    println!("\x1b[1mroute\x1b[0m: {}", controller.context().route);
    println!(
        "\x1b[1mcorrelation_id\x1b[0m: {}",
        controller.context().correlation_id
    );
    println!("\x1b[1mstate\x1b[0m: {:#?}", controller.state());
    println!("\n\x1b[1m[p]\x1b[0m begin POST  \x1b[1m[b]\x1b[0m bytes started");
    println!("\x1b[1m[r]\x1b[0m reset       \x1b[1m[q]\x1b[0m quit");
    print!("> ");
    io::stdout().flush().expect("flush prototype frame");
}

fn main() {
    let context = RouteContext {
        provider: "codex".into(),
        route: "responses_lite".into(),
        correlation_id: "prototype-0001".into(),
        policy: RetryPolicy::default(),
    };
    let mut controller = AttemptController::new(context);
    loop {
        render(&controller);
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            break;
        }
        match input.trim() {
            "p" => {
                controller.begin_post();
            }
            "b" => {
                controller.mark_response_started();
            }
            "r" => controller.reset(),
            "q" => break,
            _ => {}
        }
    }
}
```

- [ ] **Step 5: Compile and manually verify monotonic state**

Run:

```bash
cargo check --offline --manifest-path desktop/gateway/Cargo.toml --example provider_failure_prototype
cargo run --offline --manifest-path desktop/gateway/Cargo.toml --example provider_failure_prototype
```

Expected: Cargo exits successfully. In the terminal, `p`, `p`, `b`, `p` leaves `posts_started: 2` and `response_started: true`; the final `p` cannot increment the count. `r` restores all fields to zero/false.

- [ ] **Step 6: Commit the state viewer**

```bash
git add desktop/gateway/examples/provider_failure_prototype
git commit -m "prototype: scaffold provider failure controller"
```

Expected: one commit containing only the three prototype files.

### Task 2: Add classification, directives, and envelopes

**Files:**
- Modify: `desktop/gateway/examples/provider_failure_prototype/controller.rs`
- Modify: `desktop/gateway/examples/provider_failure_prototype/main.rs`
- Modify: `desktop/gateway/examples/provider_failure_prototype/README.md`

**Interfaces:**
- Consumes: `AttemptController`, `RouteContext`, `RetryPolicy`, and `AttemptState` from Task 1.
- Produces: `AttemptController::observe(FailureObservation) -> AttemptDirective`, `ProviderFailure::anthropic_json() -> serde_json::Value`, `AttemptController::last_observation`, and `AttemptController::last_directive`.

- [ ] **Step 1: Add the closed observation and directive vocabulary**

Extend `controller.rs` with these public types before `AttemptController`:

```rust
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateKind {
    RateLimit,
    Quota,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairKind {
    OmitUnsupportedAutomaticToolChoice,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FailureObservation {
    Capability { repair: Option<RepairKind> },
    Http {
        status: u16,
        rate_kind: Option<RateKind>,
        retry_after_ms: Option<u64>,
        repair: Option<RepairKind>,
    },
    Network,
    Protocol { repair: Option<RepairKind> },
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderFailure {
    pub status: u16,
    pub error_type: &'static str,
    pub message: &'static str,
    pub provider: String,
    pub route: String,
    pub failure_class: &'static str,
    pub upstream_status: Option<u16>,
    pub retryable: bool,
    pub correlation_id: String,
    pub recovery: &'static str,
}

impl ProviderFailure {
    pub fn anthropic_json(&self) -> Value {
        let mut error = json!({
            "type": self.error_type,
            "message": self.message,
            "provider": self.provider,
            "route": self.route,
            "failure_class": self.failure_class,
            "retryable": self.retryable,
            "correlation_id": self.correlation_id,
            "recovery": self.recovery,
        });
        if let Some(status) = self.upstream_status {
            error["upstream_status"] = json!(status);
        }
        json!({ "type": "error", "error": error })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptDirective {
    Fail(ProviderFailure),
    RetryAfter(u64),
    RepairOnce(RepairKind),
    Cancel,
}
```

- [ ] **Step 2: Replace `AttemptController` with the complete pure reducer**

Replace the Task 1 `AttemptController` definition and implementation with:

```rust
#[derive(Clone, Debug)]
pub struct AttemptController {
    context: RouteContext,
    state: AttemptState,
    enabled_repairs: Vec<RepairKind>,
    last_observation: Option<FailureObservation>,
    last_directive: Option<AttemptDirective>,
}

impl AttemptController {
    pub fn new(context: RouteContext, enabled_repairs: Vec<RepairKind>) -> Self {
        Self {
            context,
            state: AttemptState::default(),
            enabled_repairs,
            last_observation: None,
            last_directive: None,
        }
    }

    pub fn context(&self) -> &RouteContext {
        &self.context
    }

    pub fn state(&self) -> &AttemptState {
        &self.state
    }

    pub fn last_observation(&self) -> Option<&FailureObservation> {
        self.last_observation.as_ref()
    }

    pub fn last_directive(&self) -> Option<&AttemptDirective> {
        self.last_directive.as_ref()
    }

    pub fn begin_post(&mut self) -> bool {
        if self.state.response_started
            || self.state.posts_started >= self.context.policy.max_posts
        {
            return false;
        }
        self.state.posts_started += 1;
        self.last_observation = None;
        self.last_directive = None;
        true
    }

    pub fn mark_response_started(&mut self) -> bool {
        if self.state.posts_started == 0 {
            return false;
        }
        self.state.response_started = true;
        true
    }

    pub fn reset(&mut self) {
        self.state = AttemptState::default();
        self.last_observation = None;
        self.last_directive = None;
    }

    pub fn observe(&mut self, observation: FailureObservation) -> AttemptDirective {
        let directive = self.decide(&observation);
        self.last_observation = Some(observation);
        self.last_directive = Some(directive.clone());
        directive
    }

    fn decide(&mut self, observation: &FailureObservation) -> AttemptDirective {
        if matches!(observation, FailureObservation::Cancelled) {
            return AttemptDirective::Cancel;
        }
        if let Some(repair) = self.repair_from(observation) {
            if !self.state.response_started
                && self.state.repairs_used == 0
                && self.enabled_repairs.contains(&repair)
            {
                self.state.repairs_used = 1;
                return AttemptDirective::RepairOnce(repair);
            }
        }
        match observation {
            FailureObservation::Capability { .. } => self.fail(
                400,
                "invalid_request_error",
                "Provider Route capability cannot preserve this request",
                "capability",
                None,
                "Use equivalent supported request semantics or select a compatible model",
            ),
            FailureObservation::Http {
                status: 401, ..
            } => self.fail(
                401,
                "authentication_error",
                "Provider authentication failed",
                "authentication",
                Some(401),
                "Re-authenticate, then start a new caller request",
            ),
            FailureObservation::Http {
                status: 403, ..
            } => self.fail(
                403,
                "permission_error",
                "Provider authorization failed",
                "authorization",
                Some(403),
                "Check account, workspace, geography, and model entitlement",
            ),
            FailureObservation::Http {
                status: 429,
                rate_kind: Some(RateKind::RateLimit),
                retry_after_ms,
                ..
            } if self.can_retry() => AttemptDirective::RetryAfter(
                retry_after_ms
                    .unwrap_or_else(|| self.backoff_ms())
                    .min(self.context.policy.retry_after_cap_ms),
            ),
            FailureObservation::Http {
                status: 429,
                rate_kind,
                ..
            } => self.fail(
                429,
                "rate_limit_error",
                "Provider rate or quota limit prevents this request",
                if *rate_kind == Some(RateKind::Quota) {
                    "quota"
                } else {
                    "rate_limit"
                },
                Some(429),
                "Wait for rate capacity or restore account quota before a new request",
            ),
            FailureObservation::Http { status, .. }
                if matches!(*status, 408 | 409) || (500..=599).contains(status) =>
            {
                if self.can_retry() {
                    AttemptDirective::RetryAfter(self.backoff_ms())
                } else {
                    self.fail(
                        if *status == 408 { 504 } else { 502 },
                        "api_error",
                        "Provider transient failure exhausted the retry budget",
                        "transient",
                        Some(*status),
                        "Start a new request after the provider recovers",
                    )
                }
            }
            FailureObservation::Http { status, .. } if (400..=499).contains(status) => self.fail(
                *status,
                "invalid_request_error",
                "Provider rejected the request",
                "invalid_request",
                Some(*status),
                "Correct the request or select a compatible model",
            ),
            FailureObservation::Http { status, .. } => self.fail(
                502,
                "api_error",
                "Provider returned an unsupported status",
                "upstream",
                Some(*status),
                "Inspect sanitized diagnostics before starting a new request",
            ),
            FailureObservation::Network if self.can_retry() => {
                AttemptDirective::RetryAfter(self.backoff_ms())
            }
            FailureObservation::Network => self.fail(
                502,
                "api_error",
                "Provider network failure exhausted the retry budget",
                "network",
                None,
                "Check the network route before starting a new request",
            ),
            FailureObservation::Protocol { .. } => self.fail(
                502,
                "api_error",
                "Provider response violated the expected protocol",
                "protocol",
                None,
                "Inspect sanitized diagnostics and start a new request",
            ),
            FailureObservation::Cancelled => AttemptDirective::Cancel,
        }
    }

    fn can_retry(&self) -> bool {
        !self.state.response_started
            && self.state.posts_started > 0
            && self.state.posts_started < self.context.policy.max_posts
    }

    fn backoff_ms(&self) -> u64 {
        let mut delay = self.context.policy.base_delay_ms;
        for _ in 1..self.state.posts_started {
            delay = delay.saturating_mul(2);
        }
        delay.min(self.context.policy.max_delay_ms)
    }

    fn repair_from(&self, observation: &FailureObservation) -> Option<RepairKind> {
        match observation {
            FailureObservation::Capability { repair }
            | FailureObservation::Protocol { repair }
            | FailureObservation::Http { repair, .. } => *repair,
            FailureObservation::Network | FailureObservation::Cancelled => None,
        }
    }

    fn fail(
        &self,
        status: u16,
        error_type: &'static str,
        message: &'static str,
        failure_class: &'static str,
        upstream_status: Option<u16>,
        recovery: &'static str,
    ) -> AttemptDirective {
        AttemptDirective::Fail(ProviderFailure {
            status,
            error_type,
            message,
            provider: self.context.provider.clone(),
            route: self.context.route.clone(),
            failure_class,
            upstream_status,
            retryable: false,
            correlation_id: self.context.correlation_id.clone(),
            recovery,
        })
    }
}
```

- [ ] **Step 3: Replace the shell with the complete action surface**

Replace `main.rs` with the complete terminal shell:

```rust
mod controller;

use std::io::{self, Write};

use controller::{
    AttemptController, AttemptDirective, FailureObservation, RateKind, RepairKind, RouteContext,
    RetryPolicy,
};

fn http(
    status: u16,
    rate_kind: Option<RateKind>,
    retry_after_ms: Option<u64>,
) -> FailureObservation {
    FailureObservation::Http {
        status,
        rate_kind,
        retry_after_ms,
        repair: None,
    }
}

fn render(controller: &AttemptController) {
    print!("\x1b[2J\x1b[H");
    println!("\x1b[1mProvider Failure Contract Prototype\x1b[0m");
    println!("\x1b[2mNo network, OAuth, or persistence\x1b[0m\n");
    println!("\x1b[1mprovider\x1b[0m: {}", controller.context().provider);
    println!("\x1b[1mroute\x1b[0m: {}", controller.context().route);
    println!(
        "\x1b[1mcorrelation_id\x1b[0m: {}",
        controller.context().correlation_id
    );
    println!("\x1b[1mstate\x1b[0m: {:#?}", controller.state());
    println!(
        "\x1b[1mlast observation\x1b[0m: {:#?}",
        controller.last_observation()
    );
    println!(
        "\x1b[1mlast directive\x1b[0m: {:#?}",
        controller.last_directive()
    );
    if let Some(AttemptDirective::Fail(failure)) = controller.last_directive() {
        println!(
            "\x1b[1menvelope\x1b[0m:\n{}",
            serde_json::to_string_pretty(&failure.anthropic_json())
                .expect("serialize prototype envelope")
        );
    }
    println!("\n\x1b[1m[p]\x1b[0m begin POST   \x1b[1m[b]\x1b[0m bytes started");
    println!(
        "\x1b[1m[1]\x1b[0m capability   \x1b[1m[2]\x1b[0m 401   \x1b[1m[3]\x1b[0m 403"
    );
    println!("\x1b[1m[4]\x1b[0m rate 429     \x1b[1m[5]\x1b[0m quota 429");
    println!("\x1b[1m[6]\x1b[0m network      \x1b[1m[7]\x1b[0m upstream 500");
    println!("\x1b[1m[8]\x1b[0m known repair \x1b[1m[9]\x1b[0m protocol");
    println!(
        "\x1b[1m[c]\x1b[0m cancel       \x1b[1m[r]\x1b[0m reset  \x1b[1m[q]\x1b[0m quit"
    );
    print!("> ");
    io::stdout().flush().expect("flush prototype frame");
}

fn main() {
    let context = RouteContext {
        provider: "codex".into(),
        route: "responses_lite".into(),
        correlation_id: "prototype-0001".into(),
        policy: RetryPolicy::default(),
    };
    let mut controller = AttemptController::new(
        context,
        vec![RepairKind::OmitUnsupportedAutomaticToolChoice],
    );
    loop {
        render(&controller);
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            break;
        }
        match input.trim() {
            "p" => {
                controller.begin_post();
            }
            "b" => {
                controller.mark_response_started();
            }
            "1" => {
                controller.observe(FailureObservation::Capability { repair: None });
            }
            "2" => {
                controller.observe(http(401, None, None));
            }
            "3" => {
                controller.observe(http(403, None, None));
            }
            "4" => {
                controller.observe(http(429, Some(RateKind::RateLimit), Some(1_500)));
            }
            "5" => {
                controller.observe(http(429, Some(RateKind::Quota), None));
            }
            "6" => {
                controller.observe(FailureObservation::Network);
            }
            "7" => {
                controller.observe(http(500, None, None));
            }
            "8" => {
                controller.observe(FailureObservation::Protocol {
                    repair: Some(RepairKind::OmitUnsupportedAutomaticToolChoice),
                });
            }
            "9" => {
                controller.observe(FailureObservation::Protocol { repair: None });
            }
            "c" => {
                controller.observe(FailureObservation::Cancelled);
            }
            "r" => controller.reset(),
            "q" => break,
            _ => {}
        }
    }
}
```

- [ ] **Step 4: Document the complete action vocabulary**

Append to `README.md`:

```markdown
## Actions

- `p`: begin a POST attempt
- `b`: mark response bytes as started
- `1`: non-equivalent capability rejection
- `2`: HTTP 401 authentication rejection
- `3`: HTTP 403 authorization rejection
- `4`: rate-limited HTTP 429 with a 1500 ms Retry-After
- `5`: quota HTTP 429
- `6`: network failure
- `7`: upstream HTTP 500
- `8`: allowlisted Safe Repair observation
- `9`: unrepairable protocol failure
- `c`: cancellation
- `r`: reset in-memory state
- `q`: quit
```

- [ ] **Step 5: Compile, format, and launch the complete prototype**

Run:

```bash
cargo fmt --manifest-path desktop/gateway/Cargo.toml -- --check
cargo check --offline --manifest-path desktop/gateway/Cargo.toml --example provider_failure_prototype
cargo run --offline --manifest-path desktop/gateway/Cargo.toml --example provider_failure_prototype
```

Expected: formatting and compilation exit 0. Every action redraws the full frame; failures display a JSON object containing `type`, `error.type`, `error.message`, provider, route, failure class, retryability, correlation ID, recovery, and upstream status only when known.

- [ ] **Step 6: Commit the complete prototype**

```bash
git add desktop/gateway/examples/provider_failure_prototype
git commit -m "prototype: model provider failure directives"
```

Expected: a second prototype-only commit.

### Task 3: Drive scenarios and capture the Wayfinder decision

**Files:**
- Modify: `desktop/gateway/examples/provider_failure_prototype/README.md`
- Modify in the main worktree: `.scratch/provider-failure-contract/issues/02-choose-provider-contract-seam.md`
- Modify in the main worktree: `.scratch/provider-failure-contract/map.md`

**Interfaces:**
- Consumes: the runnable prototype and the user's live verdict.
- Produces: an accepted or rejected seam decision, a prototype branch/commit pointer, and—only on acceptance—a resolved Wayfinder ticket.

- [ ] **Step 1: Run the permanent-failure scenario with the user**

Run the prototype and enter `r`, then `1`.

Expected: zero POSTs, zero repairs, `Fail`, HTTP 400, `invalid_request_error`, `failure_class=capability`, and `retryable=false`.

- [ ] **Step 2: Run retry exhaustion with the user**

Enter `r`, `p`, `7`, `p`, `7`, `p`, `7`.

Expected directives: `RetryAfter(500)`, `RetryAfter(1000)`, then `Fail`. Final state: three POSTs, no repairs, no response bytes, and a non-retryable transient Provider Failure.

- [ ] **Step 3: Run the one-repair invariant with the user**

Enter `r`, `p`, `8`, `p`, `8`.

Expected: the first observation returns `RepairOnce(OmitUnsupportedAutomaticToolChoice)` and sets `repairs_used=1`; the second returns `Fail` and never grants another repair.

- [ ] **Step 4: Run the response-started invariant with the user**

Enter `r`, `p`, `b`, `7`, then `8`.

Expected: both the HTTP 500 and repairable protocol observation return `Fail`; the POST count remains one and no repair is consumed after bytes begin.

- [ ] **Step 5: Run rate, quota, authentication, authorization, and cancellation cases**

Run each from reset:

- `r`, `p`, `4` → `RetryAfter(1500)`.
- `r`, `p`, `5` → non-retryable quota failure.
- `r`, `p`, `2` → 401 `authentication_error` with new-request recovery.
- `r`, `p`, `3` → 403 `permission_error` with entitlement recovery.
- `r`, `p`, `c` → `Cancel`, with no new attempt.

Expected: all outputs match the design specification and contain no raw body or secret-bearing field.

- [ ] **Step 6: Ask the user for the prototype verdict**

Ask one question: “Does the driven prototype validate the pure Attempt Controller seam, including its one-repair and response-started behavior?”

If the user rejects it, leave the ticket claimed, record their exact objection under `## Prototype feedback` in the prototype README, and return to Task 2 without resolving the ticket.

If the user accepts it, append this exact structure to the README, replacing only the observation sentence with the user's stated observation:

```markdown
## Verdict

Decision: accepted

The user validated the pure Attempt Controller seam after driving the permanent-failure, retry-exhaustion, one-repair, response-started, rate/quota, authentication/authorization, and cancellation scenarios.

Observation: The controller centralizes policy and state without taking Provider Route I/O away from adapters.
```

- [ ] **Step 7: Commit and push the accepted prototype branch**

```bash
git add desktop/gateway/examples/provider_failure_prototype/README.md
git commit -m "docs: capture provider failure prototype verdict"
git push -u fork prototype/provider-failure-contract-seam
```

Expected: the fork contains a throwaway prototype branch whose tip commit contains the accepted verdict. Do not merge it into `linux-headless-oauth`.

- [ ] **Step 8: Resolve the Wayfinder ticket in the main worktree**

In `.scratch/provider-failure-contract/issues/02-choose-provider-contract-seam.md`, change `Status: claimed` to `Status: resolved` and append:

```markdown
## Answer

Choose a pure Attempt Controller as the Provider Failure Contract seam. Provider Route adapters retain protocol-specific normalization and I/O, but submit sanitized Failure Observations to the controller. The controller alone owns classification, retry and Safe Repair budgets, response-started invariants, and Provider Failure serialization.

The interactive prototype validated permanent failures, bounded retry exhaustion, one Safe Repair, the prohibition on replay after response bytes, rate versus quota behavior, authentication and authorization failures, and cancellation without using network traffic or credentials.

After the two answer paragraphs, add a `Context:` line naming branch `prototype/provider-failure-contract-seam`, followed by the literal full SHA printed by `git rev-parse HEAD`, and the path `desktop/gateway/examples/provider_failure_prototype/README.md`.
```

Do not leave a symbolic SHA token in the resolved ticket.

Append to the main map's `## Decisions so far`:

```markdown
- [Choose the Provider Failure Contract seam](issues/02-choose-provider-contract-seam.md) — A pure Attempt Controller owns failure policy and state; Provider Route adapters retain protocol-specific normalization and I/O.
```

- [ ] **Step 9: Verify and commit the resolved decision**

Run in the main worktree:

```bash
git diff --check
rg -n '^Status: resolved$|^## Answer$|Choose the Provider Failure Contract seam' \
  .scratch/provider-failure-contract/issues/02-choose-provider-contract-seam.md \
  .scratch/provider-failure-contract/map.md
git status --short --branch
```

Expected: the ticket is resolved, the answer contains the real prototype commit, the map has one new decision pointer, and no production runtime file is modified.

Commit and push:

```bash
git add .scratch/provider-failure-contract/issues/02-choose-provider-contract-seam.md \
  .scratch/provider-failure-contract/map.md
git commit -m "docs: choose provider failure contract seam"
git push fork linux-headless-oauth
```

Expected: `linux-headless-oauth` is clean and matches `fork/linux-headless-oauth`; the next remaining frontier ticket is “Prototype the Codex acceptance harness.”
