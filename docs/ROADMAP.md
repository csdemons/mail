# Development roadmap (review proposal)

This is a **proposal**, not a commitment or evidence that a vulnerability exists. The current product is **Atom Workspace** (formerly Atom Mail / RRCKI Mail). Existing functionality is described in [../README.md](../README.md) and [development.md](development.md).

## Phase 0 — AI-assisted development onboarding
- [x] Add repository-level `AGENTS.md`.
- [x] Add architecture and security-review orientation.
- [ ] Obtain human review of this documentation PR.
- [ ] Configure required PR reviews, status checks, and protected `main` in GitHub settings.
- [ ] Confirm test coverage and supported-platform matrix.

## Phase 1 — verify security hypotheses on current main
- [ ] Review HTML message iframe/sandbox/CSP/IPC interactions on Windows WebView2 and Linux WebKitGTK; reproduce before changing rendering.
- [ ] Review EWS Autodiscover trust policy, NTLM MIC/channel binding, and TLS diagnostic boundaries against current code and test servers.
- [ ] Review policy fail-closed behavior for malformed mandatory policies and safe defaults.
- [ ] Recheck offline UIDVALIDITY, Message-ID matching, and ambiguous send handling; verify existing fixes before opening issues.
- [ ] Validate update archive extraction and signature/hash verification on each supported platform.
- [ ] Validate account isolation during send, switching, logout, search, and Nextcloud operations.

Each item becomes a separate issue/PR **only after current-code confirmation**, with reproducible tests and explicit scope.

## Phase 2 — stability and release confidence
- [ ] Extend isolated protocol E2E and fuzz/property tests for untrusted input.
- [ ] Document supported Windows/Linux/Astra/macOS versions and legacy security trade-offs.
- [ ] Confirm backup/export, local secrets, and temporary-file permissions across platforms.
- [ ] Keep signed update and release workflows intact; verify CI and human approval before merge.

## Phase 3 — optional AI Assistant integration
- [ ] Write separate architecture, data-flow map, privacy/authorization requirements, and threat model.
- [ ] Define explicit opt-in for transferring mail/attachments to corporate AI services.
- [ ] Design account-scoped authorization, Keycloak integration if approved, model-gateway API, and audit/redaction.
- [ ] Implement behind a disabled-by-default feature flag with synthetic integration tests.

No direct push to `main`, production deployment, version bump, or release publication is authorized by this roadmap.
