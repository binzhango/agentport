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

The image includes Agentport, Node.js, Git, and the Codex CLI. For repositories
such as Ponytail that contain both standalone skills and a plugin, the skills
are selected by default and the plugin is optional. Select the plugin and press
`x` on the target screen only when you also want to approve and test its active
hooks.

On the target screen, press `x` if the selected artifact contains active
content that you intend to approve.

The container is disposable. `--rm` deletes it, including installed files, when
the TUI exits.

You can also provide the public repository URL when starting the container:

```sh
podman run --rm -it agentport-test https://github.com/OWNER/REPOSITORY
```

On macOS or Windows, run `podman machine start` first if Podman is not running.
