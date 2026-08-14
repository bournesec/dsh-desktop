# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] - 2026-08-14

### Added

- Add the Tauri 2 desktop client for DeepSeek Harness with the official whale logo and an integrated startup screen.
- Discover and reuse existing `dsh` commands, app-managed installations, and packages downloaded through npx.
- Automatically install the latest `@deepseek-ai/dsh` into an isolated user-level runtime when no installation is available.
- Build macOS DMG and Windows NSIS installers with GitHub Actions.
- Provide English and Simplified Chinese documentation.

### Changed

- Use a transparent macOS title bar to keep the desktop interface visually continuous.
- Update the GitHub Actions runtime and artifact upload actions.

[Unreleased]: https://github.com/bournesec/dsh-desktop/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/bournesec/dsh-desktop/releases/tag/v1.0.0
