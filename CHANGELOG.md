# Changelog

Every release takes its notes from its section here (`## <version>`): the
release workflow copies it to the GitHub release, which the website's
downloads page and the app's "Check for Updates" show. Write the next
version's notes under `## Unreleased`, then rename it when you tag.

## Unreleased

- "Download & Install" in Check for Updates: every release is signed in CI
  and Files checks the signature before opening the installer.

## 0.1.2

- After an update, a notification says what version you now have, with a
  button that opens its release notes.
- Notifications can carry an action button (for extensions too).
- Windows: no console window opens next to Files.
- The disk card at the bottom of the sidebar is a carousel of every disk
  (C:, D:, USB drives, network shares): arrows, dots, and two-finger swipes.
  Clicking a disk opens it.

## 0.1.1

- The sidebar shows your account name and "Local workspace" instead of the
  demo labels.
- The title bar reads "file FORCE".

## 0.1.0

The first release of Files.

- Fast file explorer: list, cards and dual-pane views, tabs, Quick Look
  (folders too), global search with filters (`kind:`, `size:`, `modified:`,
  `tag:`, `mode:`).
- Extensions: Git, SSH (with Tailscale sign-in and Keychain passwords), AI
  agents (Claude Code, Codex, OpenCode), archives, copy-on-write forks,
  terminal.
- Accessibility: reduce motion, high contrast, text contrast, text size,
  VoiceOver labels.
- Developer console, notifications, permissions per extension.
