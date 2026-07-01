# Changelog

All notable changes to Agentport are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and releases follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.3] - 2026-07-01

### Added

- Selectable repo subagent installs, including Markdown subagents for Claude
  Code, Cursor, Gemini CLI, and GitHub Copilot, plus Codex TOML subagents.
- Codex TOML generation for Markdown subagents and Harness agent packages.
- Project-scope Harness installs now set up root `AGENTS.md` and `.harness/`
  files in addition to native agent registration.
- Cursor and Gemini CLI detection and native global/project destinations.

## [0.1.2] - 2026-06-24

### Added

- A `-g` / `--global` flag to default the installer to global agent-specific skill
  directories.

### Changed

- Project-scope skill installs are now the default and use the shared
  `.agents/skills` Agent Skills convention at the Git repository root.
- Project-scope installs outside a Git repository now require explicit confirmation.
- crates.io publishing now runs automatically from GitHub Actions when a new
  crate version reaches `main`.
- Successful new crate publishes now create a GitHub Release populated from the
  matching `CHANGELOG.md` version section.

## [0.1.1] - 2026-06-20

### Added

- An all-in-one Podman test image with Agentport, Codex CLI, Node.js, and Git.
- A focused Podman TUI testing guide.
- Editable source input with paste, cursor movement, deletion, and horizontal scrolling.

### Changed

- Codex plugin repositories expose bundled skills as independently selectable artifacts.
- Standalone skills are selected by default while optional plugins remain opt-in.
- TUI panels, focused rows, checked choices, and the source cursor have clearer styling.
- Crates.io and docs.rs documentation now includes a richer overview, compatibility matrix,
  quick start, safety model, and installation-management guidance.

### Fixed

- Container builds now include the README required by the crate's embedded documentation.
- Empty installation plans are blocked before execution with actionable skipped reasons.
- Plugin choices are hidden when the Codex CLI is unavailable but standalone skills can
  still be installed.

## [0.1.0] - 2026-06-19

### Added

- Interactive installation of skills for Codex, Claude Code, and GitHub Copilot.
- Codex marketplace, plugin, bundled-hook, and standalone-hook installation.
- Public GitHub, local directory, ZIP, and tarball sources.
- Transactional file writes, managed installation records, and hash-safe uninstall.
- Active-content review for scripts, hooks, and MCP definitions.

[Unreleased]: https://github.com/binzhango/agentport/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/binzhango/agentport/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/binzhango/agentport/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/binzhango/agentport/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/binzhango/agentport/releases/tag/v0.1.0
