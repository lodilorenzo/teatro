#!/usr/bin/env python3
"""Copy reviewed notices for this target's normal/build dependencies. No network calls."""
import csv
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib


SUPPLEMENTAL_NOTICES = {
    'alloc-stdlib': 'packaging/licenses/alloc-stdlib',
    'binrw': 'packaging/licenses/binrw',
    'binrw_derive': 'packaging/licenses/binrw',
    'crc-catalog': 'packaging/licenses/crc-catalog',
    'rom-converto-lib': 'packaging/rom-converto',
}


def collect(destination):
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--offline', '--format-version', '1']))
    host = next(line.split(': ', 1)[1] for line in subprocess.check_output(
        ['rustc', '-vV'], text=True).splitlines() if line.startswith('host: '))
    tree = subprocess.check_output([
        'cargo', 'tree', '--locked', '--offline', '--target', host,
        '--edges', 'normal,build', '--prefix', 'none', '--format', '{p}',
    ], text=True)
    selected = {tuple(line.split()[:2]) for line in tree.splitlines()}
    # RSA is allowed only through the reviewed, key-free rom-converto-lib pin.
    if any(name == 'rsa' for name, _ in selected):
        parents = subprocess.check_output([
            'cargo', 'tree', '--locked', '--offline', '--target', host,
            '--edges', 'normal,build', '--invert', 'rsa', '--depth', '1',
            '--prefix', 'none', '--format', '{p}',
        ], text=True)
        if [line.split()[:2] for line in parents.splitlines()] != [
                ['rsa', 'v0.9.10'], ['rom-converto-lib', 'v0.22.0']]:
            raise ValueError('RSA entered the native build graph outside the reviewed exception')
    packages = {(p['name'], 'v' + p['version']): p for p in metadata['packages']}
    inventory = list(csv.DictReader(Path('RUST_DEPENDENCY_LICENSES.tsv').open(), delimiter='\t'))
    reviewed = {(r['crate'], 'v' + r['version']): r for r in inventory}
    checksums = {(p['name'], 'v' + p['version']): p.get('checksum')
                 for p in tomllib.loads(Path('Cargo.lock').read_text())['package']}
    destination.mkdir(parents=True, exist_ok=False)
    (destination / 'ACTIVE-DEPENDENCIES.txt').write_text(tree)
    retained = []
    for key in sorted(selected):
        package = packages[key]
        if package['id'] == metadata['resolve']['root']:
            continue
        row = reviewed[key]
        # Git dependencies have no archive checksum; their pinned commit is reviewed instead.
        checksum = checksums[key] or 'git:' + package['source'].rsplit('#', 1)[1]
        if row['archive_sha256'] != checksum or row['declared_license'] != (package['license'] or 'NOASSERTION'):
            raise ValueError(f'License review is stale: {key}')
        source = Path(package['manifest_path']).parent
        names = row['notice_files'].split('; ')
        # These archives omit their notices. Retain the exact VCS notice instead.
        if key[0] in SUPPLEMENTAL_NOTICES:
            source = Path(SUPPLEMENTAL_NOTICES[key[0]])
        for name in names:
            path = source / name
            if not path.resolve().is_relative_to(source.resolve()) or '..' in Path(name).parts or not path.is_file():
                raise ValueError(f'Missing or unsafe reviewed notice: {key}: {name}')
            data = path.read_bytes()
            data.decode('utf-8')  # Refuse a binary mistaken for a notice.
            target = destination / f'{key[0]}-{package["version"]}' / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
        retained.append({**row, 'notice_files': '; '.join(names)})
    with (destination / 'DEPENDENCIES.tsv').open('w') as output:
        writer = csv.DictWriter(output, fieldnames=inventory[0].keys(), delimiter='\t')
        writer.writeheader()
        writer.writerows(retained)
    sysroot = Path(subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip())
    rust_docs = sysroot / 'share/doc/rust'
    shutil.copytree(rust_docs / 'licenses', destination / 'rust-standard-library/licenses')
    shutil.copyfile(rust_docs / 'COPYRIGHT-library.html',
                    destination / 'rust-standard-library/COPYRIGHT-library.html')
    files = sorted(p for p in destination.rglob('*') if p.is_file())
    (destination / 'SHA256SUMS').write_text(''.join(
        f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.relative_to(destination)}\n' for p in files))
    print(f'Retained notices for {len(retained)} normal/build dependencies on {host}, plus Rust std.')


if __name__ == '__main__':
    collect(Path(sys.argv[1]))
