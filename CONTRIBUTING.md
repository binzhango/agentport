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

## Isolated Codex testing

Build the included container and create a disposable home volume:

```sh
podman build -t agentport-test .
podman volume create agentport-test-home
podman run --rm -it \
  -v agentport-test-home:/home/tester \
  -v /absolute/path/to/package:/input/package:ro,Z \
  agentport-test
```

Remove `,Z` when SELinux labeling is unavailable. Inside the container, run `agentport /input/package`, inspect `$CODEX_HOME`, and open Codex to verify `/skills`, `/plugins`, and `/hooks`. Never mount your real home directory into this test container.

```sh
podman volume rm agentport-test-home
```

## Pull requests

- Keep changes focused and document user-visible behavior in `CHANGELOG.md`.
- Do not commit credentials, local state, generated package archives, or build output.
- Confirm CI passes on Linux and macOS.
