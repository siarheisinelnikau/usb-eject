# Project instructions

## Authorized task and scope
The user explicitly requested a separate Codex Desktop implementation chat for this project. Work only inside this repository. Other new projects are being developed in parallel under E:/work/github: do not edit sibling projects, parent configuration, or shared build output. Read TASK.md before implementation. Use explicit repository working directories for every shell/Git command.

## Unattended code-only session
The user is AFK and unavailable. Do not ask questions, request approvals/elevation, or start login/token/browser flows. Make reasonable implementation choices and continue all unblocked code work. If a tool action would need approval, skip that action, record the exact limitation, and use a safe code-only alternative.
Do not use computer-use/CUA, mouse/keyboard automation, screenshots, browser UI or interactive application control. Do not launch the application, open visible windows, play audio, record screens/microphones, install services/drivers/extensions, change Windows settings, or operate real user files/devices. No GUI smoke tests in this run. Read-only API/web documentation research is fine. No access to credentials or secrets from sibling repositories.
Safe noninteractive formatting, compilation and focused tests on isolated fixtures are allowed with the existing toolchain. Never request escalation if downloads or tool availability are blocked. Do not run broad benchmarks or stress tests. With many chats active, keep Cargo builds to two jobs and use a repository-local target directory; no shared mutable target directories. If dependencies are unavailable, leave precise verification commands and an honest unverified status.

## Implementation expectations
Default to Rust, Windows-first, native desktop UI and one standalone EXE where practical. English is the complete base UI language; identifiers, comments, documentation, commit messages and release notes must be English. Talk to the user in Russian. Use original UI/assets and an open-source license compatible with dependencies (MIT is the preferred baseline).
Implement actual useful code and a coherent working vertical slice, not only a plan, stubbed dashboard or screenshots of a mock UI. Keep effects behind explicit user actions in the future application. Never label simulations or unavailable platform integrations as working. Put expensive I/O/computation on bounded background workers; idle UI/tray operation must not busy-poll.
Apply C:/Users/user/.codex/skills/rust-performance/SKILL.md and relevant references for UI/background lifecycle decisions. The user's prohibition on running/controlling the desktop overrides live-observation suggestions. Document deferred manual validation.

## Git and GitHub
Make frequent small local commits at meaningful milestones, with English messages, including a final commit of finished code and docs. Check status/diffs and stage only this repository. Never amend/delete someone else's history. The repo was initialized locally before the chat started.
This project is intended for GitHub, but this phase is local implementation/preparation. Do not publish a repository, push, open a PR, or create/alter releases in this run. If remote work is later authorized, use the connected GitHub plugin exclusively, never git remote commands, gh, authenticated shell HTTP, credential helpers or authentication setup.
Apply C:/Users/user/.codex/skills/github-release-conventions/SKILL.md. Include installed version, manual Check for updates, nonblocking startup checks enabled by default with a persistent opt-out, and user-initiated in-app Download and install with progress/error handling, preservation of data and restart where needed. Do not execute the updater during development. Treat a not-yet-published repository/release as unavailable rather than inventing a release.

## Deliverables and verification
Provide Cargo sources/lockfile when resolvable, build/package scripts, appropriate noninteractive checks/CI, English README, architecture/usage notes, initial capability release-note draft and a clear list of remaining manual/hardware checks. Use truthful supported-platform/capability labels.
README screenshots must be real. The user's current no-GUI instruction means screenshots are deferred: explicitly state that real application/settings screenshots and interactive verification are pending. Do not fabricate screenshots, add broken image links or block implementation waiting for screenshots.
Conclude with what was implemented, relevant commit IDs, checks actually completed, and specific remaining limitations. Do not ask the AFK user for approval or stop early just to offer to continue. Do not create more chats/subagents for this task unless separately authorized.

