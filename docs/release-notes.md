# Experimental first release

NETGEAR AV switch inspection and targeted PoE power cycling through a Rust CLI,
SSH-friendly TUI, Omarchy Quickshell plugin, and portable AI-agent skill.

- Per-port link/speed/power state, full-MAC identity and offline manufacturer lookup.
- Preferred LLDP neighbor follows NETGEAR AV UI ordering; downstream evidence stays expandable.
- Matching desktop/terminal colors and immediate one-click/Space power cycling.
- Explicit CLI `--yes`, dry run, protected ports, preflight validation and no reset retries.
- Credentials in `~/.netgear-poe.toml`, mode 0600; never in arguments or repository.

**Compatibility:** read-only hardware checks on M4250-10G2XF-PoE++, firmware
13.0.4.14 only. Native reset payload covered by mock tests, but real hardware
power-cycle recovery has NOT been verified by the maintainer. No claim of
support for other models/firmware or stacked-switch resets.

**Safety:** clicking a lightning button or pressing Space in TUI immediately
interrupts PoE, without confirmation. Protect uplinks and critical ports first.
HTTP is unencrypted; use trusted HTTPS where available. This is an unofficial
community project, not affiliated with NETGEAR.

Linux x86_64 and aarch64 static musl archives are provided with SHA-256 checksums.
Omarchy plugin and agent skill are installed separately from this repository;
see README. Optional manufacturer database: `/usr/share/hwdata/oui.txt`.
