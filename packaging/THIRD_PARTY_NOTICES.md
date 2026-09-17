# Third-party notices for the Teatro Docker image

Teatro's original code and artwork use CC BY-NC-SA 4.0. The image retains the
project license at `/usr/share/doc/teatro/LICENSE`. This license does not replace
any third-party terms below. Source-tree provenance and the full locked Cargo
inventory remain in [../THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md).

## Interface assets

- The 62 platform icons use the pinned ES-DE System Icon Set under CC0-1.0.
  The image includes its full text as `PLATFORM-ICONS-CC0`.
- Selected SVG paths use Lucide revision `658573b0171e693bc965c167592cc0b92d002a3e`
  under ISC and Feather under MIT. The local pencil adapts Feather v4.29.2
  `edit-3`. Copyright notices, the inventory and full texts are retained as
  `LUCIDE-ICONS-LICENSE`.
- Atkinson Hyperlegible Next belongs to the Atkinson Hyperlegible Next Project
  Authors and uses SIL OFL-1.1. Its notice is `ATKINSON-HYPERLEGIBLE-NEXT-OFL`.

These files are under `/usr/share/doc/teatro/`. Third-party system trademarks are
not licensed by the icon-set author's CC0 dedication. No endorsement is implied.

## Rust dependencies

`/usr/share/doc/teatro/rust/` contains each selected crate's reviewed license and
notice files, its archive checksum and declared/selected terms in
`DEPENDENCIES.tsv`, and a `SHA256SUMS` inventory. This is a conservative set of
normal and build dependencies for the actual target, not a claim that all are
linked into the binary. The build fails on an absent review, changed checksum,
changed license or missing notice. The supplemental `crc-catalog` MIT notice and
its exact upstream provenance are recorded in [licenses/README.md](licenses/README.md).

Rust's standard-library copyright inventory and license texts are retained in
`rust/rust-standard-library/`. The build also embeds Rust dependency metadata in
the executable using pinned `cargo-auditable`. Its Cargo metadata can include
disabled optional dependencies, including SQLx's MySQL/RSA dependencies. The
separate `ACTIVE-DEPENDENCIES.txt` records Cargo tree's native normal/build graph;
the build rejects RSA in that graph. Generated SBOMs retain the metadata superset
rather than silently deleting entries. No Rust dependency license is replaced by
Teatro's noncommercial restriction.

## SQLite

The Docker build sets `LIBSQLITE3_SYS_USE_PKG_CONFIG=1` and dynamically links
Debian's maintained `libsqlite3-0`, rather than compiling the SQLite 3.46.0 copy
in `libsqlite3-sys`. Its package version, copyright record and corresponding
source are included with the other Debian components. The build checks the ELF
dependency and the native smoke test checks library loading.

This is a container packaging choice. Ordinary Cargo builds without that
environment variable still use bundled SQLite and are not covered by the image
security review.

## Separate extractor

The image contains a separate `innoextract` executable, copyright Daniel Scharrer
and contributors, under the zlib license. Its revision, source archive checksum
and supported installer version are pinned in `innoextract/version.env`.

`/usr/share/doc/teatro/innoextract/` retains:

- the exact upstream source archive, license, README and changelog;
- the build provenance, package versions and runtime-library report;
- the reported extractor version; and
- the copyright/license records for Boost, bzip2, liblzma, zlib, zstd, libstdc++
  and libgcc used by the static build.

Compiler runtime libraries retain their upstream licenses and applicable GCC
Runtime Library Exception. The build checks the extractor's dynamic dependencies
rather than assuming that every requested static dependency linked statically.
The extractor is a subprocess, not linked into Teatro.

`unar`, `lsar`, `unrar` and `rar` are not bundled. Installers requiring a RAR helper
remain unsupported unless the operator separately provides a reviewed helper.
The GOG importer remains disabled by default and is intended for installers the
operator is entitled to use. Teatro and the extractor are not affiliated with GOG
or Inno Setup.

## Debian runtime components and corresponding source

The runtime uses Debian 13 with digest-pinned bases and the package snapshot
recorded by the Dockerfile. Packages retain their upstream copyright/license
records under `/usr/share/doc/`. The additional Teatro records are:

- `runtime-packages.tsv`, the installed binary package versions;
- `runtime-sources.txt`, the corresponding source package versions; and
- `debian-source/`, the matching Debian source archives, Debian patches, source
  control files and `SHA256SUMS`.

The sources accompany the binaries in the image instead of relying on a future
source request or an unversioned download link. They remain under their own
licenses, not Teatro's. Bundling these sources increases image size.

These notices describe the image recipe. Publication still requires review of
the actual images, security results, SBOMs and signatures for both architectures;
a successful build alone does not authorize redistribution or stable support.
