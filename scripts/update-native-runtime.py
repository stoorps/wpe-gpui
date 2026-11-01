#!/usr/bin/env python3
"""Propose a pinned Arch WPE runtime update; never install or execute the package.

The candidate checksum records the bytes downloaded from the official archive.
It is NOT a replacement for the installer's independent pacman-key signature check.
This checks Arch package availability, not CVE applicability or upstream-fix timing.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sys
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
METADATA = ROOT / 'native-runtime.json'
SOURCE = 'https://archlinux.org/packages/extra/x86_64/wpewebkit/json/'
ARCHIVE = 'https://archive.archlinux.org/packages/w/wpewebkit/'
MAX_PACKAGE_BYTES = 256 * 1024 * 1024


def version_key(epoch, version, release):
    if type(epoch) is not int or epoch < 0:
        raise ValueError('Package epoch must be a nonnegative integer')
    if not isinstance(version, str) or not re.fullmatch(r'\d+\.\d+\.\d+', version):
        raise ValueError('Expected a stable numeric WPE release')
    if not isinstance(release, str) or not re.fullmatch(r'\d+(?:\.\d+)*', release):
        raise ValueError('Expected a numeric Arch package release')
    return epoch, tuple(map(int, version.split('.'))), tuple(map(int, release.split('.')))


def archive_url(version, release):
    # Callers validate fields before constructing a filename; remote URLs are ignored.
    version_key(0, version, release)
    return f'{ARCHIVE}wpewebkit-{version}-{release}-x86_64.pkg.tar.zst'


def validate_metadata(data):
    expected = {'schema_version': 1, 'source': SOURCE, 'package': 'wpewebkit',
                'repository': 'extra', 'architecture': 'x86_64'}
    if not isinstance(data, dict) or any(data.get(k) != v for k, v in expected.items()):
        raise ValueError('Unsupported native runtime metadata identity/schema')
    version_key(data.get('epoch'), data.get('version'), data.get('release'))
    if data.get('archive_url') != archive_url(data['version'], data['release']):
        raise ValueError('Archive URL does not match the pinned official package')
    if not isinstance(data.get('sha256'), str) or not re.fullmatch(r'[0-9a-f]{64}', data['sha256']):
        raise ValueError('Expected a SHA256 package checksum')
    return data


def load_metadata(path=METADATA):
    return validate_metadata(json.loads(Path(path).read_text()))


def read_api():
    request = urllib.request.Request(SOURCE, headers={'User-Agent': 'wpe-gpui-fast-runtime-updater/1'})
    with urllib.request.urlopen(request, timeout=30) as response:
        if response.geturl() != SOURCE:
            raise ValueError('Unexpected package metadata redirect')
        raw = response.read(1024 * 1024 + 1)
    if len(raw) > 1024 * 1024:
        raise ValueError('Package metadata exceeds size limit')
    return json.loads(raw)


def package_digest(url):
    request = urllib.request.Request(url, headers={'User-Agent': 'wpe-gpui-fast-runtime-updater/1'})
    with urllib.request.urlopen(request, timeout=60) as response:
        if response.geturl() != url:
            raise ValueError('Unexpected package archive redirect')
        digest, total = hashlib.sha256(), 0
        while chunk := response.read(1024 * 1024):
            total += len(chunk)
            if total > MAX_PACKAGE_BYTES:
                raise ValueError('Package exceeds download size limit')
            digest.update(chunk)
    if not total:
        raise ValueError('Empty package archive')
    return digest.hexdigest()


def candidate_metadata(current, api, digest=package_digest):
    validate_metadata(current)
    if not isinstance(api, dict) or any(api.get(k) != v for k, v in {
        'pkgname': 'wpewebkit', 'repo': 'extra', 'arch': 'x86_64'}.items()):
        raise ValueError('Unexpected package identity from Arch metadata')
    epoch, version, release = api.get('epoch', 0), api.get('pkgver'), api.get('pkgrel')
    candidate_key = version_key(epoch, version, release)
    previous_key = version_key(current['epoch'], current['version'], current['release'])
    if candidate_key < previous_key:
        raise ValueError('Refusing an automatic native runtime downgrade')
    if candidate_key == previous_key:
        return current
    if tuple(map(int, version.split('.'))) < (2, 54, 0):
        raise ValueError('WPEPlatform 2.54+ is required')
    url = archive_url(version, release)
    if api.get('filename') != url.rsplit('/', 1)[1]:
        raise ValueError('Unexpected package filename')
    proposed = dict(current, epoch=epoch, version=version, release=release,
                    archive_url=url, sha256=digest(url))
    return validate_metadata(proposed)


def update(path=METADATA, api_reader=read_api, digest=package_digest, check=False):
    path = Path(path)
    current = load_metadata(path)
    proposed = candidate_metadata(current, api_reader(), digest)
    if proposed == current:
        return False
    if not check:
        # Write only after metadata and download validation have succeeded.
        temporary = path.with_name(path.name + '.tmp')
        temporary.write_text(json.dumps(proposed, indent=2) + '\n')
        temporary.replace(path)
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='Report availability without changing metadata')
    parser.add_argument('--print-install-info', action='store_true', help='Validate and print URL, checksum, version; no network')
    args = parser.parse_args()
    try:
        if args.print_install_info:
            metadata = load_metadata()
            print(metadata['archive_url'])
            print(metadata['sha256'])
            print(f"{metadata['version']}-{metadata['release']}")
            return 0
        changed = update(check=args.check)
        print('Native WPE update available' if changed else 'Pinned native WPE package is current in Arch')
        return 0
    except (ValueError, KeyError, OSError) as error:
        print(f'Native WPE update check failed: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
