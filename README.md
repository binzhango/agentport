# Agentport

Agentport is a terminal installer for agent skills and plugins across Codex, Claude Code, and GitHub Copilot.

It accepts a public GitHub repository URL, a local directory, a ZIP archive, or a `.tar.gz` archive. Agentport scans the package, shows every discovered artifact, detects available agents, and previews every destination before it writes anything.

## Install

```sh
cargo install agentport
```

Agentport supports macOS and Linux and requires Rust 1.88 or newer. Codex plugin installation additionally requires a recent `codex` executable with the `codex plugin` commands available.

## Use

```sh
# Enter a source inside the TUI
agentport

# Open and scan a source immediately
agentport https://github.com/DietrichGebert/ponytail
agentport ./my-skill
agentport ~/Downloads/skills.zip

# Inspect and remove Agentport-managed installations
agentport list
agentport uninstall
agentport uninstall <installation-id-or-package-name>
```

Agentport is interactive and requires a TTY. GitHub sources must be public; use a local directory or archive for private packages.

The installer flow is:

1. Choose a source.
2. Select one or more discovered artifacts.
3. Select detected agents and choose global or project scope for each.
4. Explicitly opt into executable scripts and hooks if desired.
5. Review exact destinations and install.

## Native destinations

| Agent | Global skills | Project skills |
| --- | --- | --- |
| Codex | `~/.codex/skills` | `.agents/skills` |
| Claude Code | `~/.claude/skills` | `.claude/skills` |
| GitHub Copilot | `~/.copilot/skills` | `.github/skills` |

`CODEX_HOME`, `CLAUDE_CONFIG_DIR`, and `COPILOT_HOME` are honored. Codex plugin repositories are installed through the native `codex plugin` commands so bundled skills, MCP servers, and hooks remain one plugin. Root GitHub marketplaces are registered directly; local directories, archives, direct plugins, and standalone `hooks.json` packages are copied into durable Agentport-managed local marketplaces. Codex plugins are global-only. Claude and Copilot agent definitions are installed in their corresponding `agents` directories, and schema-compatible Copilot hooks can be installed in its `hooks` directory.

After installing a Codex plugin with hooks, restart Codex or start a new thread, then open `/hooks` to review and trust the hook definitions. Agentport never grants hook trust automatically.

## Supported sources and components

- Public GitHub repositories and repository-root Codex marketplaces.
- Local directories, ZIP files, and `.tar.gz`/`.tgz` archives.
- Skills, command-style skills, compatible agent definitions, Codex plugins, and supported hooks.
- Codex plugin bundles remain intact so their skills, MCP servers, assets, and lifecycle hooks are installed together.

Standalone MCP configuration is detected as active content but is not merged into agent configuration because there is no safe lossless cross-agent destination.

## Safety model

- Existing unmanaged destinations are never overwritten.
- ZIP and tar extraction rejects traversal paths, links, unsupported entry types, excessive entry counts, and oversized payloads.
- Skills containing executable source files, hooks, and MCP definitions are classified as active content and require explicit approval.
- Existing compatible Codex marketplaces/plugins are adopted without taking ownership; conflicting sources are rejected.
- Writes are staged and renamed into place. A failed target is rolled back.
- Install records and hashes live under the platform data directory, normally `~/.local/share/agentport`.
- Uninstall removes files only when their hashes still match. Locally modified files are preserved and reported.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo package --locked
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the isolated container workflow and project conventions. Release steps are documented in [RELEASING.md](RELEASING.md).

## Security

Agentport installs third-party instructions and can optionally install executable hooks or MCP definitions. Review the source and the exact preview before approving active content. Report vulnerabilities according to [SECURITY.md](SECURITY.md).

## License

MIT
