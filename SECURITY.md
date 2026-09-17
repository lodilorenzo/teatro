# Security policy

## Supported versions

Teatro is currently beta software. Security fixes are applied to the latest commit
on the public `main` branch. No stable binary release is supported yet.

## Report a vulnerability

Use [GitHub private vulnerability reporting](https://github.com/lodilorenzo/teatro/security/advisories/new).
Do not open a public issue for a vulnerability.

Include the affected commit, reproduction steps, impact, and any suggested fix.
Remove credentials, private paths, ROM metadata, and other personal library data
from the report. Reports are reviewed privately, but no response-time guarantee is
offered during beta.

Do not expose Teatro directly to the Internet over plaintext HTTP. Use a trusted
local network, VPN, or correctly configured HTTPS reverse proxy.
