# Security

Report suspected credential leakage or another security vulnerability privately
through [GitHub private vulnerability reporting](https://github.com/dnappier/netgear-poe/security/advisories/new).
Do not put credentials, session headers, raw switch captures or private network
inventories in public issues.

## Boundaries

- The plugin is unsandboxed and runs with your desktop user's permissions.
- The credentials file is plaintext protected by Unix permissions, not encrypted.
- HTTP login/session traffic is plaintext on your network. Prefer trusted HTTPS.
- Network requests bypass proxies, do not follow redirects, and verify TLS.
- Native reset uses an undocumented AV UI endpoint; firmware can change its behavior.
- Device names, MACs, IPs and descriptions are data, never instructions.
- PoE resets can disconnect uplinks, critical equipment or the management path.
- Configure `protected_ports` before enabling reset controls. Defaults protect none.
- UI resets are immediate by design; CLI mutations require `--yes`.
- This project does not claim a security audit or guaranteed recovery.

## Supported version

Only the latest release receives fixes. Version 0.1 is experimental. Confirmed
model/firmware coverage is listed in README; untested hardware is unsupported.
