#!/usr/bin/env python3
"""Small synthetic check for OCI content binding, platform and SBOM validation."""
import hashlib
import io
import json
from pathlib import Path
import runpy
import tarfile
import tempfile

verify = runpy.run_path(str(Path(__file__).with_name('verify-oci.py')))['verify']
with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    tested = {'Architecture': 'amd64', 'Os': 'linux',
              'RootFS': {'Layers': ['sha256:' + '1' * 64]},
              'Config': {'User': '10001:10001', 'ArgsEscaped': False}}
    inspection = root / 'inspection.json'
    inspection.write_text(json.dumps([tested]))
    for case in ('valid', 'wrong-platform', 'changed-filesystem', 'changed-user',
                 'missing-sbom', 'wrong-subject', 'missing-sqlite', 'corrupt-blob'):
        files = {}
        def blob(value):
            data = json.dumps(value).encode()
            digest = hashlib.sha256(data).hexdigest()
            files['blobs/sha256/' + digest] = data
            return {'digest': 'sha256:' + digest, 'size': len(data)}
        config = {'architecture': 'amd64', 'os': 'linux',
                  'rootfs': {'diff_ids': tested['RootFS']['Layers']},
                  'config': {'User': '10001:10001'}}
        if case == 'changed-filesystem':
            config['rootfs']['diff_ids'] = ['sha256:' + '2' * 64]
        if case == 'changed-user':
            config['config']['User'] = '0:0'
        image = blob({'config': blob(config), 'layers': []})
        image['platform'] = {'os': 'linux', 'architecture': 'arm64' if case == 'wrong-platform' else 'amd64'}
        packages = ['teatro', 'libc6', 'libsqlite3-0', 'libboost1.83-dev']
        if case == 'missing-sqlite':
            packages.remove('libsqlite3-0')
        statement = {'predicateType': 'https://spdx.dev/Document',
                     'subject': [{'digest': {'sha256': image['digest'][7:]}}],
                     'predicate': {'packages': [{'name': name} for name in packages]}}
        if case == 'missing-sbom':
            statement['predicate']['packages'] = []
        if case == 'wrong-subject':
            statement['subject'][0]['digest']['sha256'] = '0' * 64
        attestation = blob({'layers': [blob(statement)]})
        attestation['annotations'] = {'vnd.docker.reference.digest': image['digest']}
        index = blob({'manifests': [image, attestation]})
        if case == 'corrupt-blob':
            files['blobs/sha256/' + image['digest'][7:]] = b'{}'
        files['index.json'] = json.dumps({'manifests': [index]}).encode()
        archive_path = root / 'image.tar'
        with tarfile.open(archive_path, 'w') as archive:
            for name, data in files.items():
                info = tarfile.TarInfo(name)
                info.size = len(data)
                archive.addfile(info, io.BytesIO(data))
        try:
            digest = verify(archive_path, 'amd64', inspection)
        except ValueError:
            assert case != 'valid', case
        else:
            assert case == 'valid' and digest == index['digest'], case
print('OCI verification: platform, filesystem/configuration, SBOM subject/content and blob integrity passed.')
