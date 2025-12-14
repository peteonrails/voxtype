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

## Submitting Changes

### For Bug Fixes

1. Create an issue describing the bug
2. Fork the repository
3. Create a branch: `git checkout -b fix/description`
4. Make your fix
5. Test thoroughly
6. Submit a pull request referencing the issue

### For Features

1. Open an issue to discuss the feature first
2. Wait for feedback before investing significant time
3. Fork and create a branch: `git checkout -b feature/description`
4. Implement the feature
5. Add tests and documentation
6. Submit a pull request

### Commit Messages

Use clear, descriptive commit messages:

```
type: short description

Longer description if needed. Explain what and why,
not how (the code shows how).

Fixes #123
```

Types: `fix`, `feat`, `docs`, `style`, `refactor`, `test`, `chore`

## Code of Conduct

Please read our [Code of Conduct](CODE_OF_CONDUCT.md) before contributing. We are committed to providing a welcoming and positive experience for everyone.

## Maintainer Availability

I typically respond to issues and PRs within 48-72 hours. For urgent bugs affecting core functionality, mention @peteonrails in your issue.

## Questions?

Open a discussion at https://github.com/peteonrails/voxtype/discussions

## Feedback

We want to hear from you! Voxtype is a young project and your feedback helps make it better.

- **Something not working?** If Voxtype doesn't install cleanly, doesn't work on your system, or is buggy in any way, please [open an issue](https://github.com/peteonrails/voxtype/issues). I actively monitor and respond to issues.
- **Like Voxtype?** I don't accept donations, but if you find it useful, a star on the [GitHub repository](https://github.com/peteonrails/voxtype) would mean a lot!

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
