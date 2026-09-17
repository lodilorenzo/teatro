#!/usr/bin/env python3
"""Fail closed on unsupported/incomplete scans, critical findings or unreviewed highs."""
import json
from pathlib import Path
import subprocess
import sys


def check(report, policy=Path(__file__).with_name('trivy-ignore.yaml')):
    data = json.loads(Path(report).read_text())
    os_info = data['Metadata']['OS']
    if os_info['Family'] != 'debian' or os_info['Name'].split('.')[0] != '13' or os_info.get('EOSL'):
        raise ValueError('Expected a supported Debian 13 runtime')
    results = data['Results']
    if not any(r.get('Type') == 'debian' for r in results) or not any(
        r.get('Type') == 'rustbinary' and r['Target'].lstrip('/') == 'opt/teatro/teatro'
        for r in results
    ):
        raise ValueError('Scan must include both OS packages and the Rust executable')
    for result in results:
        for finding in result.get('Vulnerabilities', []):
            if finding['Severity'] == 'CRITICAL':
                raise ValueError('Critical findings cannot be excepted')
            if finding['Severity'] == 'HIGH' and not finding.get('PkgIdentifier', {}).get('PURL'):
                raise ValueError('High finding lacks a package URL for scoped matching')
    return subprocess.run([
        'trivy', 'convert', '--format', 'table', '--severity', 'HIGH,CRITICAL',
        '--exit-code', '1', '--ignorefile', str(policy), str(report),
    ], check=False).returncode


if __name__ == '__main__':
    sys.exit(check(sys.argv[1]))
