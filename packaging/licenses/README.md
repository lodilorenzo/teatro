# Supplemental dependency notices

`crc-catalog` 2.5.0 declares `MIT OR Apache-2.0` but omits its `LICENSES`
directory from the crate archive. `crc-catalog/MIT.txt` is the unmodified MIT
notice from its recorded source revision:

- Repository: <https://github.com/akhilles/crc-catalog>
- Revision from `.cargo_vcs_info.json`: `ed4ad631f22b05055c21a3a4127eb7cf6d75bb62`
- File: <https://github.com/akhilles/crc-catalog/blob/ed4ad631f22b05055c21a3a4127eb7cf6d75bb62/LICENSES/MIT.txt>
- SHA-256: `5ef8fcfb6cccec8fcae043c834099a60c8b7406408db576e026d2b7e67dc5cf5`

The Docker build copies this notice along with the reviewed notices from Cargo's
other downloaded crates. `collect-rust-notices.py` fails if an enabled normal/build
dependency is missing from the reviewed inventory or its license/checksum changes.
The output is a conservative inventory including build dependencies, not a claim
that every listed crate is linked into the runtime binary.

`alloc-stdlib` 0.2.4 declares `BSD-3-Clause` but its crate archive contains no
license file. `alloc-stdlib/LICENSE` is the unmodified repository license:

- Repository: <https://github.com/dropbox/rust-alloc-no-stdlib>
- Revision from `.cargo_vcs_info.json`: `ae42d22078b98549e987d2f03d12df7b984fde47`
- File: <https://github.com/dropbox/rust-alloc-no-stdlib/blob/ae42d22078b98549e987d2f03d12df7b984fde47/LICENSE>
- SHA-256: `c0c56f26d9c051cac4d200c34c84e7ae9aaa853e01a982a1df08b09931e518ae`

`binrw` and `binrw_derive` 0.15.2 declare `MIT` but omit the license file from
their archives. Both use `binrw/LICENSE`, the unmodified repository license:

- Repository: <https://github.com/jam1garner/binrw>
- Revision from `.cargo_vcs_info.json`: `db9ef7bece9525057001edd260439fa2748287df`
- File: <https://github.com/jam1garner/binrw/blob/db9ef7bece9525057001edd260439fa2748287df/LICENSE>
- SHA-256: `08159509fe99d146416fe110053a7360ddfccbba5f0c8f955bdbda025d4d2a20`

`rom-converto-lib` is a Git dependency without a crate archive or declared
license field. Its MIT notice is `../rom-converto/LICENSE`, unmodified from the
repository root at the pinned revision `0bc5e29ce2c4afc3ce30b64de73f7d673f4f9817`.
