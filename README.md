# Files

A fast, keyboard-friendly file explorer for macOS and Linux, built in Rust
with [GPUI](https://www.gpui.rs). It comes with the tools developers reach for
every day, as extensions you can turn on and off.

## Highlights

- **Fast and native**: GPU-rendered, instant search with filters
  (`kind:image size:>10mb modified:<7d tag:work mode:644`), Quick Look,
  dual-pane view, tabs.
- **Accessible**: reduce motion, high contrast, text contrast outlines,
  adjustable text size, VoiceOver labels.
- **Extensions**: Git (commit, log, diff, branches, stash), SSH (your own
  `~/.ssh/config`, keys, Tailscale sign-in), AI agents (Claude Code, Codex,
  OpenCode sessions, chats and usage), archives, copy-on-write forks.
- **Safe by design**: every extension declares permissions you can revoke;
  passwords live in the Keychain; a developer console shows every call an
  extension makes.

## Build

```sh
cargo run                    # debug build
cargo xtask bundle           # release Files.app + .dmg (see Bundle.toml)
cargo xtask doctor           # what is left before publishing
```

## Extensions

Extensions are written against [`files-sdk`](crates/sdk). Built-in ones live
in `crates/ext-*`; paid ones ship as signed plugin packages
(`cargo xtask package-extension <crate>`), see `crates/ext-hello-pro`.

## License

[Functional Source License 1.1, MIT Future License](LICENSE.md): use it, read
it, change it, except to build a competing product. Each version becomes MIT
two years after its release.
