# Contributing

```sh
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
python3 -m unittest discover -s tests -p 'test_*.py'
```

On an Omarchy machine:

```sh
omarchy plugin validate "$PWD"
qmllint -I /usr/share/omarchy/shell Panel.qml
```

Test with synthetic fixtures first. Never automatically power-cycle real hardware
in tests or CI. Live testing needs explicit approval of the affected port(s).
Report model, firmware, CLI version and redacted errors. Do not attach raw
credentials, sessions, serial numbers, real MAC/IP inventories or private screenshots.

Prefer small changes consistent across CLI/TUI and Quickshell. Preserve evidence
and uncertainty: an OUI is a manufacturer, not a model, and a learned MAC is not
proof of physical attachment. Firmware-specific fields need fixture coverage.

Release maintainers create a version tag after CI passes. Release workflow
builds static x86_64/aarch64 Linux binaries and publishes a prerelease with
checksums. Public preview is an illustration using synthetic data.
