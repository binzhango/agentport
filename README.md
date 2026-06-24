# Agentport

[![Crates.io](https://img.shields.io/crates/v/agentport.svg)](https://crates.io/crates/agentport)
[![Documentation](https://docs.rs/agentport/badge.svg)](https://docs.rs/agentport)
[![MSRV](https://img.shields.io/badge/rustc-1.88%2B-blue.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/crates/l/agentport.svg)](https://github.com/binzhango/agentport/blob/main/LICENSE)

**Install agent skills and plugins without guessing where their files will go.**

Agentport is an interactive terminal installer for Codex, Claude Code, and
GitHub Copilot. Give it a public GitHub repository, local directory, ZIP file,
or tar archive; it discovers the package contents, detects compatible agents,
and previews every destination before writing anything.

## Why Agentport?

Agent packages use different layouts and every coding agent expects a different
destination. Agentport provides one reviewable workflow:

- Discover skills, command-style skills, agent definitions, hooks, and Codex
  plugins from a package.
- Select individual artifacts instead of installing everything blindly.
- Install into the current repository by default using the open `.agents/skills`
  convention, or globally with `-g` / `--global`.
- Keep Codex plugin bundles intact and install them through the Codex CLI.
- Require explicit approval before installing hooks, scripts, or MCP content.
- Record hashes so uninstall removes unchanged files and preserves local edits.

## Install

```sh
cargo install agentport
```

Agentport supports macOS and Linux and requires Rust 1.88 or newer. Native
Codex plugin installation also requires a recent `codex` executable that
provides the `codex plugin` commands.

## Quick start

Open the source field in the TUI:

```sh
agentport
```

Or scan a source immediately:

```sh
agentport https://github.com/DietrichGebert/ponytail
agentport ./my-skill
agentport ~/Downloads/skills.zip
agentport ~/Downloads/skills.tar.gz
```

By default, selected skills install into the current Git repository's
`.agents/skills` directory so any Agent Skills-compatible client can discover
them. Use `-g` or `--global` to default the installer to global agent-specific
skill directories:

```sh
agentport -g ./my-skill
agentport --global https://github.com/DietrichGebert/ponytail
```

The installer then walks through five reviewable steps:

1. Select discovered artifacts.
2. Choose detected target agents.
3. Choose global or project scope.
4. Approve active content only when you intend to install it.
5. Review exact destinations and install.

Agentport is interactive and requires a TTY. GitHub URLs must point to public
repositories; clone private repositories first and install from the local
checkout. If the current directory is not inside a Git repository, project-scope
installs require explicit confirmation before writing to the current directory.

## What gets installed?

| Component | Codex | Claude Code | GitHub Copilot |
| --- | --- | --- | --- |
| Skills and command-style skills | Global or project | Global or project | Global or project |
| Agent definitions | — | Global or project | Global or project |
| Codex plugins | Native CLI, global | — | — |
| Standalone hooks | Managed local plugin | Detected, not merged | Compatible schemas |
| Standalone MCP configuration | Detected, not merged | Detected, not merged | Detected, not merged |

Agentport does not merge standalone MCP configuration into existing agent
configuration because there is no safe, lossless cross-agent destination.
MCP servers bundled in a Codex plugin remain part of that native plugin.

### Skill destinations

| Agent | Global | Project |
| --- | --- | --- |
| Codex | `~/.codex/skills` | `.agents/skills` |
| Claude Code | `~/.claude/skills` | `.agents/skills` |
| GitHub Copilot | `~/.copilot/skills` | `.agents/skills` |

Project skill installs are rooted at the current Git repository root. The
`.agents/skills` path follows the open Agent Skills convention for cross-client
reuse. `CODEX_HOME`, `CLAUDE_CONFIG_DIR`, and `COPILOT_HOME` are honored for
global installs.

### Codex plugin behavior

Repository-root Codex marketplaces are registered directly. Local directories,
archives, direct plugins, and standalone `hooks.json` packages are copied into
durable Agentport-managed local marketplaces before native installation.
Plugins are global-only so their skills, hooks, MCP servers, and assets remain
one bundle.

After installing a plugin with hooks, restart Codex or start a new thread, then
open `/hooks` to review and trust the definitions. Agentport never grants hook
trust automatically.

## Manage installations

Inspect everything Agentport owns:

```sh
agentport list
```

Choose an installation interactively or name one directly:

```sh
agentport uninstall
agentport uninstall <installation-id-or-package-name>
```

Install records normally live in `~/.local/share/agentport`. During uninstall,
Agentport removes only files whose hashes still match the installed versions.
Modified files are reported and preserved.

## Safety model

- Existing unmanaged destinations are never overwritten.
- Archive extraction rejects traversal paths, links, unsupported entry types,
  excessive entry counts, and oversized payloads.
- Executable skill content, hooks, and MCP definitions require explicit
  approval.
- Conflicting Codex marketplace sources are rejected.
- File writes are staged and renamed into place; failed targets are rolled back.
- Uninstall preserves files changed after installation.

Agentport installs third-party instructions and can install executable content
when you approve it. Review the source and final destination preview before
continuing. Report vulnerabilities according to the
[security policy](https://github.com/binzhango/agentport/blob/main/SECURITY.md).

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo package --locked
```

- [Contributing guide](https://github.com/binzhango/agentport/blob/main/CONTRIBUTING.md)
- [Podman TUI testing](https://github.com/binzhango/agentport/blob/main/docs/podman-setup.md)
- [Release process](https://github.com/binzhango/agentport/blob/main/RELEASING.md)
- [Changelog](https://github.com/binzhango/agentport/blob/main/CHANGELOG.md)

## License

Agentport is available under the [MIT License](https://github.com/binzhango/agentport/blob/main/LICENSE).
