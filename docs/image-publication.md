# Beta container publication

[Docker operations](docker.md) · [Packaging notices](../packaging/THIRD_PARTY_NOTICES.md)

The destination is `ghcr.io/lodilorenzo/teatro`. Publication is manual and beta
only. The image tag comes from [Cargo.toml](../Cargo.toml), with no `latest` tag,
Git tag or stable GitHub Release. Check the
[package page](https://github.com/users/lodilorenzo/packages/container/package/teatro)
and completed workflow before assuming a version is available.

## What the workflow checks

[Beta Docker images](../.github/workflows/docker.yml) requires an owner-reviewed
full source SHA on `main` and passing `rust`, `msrv`, `browser`, `shell-and-audit`
and `docker` checks. It builds on native amd64 and arm64 GitHub runners without
emulation. Each image must pass fresh-volume startup, authentication, persistence
across container replacement, clean shutdown and package notice/source checks.

The service runs as UID/GID 10001. Compose and the tests drop all capabilities
and set no-new-privileges. The image removes setuid/setgid bits. Do not run it as
root, add capabilities or use privileged mode to bypass storage permissions.

Each native job scans the runtime and exports OCI content with SPDX SBOMs and
BuildKit provenance. The exported filesystem and runtime configuration must
match the smoke-tested image. The publishing job promotes those blobs without
rebuilding. It signs and verifies the multi-platform index and native indexes
with Sigstore's GitHub Actions identity before creating the version tag. The
workflow refuses an existing version tag; install by digest rather than relying
on a tag staying unchanged outside this workflow.

## Known findings and scanner limits

The initial Debian 13 review accepted 12 specific high-severity advisories for
the default non-root service. The exact CVE/package-version pairs and reasons are
in [the exception file](../packaging/trivy-ignore.yaml). They expire at
2026-10-17 00:00 UTC. These are accepted beta risks, not package fixes or an
independent security audit. New package versions require review. Critical
findings, unreviewed highs, missing package identifiers, incomplete scans and
expired exceptions block publication.

Unfiltered runtime scans remain workflow artifacts for 30 days, even when a
security gate fails. OCI build artifacts last one day; published SBOMs and
provenance accompany their registry digests. The image retains its exception
file at `/usr/share/doc/teatro/SECURITY-EXCEPTIONS.yaml`. Existing deployments do
not stop automatically when an exception expires. Review them and move to a
newly reviewed image.

Cargo-auditable uses Cargo metadata, which can overreport disabled optional
SQLx/MySQL/RSA dependencies. The image separately retains Cargo tree's native
normal/build graph and rejects RSA in that graph. The source CI's guarded Cargo
audit remains mandatory. SBOMs also include extractor build-stage packages, so
not every listed package is runtime code. Embedded interface assets have separate
inventories and licenses in the packaging notices. A clean scanner result does
not establish that every embedded component is free of vulnerabilities.

The Docker binary uses Debian's patched SQLite library. Plain Cargo builds still
use the bundled SQLite copy unless explicitly configured otherwise; the image
review does not qualify those binaries.

## Maintainer procedure

1. Review sanitized source changes and redistribution terms. Never merge private
   development history. Obtain owner approval before pushing public source.
2. Keep required checks enforced for administrators. If a temporary public review
   branch is needed to run CI before advancing `main`, include that branch in the
   owner's publication approval. Do not bypass protection.
3. Wait for all five source checks on the approved `main` SHA. Run the manual
   workflow with that full `source_sha` and `publish=false` to check packaging
   without registry writes.
4. Review both scans, known exceptions, SBOMs, notices and source archives. Obtain
   image-publication approval, then dispatch with the same source SHA and
   `publish=true`. This run builds and tests its own artifacts before promotion.
   Do not use broad advisory ignores or waive a failed architecture.
5. For the first upload, verify the GHCR package is linked to this repository and
   set its visibility to public through package settings. Do not assume a public
   repository makes its new package public automatically.
6. Verify anonymous pulls of both platforms, the signed index digest and native
   smoke/persistence tests on the pulled images. Retain the exact digests, source
   SHA, reports and workflow URL before announcing availability.

GitHub Actions needs `packages: write` and `id-token: write` only in the publishing
job. It uses its short-lived token and keyless signing, not a stored personal
package token or signing key. Publishing failures may leave untagged blobs or
attestations in the registry. Inspect them before recovery; do not overwrite an
existing version or silently rerun failed tests.

Production recovery qualification, independent reproducibility, additional host
architectures and client-device qualification remain separate work. Signed beta
images are not a stable or production-qualified release.
