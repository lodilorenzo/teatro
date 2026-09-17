# Third-party notices for the Teatro Docker image

> Source-tree notices and the locked dependency inventory are in
> [../THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md).
> This evaluation notice inventories the Atkinson Hyperlegible Next font,
> Lucide/Feather interface icons, packaged `innoextract` sidecar, and its linked
> dependencies only. It is not
> yet a complete Teatro Rust-dependency notice or release SBOM and must not be
> treated as public release clearance.

Teatro's web interfaces include selected SVG markup from
[Lucide](https://github.com/lucide-icons/lucide), pinned to revision
`658573b0171e693bc965c167592cc0b92d002a3e` under the ISC license. Icons derived
from Feather and the local pencil adapted from Feather v4.29.2 `edit-3`
remain under Feather's MIT license. The retained notices and the
vendored icon list are in `web/public/icons.LICENSE`; the Docker image copies that
file to `/usr/share/doc/teatro/LUCIDE-ICONS-LICENSE`.

Teatro's interface typography uses [Atkinson Hyperlegible Next](https://github.com/googlefonts/atkinson-hyperlegible-next),
copyright the Atkinson Hyperlegible Next Project Authors, under the SIL Open
Font License 1.1. The retained license is in
`web/fonts/AtkinsonHyperlegibleNext-OFL.txt`; the Docker image copies it to
`/usr/share/doc/teatro/ATKINSON-HYPERLEGIBLE-NEXT-OFL`.

The Teatro Docker image contains a separately executable copy of
[`innoextract`](https://github.com/dscharrer/innoextract), copyright Daniel
Scharrer and contributors, under the zlib license.

The bundled revision is pinned in `packaging/innoextract/version.env`. The
Docker image includes:

- the exact `innoextract` source archive and its SHA-256 provenance;
- the upstream `LICENSE`, `README.md`, and `CHANGELOG`;
- the exact build-package manifest and runtime-library report; and
- copyright/license records for statically linked Boost, bzip2, liblzma, zlib,
  libstdc++, and libgcc components.

Those materials are stored under `/usr/share/doc/teatro/innoextract/` in the image.
The executable at `/opt/teatro/tools/innoextract` remains a separate subprocess
and is not linked into Teatro. The runtime operating-system packages and Teatro's
Rust dependencies still require a complete redistribution review before release.

`unar`, `lsar`, `unrar`, and `rar` are **not** bundled. Installers that require
a RAR helper remain unsupported unless an operator separately supplies and
configures a reviewed helper directory.

Teatro and the bundled tool are not affiliated with GOG or Inno Setup. The GOG
import path remains disabled by default and is intended only for private use
with installers the operator is legally entitled to use.
