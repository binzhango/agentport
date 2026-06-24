# Releasing Agentport

Releases are published to crates.io from GitHub Actions when a new crate
version reaches `main`. The same publish workflow also supports `v<version>`
tags for explicit release events.

## One-time repository setup

1. Sign in to crates.io, verify your email, and confirm that `agentport` is available.
2. Create a crates.io API token scoped to publishing `agentport` when available.
3. In GitHub, add the token as the repository secret `CARGO_REGISTRY_TOKEN`. The publish job maps this secret to Cargo's standard environment variable of the same name.
4. The workflow uses a `crates-io` deployment environment; configure required reviewers for it if desired.

## Release checklist

1. Update `version` in `Cargo.toml` and run `cargo update -w` if the lockfile needs refreshing.
2. Move entries from `Unreleased` into a dated version in `CHANGELOG.md` and update its comparison links. The publish workflow uses this version section as the GitHub Release notes.
3. Run:

   ```sh
   cargo fmt --all -- --check
   cargo clippy --all-targets --all-features -- -D warnings
   cargo test --all-targets --all-features
   cargo publish --dry-run --locked
   ```

4. Commit the release changes and push them to `main`. The publish workflow
   will publish the crate if that exact version is not already on crates.io.
5. Optionally create and push the matching annotated tag:

   ```sh
   git tag -a v0.1.0 -m "Agentport 0.1.0"
   git push origin v0.1.0
   ```

The publish workflow verifies that any pushed tag matches `Cargo.toml`, reruns
release checks, publishes new crate versions, and creates a GitHub Release from
the matching `CHANGELOG.md` section. It skips publishing when the crate version
already exists. crates.io versions are permanent and cannot be overwritten.
