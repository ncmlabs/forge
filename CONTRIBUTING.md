# Contributing to FORGE

Thank you for your interest in contributing to FORGE! This guide will help you get started.

## Development Setup

### Prerequisites

- [Rust](https://rustup.rs/) (stable toolchain, MSRV 1.85)
- Git

### Getting Started

```bash
git clone https://github.com/ncmlabs/forge.git
cd forge
cargo build
cargo test
```

### Running FORGE Programs

```bash
cargo run -- run examples/basics/hello.forge
cargo run -- check examples/basics/hello.forge
cargo run -- parse examples/basics/hello.forge
```

## Making Changes

### Workflow

1. Fork the repository
2. Create a feature branch from `main`
3. Make your changes
4. Run the checks below
5. Submit a pull request

### Before Submitting

Ensure all checks pass:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

### Code Style

- Run `cargo fmt` before committing
- All clippy warnings must be resolved
- Follow existing patterns in the codebase

### Tests

- Add tests for new functionality
- Run the full test suite with `cargo test`
- Conformance tests live in `conformance/` and validate language semantics

## Pull Request Guidelines

- Keep PRs focused on a single change
- Include a clear description of what and why
- Reference related issues (e.g., "Fixes #123")
- Ensure CI passes before requesting review

## Reporting Issues

- Use the bug report template for bugs
- Use the feature request template for new ideas
- Include a minimal `.forge` file that reproduces the issue when possible

## License

By contributing, you agree that your contributions will be licensed under the MIT OR Apache-2.0 license.
