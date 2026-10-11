# Contributing to Tileling

Thanks for helping! Bug reports, ideas and pull requests are all welcome.

## Reporting a problem

Open an [issue](https://github.com/alexandreravelli/tileling/issues) with the output of
`tileling --version`, your distribution, desktop and window manager, and
`tileling --check-config`. Run `TILELING_DEBUG=1 tileling --replace` from a terminal to see what
Tileling does (each drag, each tiling, X errors). For a security problem, follow
[SECURITY.md](SECURITY.md) instead.

## Setting up

Tileling needs Rust 1.92 or newer, and nothing else to build:

```sh
sudo apt install -y build-essential curl git xvfb
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
rustup target add x86_64-unknown-linux-musl   # for the static binary of the releases
git clone https://github.com/alexandreravelli/tileling
cd tileling
```

The code is split in two:

- the library (`src/lib.rs`): the layouts and their geometry (`layout.rs`) and the
  configuration file (`config.rs`), pure functions documented on [docs.rs](https://docs.rs/tileling);
- the program (`src/main.rs`, `app.rs`): XInput 2, the picker, the preview and the assistant.

Try a change on your session with `cargo run -- --replace`; `tileling --replace` brings yours
back. `examples/xtest.rs` drives the real pointer through XTest to replay a drag.

## Before you open a pull request

CI runs these checks; run them first:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test                  # tests/x11.rs runs Tileling on Xvfb when it is installed
cargo doc --no-deps
cargo deny check            # cargo install cargo-deny
```

- Keep pull requests small, with one topic each, and add tests for new behaviour. Geometry
  belongs in `layout.rs`, where it can be tested without an X server.
- Document every public item of the library; examples in doc comments are run as tests.
- Keep Tileling light: one static binary, no toolkit, and think twice before adding a dependency.
- Write code, comments, messages and docs in English.

## Commit messages

Commits follow [Conventional Commits](https://www.conventionalcommits.org/): `type(scope): summary`,
in the imperative, under 72 characters, with a body that explains why when it is not obvious.

| Type | For |
| --- | --- |
| `feat` | a new feature |
| `fix` | a bug fix |
| `docs`, `test`, `refactor`, `style`, `perf` | documentation, tests, code changes without new behaviour |
| `build`, `ci`, `chore` | dependencies, packaging, workflows, maintenance |

Add `!` after the type (`feat!:`) for a change that breaks the configuration file, the command
line or the library's API. The release notes and the next version number are computed from these
messages.

## Releases

[release-plz](https://release-plz.dev) keeps a release pull request open: it bumps the version
from the commit messages and writes the [changelog](CHANGELOG.md). Merging it makes the Release
workflow run every check, build the static binary and the Debian package, publish the GitHub
release and, once enabled, the crates.io version.
