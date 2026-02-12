# Contributing to g2d-rs

Thank you for your interest in contributing! This project provides Rust bindings for the NXP i.MX G2D 2D graphics accelerator.

## Code of Conduct

Please read our [Code of Conduct](CODE_OF_CONDUCT.md) before contributing.

## Ways to Contribute

- **Code**: Bug fixes, new features, performance improvements
- **Documentation**: Improvements, examples, platform notes
- **Testing**: Bug reports, hardware platform validation
- **Community**: Answer questions, share use cases

## Development Setup

### Prerequisites

- Rust 1.70 or later
- Linux (for testing with actual G2D hardware)
- Optional: NXP i.MX8/i.MX9 platform for hardware testing

### Build and Test

```bash
# Clone the repository
git clone https://github.com/EdgeFirstAI/g2d-rs.git
cd g2d-rs

# Build
cargo build --workspace

# Run tests (will skip hardware tests if G2D not available)
cargo test --workspace

# Format code
cargo +nightly fmt --all

# Lint
cargo clippy --workspace
```

## Contribution Process

### 1. Fork and Clone

```bash
git clone https://github.com/YOUR_USERNAME/g2d-rs.git
cd g2d-rs
git remote add upstream https://github.com/EdgeFirstAI/g2d-rs.git
```

### 2. Create Feature Branch

```bash
git checkout -b feature/your-feature-name
```

### 3. Make Changes

- Follow the code style (rustfmt, clippy)
- Add tests for new functionality
- Update documentation as needed

### 4. Test Your Changes

```bash
cargo test --workspace
cargo clippy --workspace
cargo +nightly fmt --all --check
```

### 5. Commit and Push

```bash
git add .
git commit -s -m "Add new feature"
git push origin feature/your-feature-name
```

### 6. Submit Pull Request

1. Go to [g2d-rs repository](https://github.com/EdgeFirstAI/g2d-rs)
2. Click "New Pull Request"
3. Fill out the PR template
4. Wait for CI checks to pass
5. Address review feedback

## Code Style

- Use `rustfmt` for formatting
- Use `clippy` for linting
- Document public APIs with doc comments
- Use `Result<T, E>` for error handling

## Testing on Hardware

If you have access to NXP i.MX hardware:

1. Ensure `libg2d.so.2` is installed
2. Run tests with `LIBG2D_PATH` if needed:
   ```bash
   LIBG2D_PATH=/usr/lib/libg2d.so.2 cargo test
   ```

## License

By contributing, you agree that your contributions will be licensed under the Apache License 2.0.
