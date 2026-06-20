# Contributing

## Development setup

Agentport requires Rust 1.88 or newer.

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo package --locked
```

Keep installation behavior transactional, never overwrite unmanaged paths, and add regression tests for scanner, planning, rollback, and uninstall changes.

## Isolated TUI testing

Build the image and run Agentport interactively:

```sh
podman build -t agentport-test .
podman run --rm -it agentport-test
```

Enter a public GitHub repository URL in the TUI, then verify source scanning,
selection, destination preview, approval prompts, and installation completion.
The image includes the Codex CLI so native plugin installation can also be
tested. The container and installed files are discarded when the TUI exits.

See [Verify the TUI with Podman](docs/podman-setup.md) for the complete workflow.

## Pull requests

- Keep changes focused and document user-visible behavior in `CHANGELOG.md`.
- Do not commit credentials, local state, generated package archives, or build output.
- Confirm CI passes on Linux and macOS.
