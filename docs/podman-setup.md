# Verify the TUI with Podman

Build Agentport from the repository root:

```sh
podman build -t agentport-test .
```

Start the interactive TUI:

```sh
podman run --rm -it agentport-test
```

Enter a public GitHub repository URL containing a standalone skill when
prompted, then verify that Agentport:

1. Discovers the expected artifacts.
2. Allows artifact and agent selection.
3. Shows the installation destinations.
4. Requests approval for active content when applicable.
5. Completes the installation without an error.

The image includes Agentport, Node.js, Git, and the Codex CLI. It also creates
empty home directories for Codex, Claude Code, Cursor, Gemini CLI, and GitHub
Copilot so the TUI can exercise every supported target without installing each
agent binary.

For repositories such as Ponytail that contain both standalone skills and a
plugin, the skills are selected by default and the plugin is optional. Select
the plugin and press `x` on the target screen only when you also want to approve
and test its active hooks.

On the target screen, press `x` if the selected artifact contains active
content that you intend to approve.

The container is disposable. `--rm` deletes it, including installed files, when
the TUI exits.

You can also provide the public repository URL when starting the container:

```sh
podman run --rm -it agentport-test https://github.com/OWNER/REPOSITORY
```

To test repo-style subagent discovery, run:

```sh
podman run --rm -it agentport-test https://github.com/binzhango/harness_util
```

Verify that Codex TOML subagents start selected when Codex is detected, and
Markdown or Harness subagents also start selected for Codex because Agentport
can generate Codex TOML from them. The `harness_util` repository should show one
selectable `harness` Harness agent package row; its `.harness/Skills/*.md`
files are internal package files, not separate subagents. In project scope, the
review screen should include project-root `AGENTS.md` and `.harness/` setup
operations in addition to any native agent registration file or generated Codex
TOML. For Markdown-style targets, the registration should be a generated file
such as `.github/agents/harness.md`, not a copied
`.github/agents/harness/` directory.

On macOS or Windows, run `podman machine start` first if Podman is not running.
