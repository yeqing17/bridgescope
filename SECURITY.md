# Security policy

## Reporting

Do not open a public issue for a vulnerability that could target a connected Android device. Report it privately to the future project maintainer. This repository does not yet publish a security contact.

## Security model

- No device is selected implicitly when multiple devices are attached.
- Structured commands include an exact device serial and device generation.
- Local processes are started with executable/argument arrays, not shell command strings.
- Commands and streams have timeouts, cancellation, output bounds, and cleanup.
- Secrets, terminal input, clipboard data, and file contents are not logged by default.
- Delete, uninstall, clear-data, root, reboot, and AVD wipe operations require risk-appropriate confirmation enforced by the Rust backend.

The interactive terminal can execute arbitrary Android shell commands and must be treated as an unrestricted expert interface. Its input/output and clipboard contents are not intentionally written to diagnostic logs. Screenshot captures can contain sensitive data; BridgeScope keeps them in memory unless the user explicitly chooses Save PNG.
