# Changelog

Every release takes its notes from its section here (`## <version>`): the
release workflow copies it to the GitHub release, which the website's
downloads page and the app's "Check for Updates" show. Write the next
version's notes under `## Unreleased`, then rename it when you tag.

## Unreleased

## 0.1.5

- Quick Look (Space) on ZIP, RAR, TAR and GZ archives lists what is inside.
- Windows: forks of folders on a ReFS volume (Dev Drive) are kept on that
  volume in `.file-force-forks`, so Windows 11 24H2+ makes them as block
  clones: instant and taking no extra space. NTFS still makes a full copy.

## 0.1.4

- RAR archives: preview, browse and extract on every system (no `unrar`
  needed).
- PDF previews on Windows and Linux (Linux needs poppler-utils).
- Windows: the context menu opens instantly, programs no longer flash a
  console window, "Choose app" in Default apps opens a file dialog, and
  Terminal offers Windows Terminal, PowerShell and Command Prompt.
- Terminal lists only the apps installed on this computer.
- An empty folder says "This folder is empty" instead of "No matching files".
- Accessibility settings say "System: on/off" instead of "macOS".

## 0.1.3

- "Files X is available" notification with Install (signed download),
  Later (asks again at the next check) and Skip this release.
- Settings → General → Check now: look for an update any time, with
  Download & Install and the release notes right there.

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
