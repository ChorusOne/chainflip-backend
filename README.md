[![Gitpod ready-to-code](https://img.shields.io/badge/Gitpod-ready--to--code-blue?logo=gitpod)](https://gitpod.io/#https://github.com/chainflip-io/chainflip-backend)

# Chainflip

This repo contains everything you need to run a validator node on the Chainflip network.

## Getting started

The project is organised using rust workspaces. See the `Cargo.toml` in this directory for a list of contained
workspaces. Each workspace should have its own `README` with instructions on how to get started. If not, please raise an
issue!

## Contributing

### Code style

The best way to ensure that your code is easy to merge, is to copy the project's pre-commit hook into your local `.git/` directory. You can do this with:

```bash
cp .git-hooks/pre-commit .git/hooks/
chmod +x .git/hooks/pre-commit
```

Since much of the project is reliant on parity substrate, please take inspiration from
parity's [Substrate code style](https://github.com/paritytech/substrate/blob/master/docs/STYLE_GUIDE.md) where possible.
Please see this as a guideline rather than rigidly enforced rules. We will define and enforce formatting rules
with `rustfmt` in due course. It should be straightforward to integrate this with your favourite editor for
auto-formatting.

> TODO: research and set up .rustfmt and/or .editorconfig settings, and enforce with CI. We may need to have separate settings files for each sub-project since substrate code has some funky settings by default and we may want to stick to a more common setup for our non-substrate components.

### Branching and merging

Before making any changes:

- create a new branch always.
- give it a descriptive name: `feature/my-awesome-feature`

When your changes are ready, or you just want some feedback:

- open a PR.
- once the PR is open, avoid force-push, use `git merge` instead of `git rebase` to merge any upstream changes.

### Useful commands

The following commands should be executed from the repo root directory.
- Check formatting:<br>
  `cargo fmt --check`
- Format code:<br>
  - `cargo fmt -- <filename>`
  - `cargo fmt --all` (format all packages)
- Run clippy with the same settings as the CI:<br>
  `sh clippy.sh`
- Check the state-chain and cfe compile:
  - `cargo check --all-targets`
  - `cargo check --all-targets --all-features` (This is used by the CI, but you don't typically need it)
- Run all unit tests:<br>
  `cargo test --lib`
- Expand macros for a given part of the code. You'll need to pipe output to a file.<br>
  Requires _cargo-expand_ (`cargo install cargo-expand`):<br>
  `cargo expand <options>`
- Clean up old build objects (sometimes this will fix compile problems):
  - `cargo clean`
  - `cargo clean -p <package>`
- Audit external dependencies (The CI runs this https://github.com/chainflip-io/chainflip-backend/issues/1175):<br>
  `cargo audit`