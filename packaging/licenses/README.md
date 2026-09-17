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
