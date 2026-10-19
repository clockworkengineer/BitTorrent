# Contributing to BitTorrent-rs

Thank you for your interest in contributing to the BitTorrent-rs project! This document outlines our guidelines for issues, code changes, and pull requests.

---

## Code of Conduct

All contributors and participants agree to abide by our [Code of Conduct](CODE_OF_CONDUCT.md).

---

## Development Setup

1. **Prerequisites**:
   - Rust 1.85+ (Edition 2024 support)
   - `cargo`, `rustfmt`, and `clippy`

2. **Clone and Build**:
   ```bash
   git clone https://github.com/clockworkengineer/BitTorrent.git
   cd BitTorrent
   cargo build --workspace
   ```

3. **Run the Test Suite**:
   ```bash
   cargo test --workspace
   ```

4. **Verify `#![no_std]` Compatibility**:
   ```bash
   cargo check -p bittorrent-rs --no-default-features
   ```

5. **Run Lints & Formatting**:
   ```bash
   cargo fmt --check
   cargo clippy --workspace --all-targets -- -D warnings
   ```

---

## Pull Request Guidelines

1. **Target Branch**: Submit PRs against the `main` branch.
2. **Commit Messages**: Write concise, conventional commit messages (e.g. `feat: add BEP 53 selective download`, `fix: address tracker timeout`).
3. **Tests**: Add unit or integration tests for all new features and bugfixes.
4. **Documentation**: Update doc comments (`///`), module docs (`//!`), and [CHANGELOG.md](CHANGELOG.md) under `[Unreleased]`.
5. **No Regressions**: Ensure all workspace tests pass and clippy produces zero warnings.
