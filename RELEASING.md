# Releasing Agentport

Releases are published to crates.io from GitHub Actions when a `v<version>` tag is pushed.

## One-time repository setup

1. Sign in to crates.io, verify your email, and confirm that `agentport` is available.
2. Create a crates.io API token scoped to publishing `agentport` when available.
3. In GitHub, create an environment named `crates-io` and add the token as the `CRATES_IO_TOKEN` environment secret.
4. Protect the environment with required reviewers if desired.

## Release checklist

1. Update `version` in `Cargo.toml` and run `cargo update -w` if the lockfile needs refreshing.
2. Move entries from `Unreleased` into a dated version in `CHANGELOG.md` and update its comparison links.
3. Run:

   ```sh
   cargo fmt --all -- --check
   cargo clippy --all-targets --all-features -- -D warnings
   cargo test --all-targets --all-features
   cargo publish --dry-run --locked
   ```

4. Commit the release changes and push them to `main`.
5. Create and push the matching annotated tag:

   ```sh
   git tag -a v0.1.0 -m "Agentport 0.1.0"
   git push origin v0.1.0
   ```

The publish workflow verifies that the tag matches `Cargo.toml`, reruns release checks, and publishes exactly once. crates.io versions are permanent and cannot be overwritten.
