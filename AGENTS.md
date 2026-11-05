# AI development instructions — Atom Workspace

These instructions apply to the entire repository. Read `README.md`, `docs/development.md`, and `SECURITY.md` before modifying code. Follow more-specific instructions if added in subdirectories.

## Project identity and scope
- Product name: **Atom Workspace** (formerly Atom Mail / RRCKI Mail). Preserve existing package, binary, bundle, and API identifiers unless an explicit migration is approved.
- This is a **desktop application**, not a hosted mail gateway: it connects directly to configured IMAP/SMTP, Exchange EWS, CalDAV/CardDAV, SOGo, and Nextcloud services.
- Primary stack: Tauri 2 + Rust (`src-tauri/`), React + TypeScript + Vite (`frontend/`).
- Legacy Astra Linux SE 1.7 uses Tauri 1 in `legacy/` and shares Rust modules; other targets include Windows, Linux/Astra 1.8, and macOS.
- Keep existing features, localization, compatibility, and user data intact. Avoid unrelated refactoring and speculative architecture changes.

## Repository map
- `frontend/src/`: UI, rendering, IPC callers.
- `src-tauri/src/commands.rs`: Tauri commands, account-scoped `AccountCtx`, send/offline coordination.
- `src-tauri/src/imap.rs`, `smtp.rs`, `ews.rs`, `ntlm.rs`: mail protocols and Exchange authentication.
- `src-tauri/src/html.rs`: untrusted message HTML sanitization.
- `src-tauri/src/store.rs`, `search.rs`, `offline.rs`: encrypted local data, search, offline operations.
- `src-tauri/src/policy.rs`, `updater.rs`: administrator policy and signed updates.
- `src-tauri/src/nextcloud.rs`, `dav.rs`, `sogo.rs`: connected services.
- `src-tauri/capabilities/`, `src-tauri/permissions/`, `src-tauri/build.rs`: Tauri command authorization.
- `tests/imap-server/`, `tests/servers/`, `tests/e2e/`: test servers and end-to-end tests.
- `.github/workflows/build.yml`: checks, platform bundles, and release workflow.
- `vendor/outlook-pst/`: vendored patched dependency; review carefully before changing.

## Non-negotiable security boundaries
1. **Account isolation:** every mail action, outgoing message, Sent-folder append, cached item, search, and queued operation must use the intended account's context. Never fall back to another account's credentials or session.
2. **Untrusted content:** treat HTML mail, MIME, attachments, imported PST/mbox/ICS/vCard, filenames, URLs, and server responses as attacker-controlled. Preserve HTML sanitization, CSP, safe URL handling, limits, and Tauri IPC isolation. No script execution from email content.
3. **Credentials and tokens:** use platform keyring and existing secure storage; never log or commit secrets, raw authorization headers, message bodies, production configuration, or private keys. Do not add TLS-verification bypasses or insecure HTTP fallbacks.
4. **Network trust:** preserve certificate validation, HTTPS-only rules where required, redirect protections, and account-bound authentication. Review Exchange Autodiscover/NTLM/CBT/MIC changes against protocol specifications and tests.
5. **Offline correctness:** preserve UIDVALIDITY and Message-ID validation, conflict handling, and the ambiguous-send state. Do not automatically resend when SMTP/EWS delivery may have succeeded.
6. **Update integrity:** never weaken signed manifest, package hash, code-signing, archive-path, or rollback checks. Do not publish releases without explicit approval.
7. **Administrative policy:** distinguish missing, malformed, and mandatory policy; avoid silently weakening enforced settings. Do not broaden Tauri capabilities without explaining and testing the privilege impact.
8. **Data privacy:** keep message and attachment content local or on explicitly configured organizational services. Any future AI Assistant integration requires explicit user action, scoped authorization, documented data flow, and a separate security review; never silently upload mail to external models.

## Implementation workflow
1. Inspect current `main`, relevant code, existing tests, and recent changelog before editing. **Do not assume previous audit findings still apply**; verify each finding against the current commit.
2. Work in a short-lived feature branch, one narrowly scoped change per PR. Never push directly to `main`, merge a PR, publish a release, or change production systems without explicit approval.
3. Explain the problem, affected threat boundary, proposed fix, and compatibility impact. Prefer small, reviewable diffs with regression tests.
4. When introducing a Tauri command, update the command registration in `commands.rs`, `src-tauri/build.rs`, applicable capability JSON, and generated permissions; review whether `message-window.json` actually needs the permission. Follow `docs/development.md` for the legacy integration.
5. When changing shared Rust code, assess the Tauri 1 legacy build, platform-specific paths, and existing offline data formats.
6. Never embed real credentials, personal correspondence, live server addresses, or customer data in fixtures, logs, screenshots, issues, or PR descriptions. Use synthetic test accounts and isolated servers.
7. For security findings, report **confirmed vulnerability**, **configuration weakness**, or **hypothesis requiring reproduction** separately; do not overstate severity.

## Local validation (run what the environment supports)
```bash
npm ci
npm --prefix frontend run typecheck
npm --prefix frontend run build
npm --prefix frontend audit --omit=dev --audit-level=high
cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo audit
cd ../legacy && cargo audit
```

Linux builds require system packages listed in `docs/development.md`. Full E2E tests require the isolated test servers and display/keyring/WebDriver setup described there. Do not run test-server setup scripts on production hosts. If a check cannot run, document the reason; never claim it passed. CI is the final cross-platform verification, not a substitute for code review.

## PR completion criteria
- Concise summary and exact affected components.
- Threat model or failure scenario for security-sensitive changes.
- Tests added or updated, with commands and actual results.
- Compatibility notes: Windows, Linux/Astra 1.7 and 1.8, macOS, EWS vs IMAP/SMTP, and existing user data as applicable.
- No secrets, generated binaries, unrelated edits, or unapproved dependency upgrades.
- All required CI checks pass before a human-approved merge.
