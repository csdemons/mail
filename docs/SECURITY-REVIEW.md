# Security review workflow for contributors

The authoritative project security description and private reporting guidance are in [../SECURITY.md](../SECURITY.md). This file describes **how to review changes**, not a new vulnerability disclosure channel.

## Threat model checklist
- Malicious email HTML, MIME, attachments, ICS, PST, mbox, and vCard content.
- Untrusted Autodiscover, EWS, IMAP, SMTP, DAV, and Nextcloud responses and URLs.
- Tauri IPC from the main window, message windows, and external document windows.
- Multiple account sessions, switching, logout, caching, and Sent-folder handling.
- Network interruptions and ambiguous SMTP/EWS outcomes.
- OS credential store, encrypted local files, exports, logs, and temporary attachments.
- Update metadata, downloaded archives, release signing, and CI supply chain.

## Review gates
1. Reproduce or trace a reported issue on the **current commit**. Document affected source lines, platform, preconditions, and impact.
2. Classify evidence as confirmed, potential, or not reproduced. Never label a code pattern a proven exploit without a credible execution path.
3. Implement the smallest safe fix; avoid compatibility regressions or weakening TLS/IPC protections.
4. Add negative tests (malformed or malicious inputs) and positive tests (normal operation).
5. Run relevant Rust/frontend checks, protocol simulators, and E2E tests. Record exactly what ran and what did not.
6. Require human review for security-sensitive changes; keep operational details and exploit payloads out of public issues when disclosure could cause harm.

## High-priority regression scenarios
- HTML email: script execution, CSP, iframe origin/sandbox behavior, link opening, remote images, Tauri command access.
- Exchange: Autodiscover allowlists, TLS failure, NTLM MIC/channel binding, connection affinity, EWS redirects.
- Offline: UIDVALIDITY change, Message-ID mismatch, repeated UID, queued deletion/move, uncertain send.
- Cross-account: switch/logout during SMTP send, EWS save, search, attachments, and Nextcloud requests.
- Updates: signature/hash failure, path traversal, symlinks, excessive extraction, rollback and atomic install.
- Policy: missing vs malformed mandatory policy, unsafe defaults, file permissions and enforcement.
- Import: corrupted PST/MIME, decompression limits, large nested attachments, unsafe filenames.

## CI and release controls
The existing `.github/workflows/build.yml` runs dependency audits, frontend checks, Rust lint/tests, platform builds, and release steps. Verify required status checks and branch protection in GitHub settings; do not assume they are enabled. Do not alter release signing, versions, or production credentials as part of a documentation/security triage PR.
