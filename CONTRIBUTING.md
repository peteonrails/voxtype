# Contributing to Voxtype

Thank you for your interest in contributing to Voxtype! This document provides guidelines for contributing.

## Ways to Contribute

- **Report bugs** - Open an issue describing the bug
- **Request features** - Open an issue describing the feature
- **Submit code** - Fork, make changes, and submit a pull request
- **Improve documentation** - Help make the docs clearer
- **Help others** - Answer questions in discussions

## Development Setup

### Prerequisites

- Rust (stable, 1.70+)
- Linux with Wayland
- Build dependencies:
  - Arch: `sudo pacman -S base-devel clang alsa-lib`
  - Debian: `sudo apt install build-essential libclang-dev libasound2-dev`
  - Fedora: `sudo dnf install @development-tools clang-devel alsa-lib-devel`

### Building

```bash
# Clone the repo
git clone https://github.com/peteonrails/voxtype
cd voxtype

# Build debug version (faster compilation)
cargo build

# Build release version
cargo build --release

# Run tests
cargo test

# Run with verbose output
cargo run -- -vv
```

### Code Style

- Follow Rust conventions (use `cargo fmt` and `cargo clippy`)
- Write clear commit messages
- Add tests for new functionality
- Update documentation as needed

### Local pre-push smoke

Run this before pushing a branch. It mirrors the parallel jobs that feed the
`ci-success` aggregator and catches the common failures locally:

```bash
cargo fmt && cargo clippy --all-targets --no-deps -- -D warnings && cargo test
```

If any step fails, fix it before pushing. CI runs the same checks and will
block the merge otherwise.

## Branching

Voxtype uses a three-branch flow:

1. `dev` is the default branch and the base for all PRs. Day-to-day work
   merges here once `ci-success` passes.
2. `rc/x.y.z` branches are cut from `dev` for release candidates. Pushing to
   an `rc/*` branch triggers the full build matrix (Linux variants and macOS).
3. `main` only receives merges from a green `rc/x.y.z` branch and is tagged
   `vX.Y.Z` at release time.

Branch protection requires:

- `dev`: the `ci-success` aggregator check (which depends on the parallel
  `fmt`, `clippy`, and `test` jobs).
- `main`: `ci-success`, `linux-ci-success`, and `macos-ci-success`.

Open PRs against `dev`. Draft PRs are encouraged while CI is red; mark Ready
for Review once `ci-success` is green.

## Submitting Changes

### For Bug Fixes

1. Create an issue describing the bug
2. Fork the repository
3. Create a branch from `dev`: `git checkout -b fix/description origin/dev`
4. Make your fix
5. Test thoroughly (run the local pre-push smoke above)
6. Submit a pull request against `dev` referencing the issue

### For Features

1. Open an issue to discuss the feature first
2. Wait for feedback before investing significant time
3. Fork and create a branch from `dev`: `git checkout -b feature/description origin/dev`
4. Implement the feature
5. Add tests and documentation
6. Submit a pull request against `dev`

### Commit Messages

Use clear, descriptive commit messages:

```
type: short description

Longer description if needed. Explain what and why,
not how (the code shows how).

Fixes #123
```

Types: `fix`, `feat`, `docs`, `style`, `refactor`, `test`, `chore`

## Refactoring while contributing

If you're already inside a file to fix a bug or add a feature, it's fine to
clean up what you're touching. The rules are short.

Stay inside the files your change already touches. If you notice something
ugly in a neighbouring module, open an issue and link it from your PR; don't
expand the diff. Adjacent cleanup is how PRs grow until they don't ship.

Wait for the third call site before extracting a helper. Two might be a
coincidence. The same shape happening to recur in places that answer
different questions is also a coincidence; don't merge those. Genuine
duplication of a fact across files (the same name spelled out repeatedly, the
same parse logic copy-pasted) is the case worth fixing.

Put the cleanup in its own commit, separate from the behaviour change. It
makes the PR easier to review and easier to revert in pieces if needed. If
you're refactoring something with no test coverage, write a small test that
pins current behaviour before you change anything.

If your cleanup is going to add more than half a day of work, stop and split.
Ship the feature; do the refactor as a follow-up PR. Don't invent abstractions
for a single implementation, and don't split a file just because it's long.
File splits and other structural decisions are
[the maintainer's call](docs/REFACTORING.md).

If you're not sure whether a cleanup belongs in your PR, ask in the
description before doing it. Skipping a refactor is always cheaper than
reverting one.

## Code of Conduct

Please read our [Code of Conduct](CODE_OF_CONDUCT.md) before contributing. We are committed to providing a welcoming and positive experience for everyone.

## Maintainer Availability

I typically respond to issues and PRs within 48-72 hours. For urgent bugs affecting core functionality, mention @peteonrails in your issue.

## Questions?

Open a discussion at https://github.com/peteonrails/voxtype/discussions

## Feedback

We want to hear from you! Voxtype is a young project and your feedback helps make it better.

- **Something not working?** If Voxtype doesn't install cleanly, doesn't work on your system, or is buggy in any way, please [open an issue](https://github.com/peteonrails/voxtype/issues). I actively monitor and respond to issues.
- **Like Voxtype?** I don't accept donations, but if you find it useful, a star on the [GitHub repository](https://github.com/peteonrails/voxtype) and/or a vote on the [AUR package](https://aur.archlinux.org/packages/voxtype/) would mean a lot!

## Credit where it is due

If you submit code that is incorporated into Voxtype, whether I modify it or accept it unchanged, I will credit you and list you as a contributor. If, for some reason, you do not want this, please let me know. I'll make every effort to: 

1. Include your commits with your github username
2. Add you to the README
3. Add you to the website pages

Bonus points to you if you make this easy for me by including those changes in your pull request. It's not required, but it is appreciated. 

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
