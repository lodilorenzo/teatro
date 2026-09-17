# Third-party notices

Teatro's [project license](LICENSE) applies to its original code, documentation
and artwork only. It does not replace the licenses below or grant rights to
third-party trademarks, ROMs, installers, game covers or metadata supplied by
operators and external services.

| Material in this source tree | Source and terms | Retained notice |
| --- | --- | --- |
| 62 platform PNGs in `web/public/platform-icons/` | [ES-DE System Icon Set](https://github.com/Zoidburg13/ES-DE-System-Icon-Set/tree/5ede66c7156ec1bf4dec8aaba0bb24cb1bb97584), CC0-1.0; resized to 128×128, with `windows.png` renamed `win.png` | [CC0 text](web/public/platform-icons/LICENSE) |
| SVG paths in `web/public/icons.js`, shared by the web interfaces | [Lucide revision 658573b0171e693bc965c167592cc0b92d002a3e](https://github.com/lucide-icons/lucide/tree/658573b0171e693bc965c167592cc0b92d002a3e), ISC and Feather MIT; local pencil adapts [Feather v4.29.2 edit-3](https://github.com/feathericons/feather/blob/v4.29.2/icons/edit-3.svg) | [Copyright notices, icon inventory and full ISC/MIT texts](web/public/icons.LICENSE) |
| Atkinson Hyperlegible Next Latin and LatinExt WOFF2 files in `web/fonts/` | [Atkinson Hyperlegible Next Project Authors](https://github.com/googlefonts/atkinson-hyperlegible-next), SIL OFL-1.1 | [Copyright notice and full OFL text](web/fonts/AtkinsonHyperlegibleNext-OFL.txt) |

Platform marks identify supported systems. CC0 covers rights held by the icon-set
author, not third-party trademark rights. No affiliation or endorsement is implied.
The fonts remain under OFL-1.1, including their restriction on standalone sale.

The [README header](docs/assets/readme-header.png) reuses Teatro's original
[application icon](web/admin/teatro.svg) and renders the lowercase wordmark in
Atkinson Hyperlegible Next Bold. It matches the [website](https://teatro.host/)
header at 1.5 times its desktop size, with the same spacing and colors. The PNG
is rendered at twice its display resolution using the font retained in `web/fonts/`.

## Rust dependencies and build tools

Cargo fetches third-party crates separately. No crate sources or compiled libraries
are vendored in this source tree. [The locked inventory](RUST_DEPENDENCY_LICENSES.tsv)
records all 304 external packages, their exact versions, declared terms, selected
license route, crate checksums and notice locations within their published archives.
Dependencies retain their own licenses, including their permissions for commercial
use independently of Teatro.

Publishing these source files and build recipes does not redistribute the
`innoextract` executable, its source archive, Cargo dependencies, compiler runtimes
or operating-system packages. Generated distributions need their own complete
license and notice set for the exact components they contain. See the
[container notices](packaging/THIRD_PARTY_NOTICES.md) for the separately executed,
zlib-licensed `innoextract` and the existing sidecar obligations. This source review
does not authorize a binary, container or sidecar release.
