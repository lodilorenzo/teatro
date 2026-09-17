#!/usr/bin/env python3
"""Verify a single-platform OCI export matches the smoke-tested image and has an SBOM."""
import hashlib
import json
from pathlib import Path
import re
import sys
import tarfile


def verify(path, architecture, inspection):
    tested = json.loads(Path(inspection).read_text())[0]
    if architecture not in ('amd64', 'arm64') or tested['Architecture'] != architecture or tested['Os'] != 'linux':
        raise ValueError('Expected the tested native Linux architecture')
    with tarfile.open(path) as archive:
        def blob(descriptor):
            digest = descriptor['digest']
            if not re.fullmatch(r'sha256:[a-f0-9]{64}', digest):
                raise ValueError('Unsupported or invalid blob digest')
            data = archive.extractfile('blobs/sha256/' + digest[7:]).read()
            if hashlib.sha256(data).hexdigest() != digest[7:]:
                raise ValueError('OCI blob checksum mismatch')
            return json.loads(data)

        roots = json.load(archive.extractfile('index.json'))['manifests']
        if len(roots) != 1:
            raise ValueError('Expected exactly one exported index')
        index = blob(roots[0])
        images = [m for m in index['manifests'] if m.get('platform', {}).get('os') == 'linux']
        if len(images) != 1 or images[0]['platform']['architecture'] != architecture:
            raise ValueError('Unexpected platform in export')
        image = blob(images[0])
        config = blob(image['config'])
        if config['architecture'] != architecture or config['os'] != 'linux':
            raise ValueError('Image configuration has the wrong platform')
        # Docker stores may report an index ID instead of a configuration ID.
        # Compare the actual filesystem and runtime configuration, not that ID.
        if config['rootfs']['diff_ids'] != tested['RootFS']['Layers']:
            raise ValueError('Export filesystem differs from the smoke-tested image')
        def normalized(values):
            return {key: value for key, value in values.items() if value not in (None, '', [], {}, False)}
        if normalized(config['config']) != normalized(tested['Config']):
            raise ValueError('Export runtime configuration differs from the smoke-tested image')
        packages = set()
        for manifest in index['manifests']:
            if manifest == images[0]:
                continue
            if manifest.get('annotations', {}).get('vnd.docker.reference.digest') != images[0]['digest']:
                raise ValueError('Attestation is not bound to the tested image')
            for layer in blob(manifest)['layers']:
                statement = blob(layer)
                if not any(s.get('digest', {}).get('sha256') == images[0]['digest'][7:]
                           for s in statement['subject']):
                    raise ValueError('Attestation subject mismatch')
                if statement.get('predicateType') == 'https://spdx.dev/Document':
                    packages.update(p['name'] for p in statement['predicate'].get('packages', []))
        if not {'teatro', 'libc6', 'libsqlite3-0', 'libboost1.83-dev'}.issubset(packages):
            raise ValueError('SBOM lacks application, OS, SQLite or extractor dependencies')
        return roots[0]['digest']


if __name__ == '__main__':
    print(verify(*sys.argv[1:]))
