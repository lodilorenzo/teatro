#!/usr/bin/env python3
"""Exercise scoped Trivy filtering against a real scan. Requires pinned Trivy on PATH."""
import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from urllib.parse import unquote

source = json.loads(Path(sys.argv[1]).read_text())
policy = json.loads(Path(__file__).with_name('trivy-ignore.yaml').read_text())
base = copy.deepcopy(source)
rule = policy['vulnerabilities'][0]
purl = rule['purls'][0]
name, version = purl.rsplit('@', 1)
chosen = {'VulnerabilityID': rule['id'], 'Severity': 'HIGH',
          'PkgName': name.rsplit('/', 1)[1], 'InstalledVersion': unquote(version.split('?')[0]),
          'PkgIdentifier': {'PURL': purl}}
for result in base['Results']:
    result['Vulnerabilities'] = []
base['Results'][0]['Vulnerabilities'] = [chosen]
checker = Path(__file__).with_name('check-image-security.py')
command = 'import runpy,sys; sys.exit(runpy.run_path(sys.argv[1])["check"](sys.argv[2],sys.argv[3]))'
with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    for case in ('approved', 'new-version', 'other-package', 'new-advisory',
                 'expired', 'critical', 'missing-purl', 'missing-rust', 'eol'):
        data, rules = copy.deepcopy(base), copy.deepcopy(policy)
        finding = data['Results'][0]['Vulnerabilities'][0]
        if case == 'new-version':
            prefix, tail = finding['PkgIdentifier']['PURL'].rsplit('@', 1)
            finding['PkgIdentifier']['PURL'] = prefix + '@0.0.0?' + tail.split('?', 1)[1]
            finding['InstalledVersion'] = '0.0.0'
        elif case == 'other-package':
            _, tail = finding['PkgIdentifier']['PURL'].rsplit('@', 1)
            finding['PkgIdentifier']['PURL'] = 'pkg:deb/debian/unreviewed@' + tail
            finding['PkgName'] = 'unreviewed'
        elif case == 'new-advisory':
            finding['VulnerabilityID'] = 'CVE-2099-123456'
        elif case == 'expired':
            for rule in rules['vulnerabilities']:
                rule['expired_at'] = '2000-01-01T00:00:00Z'
        elif case == 'critical':
            finding['Severity'] = 'CRITICAL'
        elif case == 'missing-purl':
            del finding['PkgIdentifier']['PURL']
        elif case == 'missing-rust':
            data['Results'] = data['Results'][:1]
        elif case == 'eol':
            data['Metadata']['OS']['EOSL'] = True
        report, ignore = root / 'scan.json', root / 'ignore.yaml'
        report.write_text(json.dumps(data))
        ignore.write_text(json.dumps(rules))
        result = subprocess.run([sys.executable, '-c', command, str(checker), str(report), str(ignore)],
                                capture_output=True, text=True)
        expected = 0 if case == 'approved' else 1
        assert result.returncode == expected, (case, result.stdout, result.stderr)
print('Image security policy: exact package/version, new CVE, expiry, critical and incomplete-scan guards passed.')
