# Clean-room policy

BridgeScope reimplements Android debugging workflows independently.

## Allowed inputs

- Public Android and ADB documentation.
- Public scrcpy and Chrome DevTools protocol documentation.
- Observable input/output behavior from tools used on devices owned or authorized by contributors.
- Fixtures captured by BridgeScope's own tests.

## Prohibited inputs

- Copying, translating, porting, or mechanically transforming AYA source code.
- Copying AYA icons, screenshots, CSS, text, translations, test fixtures, package identifiers, bytecode, or device helper.
- Presenting BridgeScope as an official AYA or Android product.

Each protocol feature must cite normative sources in `protocol-sources.md`. If protected AYA code is ever intentionally incorporated, clean-room work stops until licensing obligations, including possible AGPL-3.0 requirements, are reviewed.
