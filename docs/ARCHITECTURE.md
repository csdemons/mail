# Architecture — Atom Workspace

This is a concise onboarding map, not a replacement for the implementation and `docs/development.md`. Verify against the current source before making changes.

## Runtime
- **Desktop UI:** React + TypeScript + Vite (`frontend/`) running in a Tauri WebView.
- **Native core:** Rust (`src-tauri/`) exposes capability-gated Tauri commands. The main window and message windows have distinct capabilities in `src-tauri/capabilities/`.
- **Mail:** IMAP/SMTP and Exchange EWS/NTLM, with account-scoped session state in `commands.rs`. Never cross account boundaries for send, Sent append, search, caches, or offline queues.
- **Local persistence:** encrypted message storage, search index, offline operations, and settings; credentials use OS keyring. See `SECURITY.md` for guarantees and limitations.
- **Groupware:** CalDAV/CardDAV, SOGo, and Nextcloud (Talk, Deck, files).
- **Updates:** signed update manifests and verified downloads; see `updater.rs` and `SECURITY.md`.
- **Legacy:** `legacy/` builds a Tauri 1 variant for Astra Linux 1.7 with shared Rust modules.

## Trust boundaries
```text
Untrusted mail / attachments / imported files / remote responses
             |
             v
Rust protocol clients + parsers + HTML sanitizer
             |
             +--> account-scoped encrypted local store and offline queue
             |
             v
Tauri capability-gated IPC <--> React WebView
             |
             v
User-authorized actions against configured organizational servers
```

Do not interpret the diagram as a claim that every boundary has been penetration-tested. Security-sensitive code changes require review of URL validation, resource limits, data exposure, command permissions, and account context.

## Validation and releases
- Use `docs/development.md` for platform prerequisites, commands, test servers, and E2E tests.
- Use `.github/workflows/build.yml` for the actual CI/release contract.
- Keep product naming and existing bundle identifiers stable unless a separately reviewed migration is requested.
- A passing unit test or build is not proof of security; reproduce findings and add regression tests.

## AI integration status
An AI Assistant has been discussed as a future integration. This repository currently describes a direct-to-server desktop client; do **not** assume an AI Gateway, Keycloak flow, or DMZ service is implemented here. Introduce such components only through a separate design, threat model, explicit consent/data-flow controls, and reviewable PRs.
