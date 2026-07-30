# Linux Launcher Persistence Hardening Design

Date: 2026-07-30
Status: proposed
Scope: machine-local installed launcher only

## Context

The merged Ticket 07 Gateway is installed and healthy on `127.0.0.1:11535`. Its running process uses the required upstream proxy on loopback port `2999`, and both uppercase and lowercase `NO_PROXY` forms bypass `127.0.0.1` and `localhost`. Claude Science remains on port `9002` and points to Gateway `11535`.

The installed `/home/bio-13/.local/bin/csswitch-codex` launcher still inherits two generic source defaults that do not describe this machine:

- `CSSWITCH_GATEWAY_PORT` falls back to `11434`;
- Gateway proxy variables are inherited from whichever shell invokes the launcher.

Therefore a later plain `csswitch-codex start`, `status`, or `stop` can target the wrong port, and a start from a shell without the current proxy environment can omit proxy `2999`. The currently running services are correct; this is a restart-persistence defect in the machine-local launcher.

## Goals

- Make plain installed-launcher commands target Gateway port `11535` on this machine.
- Make every Gateway start use loopback proxy port `2999` in `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, and lowercase equivalents.
- Make every Gateway start set both `NO_PROXY` forms to `127.0.0.1,localhost`.
- Preserve explicit environment overrides for controlled future changes.
- Leave Claude Science launch semantics, data, PID, port `9002`, and Gateway base URL unchanged.
- Back up the installed launcher and provide a byte-verifiable rollback.
- Apply the launcher correction without restarting the healthy Gateway or Science processes.

## Non-goals

- Do not change the repository's generic `scripts/csswitch-codex` default of `11434`.
- Do not change provider selection, Codex authentication, credentials, profiles, model selection, fallback behavior, tunnels, or firewall settings.
- Do not add a second launcher command that operators must remember.
- Do not induce a live API-key provider failure or claim macOS verification.

## Approaches considered

### 1. Harden the installed launcher in place — selected

Back up the installed launcher, patch only its machine-local defaults, verify a candidate, and atomically replace the installed file. Existing commands keep their names and the repository remains portable.

Trade-off: reinstalling the generic repository script would overwrite the machine-local defaults, so the deployment record must state that this launcher is locally managed.

### 2. Add a second wrapper command

Keep `csswitch-codex` generic and introduce a separate wrapper that exports port and proxy values.

Trade-off: the operator or an SSH automation path can accidentally call the old command, so the original persistence defect remains reachable.

### 3. Depend on shell startup exports

Put the port and proxy variables in interactive shell configuration.

Trade-off: non-interactive SSH commands, services, and clean shells do not reliably load the same files. This reproduces the failure mode the hardening is intended to remove.

## Installed launcher contract

The installed launcher will use these machine defaults:

```text
gateway port: 11535
Gateway proxy: http://127.0.0.1:2999
Gateway no_proxy: 127.0.0.1,localhost
Science port: 9002 (unchanged)
sandbox port: 9003 (unchanged)
```

The launcher will retain these overrides:

```text
CSSWITCH_GATEWAY_PORT
CSSWITCH_GATEWAY_PROXY
CSSWITCH_GATEWAY_NO_PROXY
```

`start_gateway` will pass the selected proxy value directly to `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `http_proxy`, `https_proxy`, and `all_proxy` for the Gateway child only. It will pass the selected local-bypass value to `NO_PROXY` and `no_proxy`. The launcher process itself will not export or persist those values, so the existing Science child environment remains unchanged: Science continues to use Gateway as its Anthropic endpoint and HTTPS proxy.

The Gateway remains Codex-only. Authentication, catalog, or inference failure continues to fail closed without provider or model fallback.

## Backup, installation, and rollback

Before editing, copy the current launcher byte-for-byte to:

```text
/home/bio-13/.local/bin/csswitch-codex.backup-pre-persistence-20260730
```

Record SHA-256, ownership, mode, and size for the installed launcher and backup. Build the candidate as a regular file in the writable workspace, run all deterministic checks against that candidate, copy it beside the installed launcher, set mode `0755`, verify its staged hash, and atomically rename it over `/home/bio-13/.local/bin/csswitch-codex`.

If candidate checks, atomic installation, plain status, or post-install health checks fail, restore the verified backup atomically. Because the running Gateway and Science executables are not restarted or signalled, launcher rollback is independent of runtime rollback.

## Regression-first verification

The RED check runs against an unmodified candidate copied from the installed launcher with all port/proxy override variables removed. It must demonstrate:

- the default port is `11434` rather than required `11535`;
- no launcher-owned Gateway proxy default exists.

The GREEN checks run against the patched candidate and require:

1. A sanitized source-level contract reports default port `11535`, proxy `http://127.0.0.1:2999`, and local bypass `127.0.0.1,localhost`.
2. A real `start_gateway` invocation with a fake Gateway executable captures the exact six proxy variables, both no-proxy variables, `--provider codex`, and `--port 11535`.
3. Explicit port, proxy, and no-proxy overrides replace the defaults in the fake child environment and arguments.
4. `bash -n` succeeds.
5. The existing headless controller test passes against the repository's unchanged generic launcher.
6. The installed candidate contains no Kimi fallback, broad process-kill command, credential literal, or bearer URL.

After atomic installation, plain `/home/bio-13/.local/bin/csswitch-codex status` must recognize the already-running Gateway at `11535` and report authentication, health, catalog, and Science as ready. The running Gateway executable hash, proxy environment, PID, and Science PID must remain unchanged across the launcher-only installation.

## Evidence and limitations

Record the backup and installed launcher hashes, RED/GREEN observations, exact child environment assertions, plain status result, and unchanged runtime identities in Ticket 07's installed Linux verification record. Do not record authentication tokens, Science bearer URLs, account identifiers, raw provider responses, or proxy payloads.

This hardening proves restart configuration and launcher behavior on this Linux machine. It does not perform a restart, test a live API-key provider failure, or provide macOS evidence.
