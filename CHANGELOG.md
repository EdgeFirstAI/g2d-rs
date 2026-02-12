# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.1.0] - 2026-02-12

### Changed

- Moved to standalone repository (previously part of EdgeFirst HAL)
- Changed license from MIT to Apache-2.0 for consistency
- Updated to use workspace version inheritance
- Added comprehensive documentation

### Added

- ARCHITECTURE.md documenting ABI compatibility handling
- GitHub Actions workflows for CI/CD
- SBOM generation and license compliance checking

## [1.0.1] - 2025-11-15

### Fixed

- ABI compatibility with older G2D library versions (< 6.4.11)
- Version detection from `_G2D_VERSION` symbol

## [1.0.0] - 2025-10-01

### Added

- Initial release of g2d-sys FFI bindings
- Dynamic loading via libloading
- Support for G2D blit, clear, and format conversion operations
- Version detection and ABI adaptation
- Support for RGB, RGBA, NV12, YUYV formats
