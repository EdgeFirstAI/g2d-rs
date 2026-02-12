# AI Assistant Development Guidelines for g2d-rs

**Purpose:** Instructions for AI coding assistants working on this project.

---

## Project Overview

g2d-rs provides Rust bindings for the NXP i.MX G2D 2D graphics accelerator. The library is loaded dynamically at runtime using `libloading`.

### Crate Structure

```
crates/
└── g2d-sys/     # Low-level FFI bindings (current)
└── g2d/         # Safe Rust API (future)
```

## Git Workflow

### Commit Messages

**For feature/bug work (with JIRA ticket):**
```bash
PROJECTKEY-###: Brief description
```

**For housekeeping (no JIRA required):**
```bash
Release v1.1.0
Update dependencies
Fix typo in README
```

## Code Quality

- **Rust:** `cargo fmt` and `cargo clippy`
- Use `Result<T, E>` for error handling
- Document public APIs with doc comments

## Build Commands

```bash
cargo build --workspace      # Build
cargo test --workspace       # Test
cargo fmt --all               # Format
cargo clippy --workspace     # Lint
```

## Release Process

1. Update version in `Cargo.toml` workspace
2. Update `CHANGELOG.md`
3. Run `cargo check --workspace`
4. Run `make format lint check` (or cargo commands)
5. Commit: `git commit -s -m "Release vX.Y.Z"`
6. Tag: `git tag -a -s vX.Y.Z -m "Version X.Y.Z"`
7. Push: `git push origin main && git push origin vX.Y.Z`

## License Policy

**Allowed:** MIT, Apache-2.0, BSD, ISC, 0BSD  
**Disallowed:** GPL, AGPL

## Security

Report vulnerabilities to: `support@au-zone.com`
