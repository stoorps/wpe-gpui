#!/usr/bin/env python3
"""Inventory declared Cargo licenses and optionally collect package notice files.

Uses the locked, target-filtered Cargo resolve graph. This is evidence gathering,
not a license-compatibility decision or a complete native/binary bill of materials.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
TOOL = 'wpe-gpui-fast/license-inventory'
MANIFEST = 'inventory-manifest.json'
MAX_NOTICE_BYTES = 16 * 1024 * 1024
LIMITATIONS = [
    'License expressions are package declarations; no legal obligations or license compatibility are inferred.',
    'The selected Cargo dependency graph includes build/dev dependencies; it is not the exact final linker/file inventory.',
    'System libraries, WPE/WebKit binaries, native bundled components, fonts, assets and their source obligations are not inventoried here.',
    'Package-root notices and an explicitly declared license_file may omit notices elsewhere in a package or upstream source.',
    'Missing metadata/files are review issues, not a finding that a package has no license or obligations.',
]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def stable_source(package, workspace):
    if package.get('source'):
        return str(package['source'])
    directory = Path(package['manifest_path']).resolve().parent
    try:
        return 'path:' + directory.relative_to(workspace).as_posix()
    except ValueError:
        return 'path:' + str(directory)


def selected_packages(metadata, requested):
    packages = {p['id']: p for p in metadata.get('packages', [])}
    resolve = metadata.get('resolve')
    if not isinstance(resolve, dict):
        raise ValueError('Cargo metadata has no resolve graph; do not use --no-deps')
    nodes = {n['id']: n for n in resolve.get('nodes', [])}
    roots = list(metadata.get('workspace_members', []))
    if requested:
        names = set(requested)
        found = {packages[p]['name']: p for p in roots if p in packages}
        missing = names - found.keys()
        if missing:
            raise ValueError('Unknown workspace package(s): ' + ', '.join(sorted(missing)))
        roots = [found[name] for name in sorted(names)]
    pending, seen = list(roots), set()
    while pending:
        package_id = pending.pop()
        if package_id in seen:
            continue
        if package_id not in packages or package_id not in nodes:
            raise ValueError(f'Incomplete Cargo resolve graph for {package_id}')
        seen.add(package_id)
        pending.extend(d['pkg'] for d in nodes[package_id].get('deps', []))
    return [packages[p] for p in seen], nodes, roots


def notice_files(package):
    """Return contained, regular package-root/declared files; never follow escapes."""
    root = Path(package['manifest_path']).resolve().parent
    candidates = [p for p in root.iterdir()
                  if re.match(r'^(LICEN[CS]E|COPYING|NOTICE)(?:$|[._-])', p.name, re.I)]
    declared = package.get('license_file')
    if declared:
        candidates.append(root / declared)
    files, issues = {}, []
    for candidate in candidates:
        try:
            path = candidate.resolve(strict=True)
            relative = path.relative_to(root)
            if not path.is_file():
                continue
            if path.stat().st_size > MAX_NOTICE_BYTES:
                issues.append(f'Notice exceeds {MAX_NOTICE_BYTES} bytes: {relative.as_posix()}')
                continue
            files[relative.as_posix()] = path
        except ValueError:
            issues.append('Notice path escapes package root: ' + str(candidate.name))
        except OSError:
            issues.append('Notice file is unavailable: ' + str(candidate.name))
    if not files:
        issues.append('No contained package-root/declared notice files found')
    return [(name, files[name]) for name in sorted(files)], sorted(set(issues))


def make_inventory(metadata, *, target, all_features, no_default_features, features,
                   requested, lock_bytes):
    workspace = Path(metadata['workspace_root']).resolve()
    chosen, nodes, roots = selected_packages(metadata, requested)
    members = set(metadata.get('workspace_members', []))
    entries, copies = [], []
    for package in chosen:
        source = stable_source(package, workspace)
        identity = f"{package['name']}@{package['version']}#{source}"
        folder = re.sub(r'[^A-Za-z0-9._-]', '_', package['name'] + '-' + package['version'])
        folder += '-' + digest(identity.encode())[:12]
        files, issues = notice_files(package)
        expression = package.get('license')
        if not expression and not package.get('license_file'):
            issues.append('No license expression or license_file declared')
        notices = []
        for relative, path in files:
            content = path.read_bytes()
            destination = (Path('packages') / folder / relative).as_posix()
            notices.append({'package_relative_path': relative, 'collection_path': destination,
                            'sha256': digest(content), 'bytes': len(content)})
            copies.append((destination, content))
        entries.append({
            'name': package['name'], 'version': package['version'], 'identity': identity,
            'source': source, 'repository': package.get('repository'),
            'license_expression': expression, 'declared_license_file': package.get('license_file'),
            'workspace_member': package['id'] in members,
            'enabled_features': sorted(nodes[package['id']].get('features', [])),
            'notices': notices, 'review_issues': sorted(set(issues)),
        })
    entries.sort(key=lambda p: (p['name'], p['version'], p['source']))
    # Workspace source notices are collected separately, not inferred to replace
    # absent license files from a third-party package.
    project_files, project_issues = notice_files({'manifest_path': str(workspace / 'Cargo.toml')})
    project_notices = []
    for relative, path in project_files:
        content = path.read_bytes()
        destination = (Path('workspace') / relative).as_posix()
        project_notices.append({'workspace_relative_path': relative, 'collection_path': destination,
                                'sha256': digest(content), 'bytes': len(content)})
        copies.append((destination, content))
    root_names = sorted(f"{p['name']}@{p['version']}" for p in chosen if p['id'] in roots)
    report = {
        'schema_version': 1, 'generator': TOOL, 'target': target,
        'cargo_metadata_flags': {'locked': True, 'all_features': all_features,
                                 'no_default_features': no_default_features,
                                 'requested_features': features},
        'roots': root_names, 'cargo_lock_sha256': digest(lock_bytes),
        'package_count': len(entries), 'limitations': LIMITATIONS,
        'native_components_included': False,
        'workspace_notices': project_notices, 'workspace_review_issues': project_issues,
        'packages': entries,
    }
    return report, copies


def cell(value):
    value = '(not declared)' if value is None else str(value)
    return value.replace('\\', '\\\\').replace('|', '\\|').replace('\n', ' ').replace('\r', ' ').replace('<', '&lt;').replace('>', '&gt;')


def markdown(report):
    lines = ['# Cargo license inventory', '',
             f"Target: `{report['target']}`. Packages: {report['package_count']}.",
             f"Cargo.lock SHA256: `{report['cargo_lock_sha256']}`.", '',
             '**Declared metadata and discovered files only; not a legal approval or native/binary bill of materials.**', '',
             '| Package | Version | Declared license | Source | Review |',
             '| --- | --- | --- | --- | --- |']
    for p in report['packages']:
        review = '; '.join(p['review_issues']) or 'Metadata/files found; review still required'
        lines.append('| ' + ' | '.join(map(cell, [p['name'], p['version'],
                     p['license_expression'], p['source'], review])) + ' |')
    lines += ['', '## Collection scope', '']
    lines += ['- ' + limitation for limitation in report['limitations']]
    lines += ['', 'The JSON report records package-relative notice paths, SHA256 hashes, selected features,',
              'and document provenance. A collected directory contains only files listed in its manifest.',
              'Check review issues and supply any additional files required for the actual release.', '']
    return '\n'.join(lines)


def write_atomic(path, content):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix='.' + path.name, delete=False) as out:
        staging = Path(out.name)
        out.write(content)
    try:
        os.replace(staging, path)
    finally:
        staging.unlink(missing_ok=True)


def collect_notices(destination, report, copies):
    """Replace only a previously owned, unchanged generated directory."""
    destination = Path(destination).absolute()
    if destination.is_symlink():
        raise ValueError('Refusing to replace a symlinked notice directory')
    destination = destination.resolve()
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        marker = destination / MANIFEST
        if destination.is_symlink() or not marker.is_file():
            raise ValueError('Refusing to replace a notice directory without our ownership manifest')
        previous = json.loads(marker.read_text())
        if previous.get('generator') != TOOL or previous.get('schema_version') != 1:
            raise ValueError('Unrecognized notice-directory ownership manifest')
        files = previous.get('files')
        if not isinstance(files, dict) or any(
            not isinstance(name, str) or not isinstance(checksum, str)
            or not re.fullmatch(r'[0-9a-f]{64}', checksum)
            or Path(name).is_absolute() or '..' in Path(name).parts or name == MANIFEST
            for name, checksum in files.items()
        ):
            raise ValueError('Invalid notice-directory file manifest')
        expected = {MANIFEST, *files}
        actual = {p.relative_to(destination).as_posix() for p in destination.rglob('*') if p.is_file() or p.is_symlink()}
        if expected != actual:
            raise ValueError('Refusing to replace notice directory containing unexpected/missing files')
        for name, checksum in previous['files'].items():
            path = destination / name
            if path.is_symlink() or not path.resolve().is_relative_to(destination) or digest(path.read_bytes()) != checksum:
                raise ValueError('Refusing to replace modified/unsafe collected notice: ' + name)
    stage = Path(tempfile.mkdtemp(dir=destination.parent, prefix='.' + destination.name + '-'))
    backup = None
    try:
        hashes = {}
        for name, content in copies:
            path = stage / name
            if not path.resolve().is_relative_to(stage):
                raise ValueError('Unsafe collection path')
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
            hashes[name] = digest(content)
        (stage / MANIFEST).write_text(json.dumps({
            'schema_version': 1, 'generator': TOOL, 'target': report['target'],
            'cargo_lock_sha256': report['cargo_lock_sha256'], 'files': hashes,
            'cargo_metadata_flags': report['cargo_metadata_flags'], 'roots': report['roots'],
            'package_provenance': report['packages'],
            'workspace_notices': report['workspace_notices'], 'limitations': LIMITATIONS,
        }, indent=2, sort_keys=True) + '\n')
        if destination.exists():
            backup = Path(tempfile.mkdtemp(dir=destination.parent, prefix='.' + destination.name + '-old-'))
            backup.rmdir()
            destination.rename(backup)
        try:
            stage.rename(destination)
        except BaseException:
            if backup:
                backup.rename(destination)
                backup = None
            raise
    finally:
        if stage.exists():
            shutil.rmtree(stage)
        if backup and backup.exists():
            shutil.rmtree(backup)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', default='x86_64-unknown-linux-gnu')
    parser.add_argument('--package', action='append', default=[], help='Workspace package root (repeatable; default: whole workspace)')
    parser.add_argument('--all-features', action='store_true')
    parser.add_argument('--no-default-features', action='store_true')
    parser.add_argument('--features', action='append', default=[])
    parser.add_argument('--offline', action='store_true')
    parser.add_argument('--output', type=Path, default=ROOT / 'target/license-inventory.json')
    parser.add_argument('--markdown', type=Path, default=ROOT / 'target/license-inventory.md')
    parser.add_argument('--collect', action='store_true')
    parser.add_argument('--notices-dir', type=Path, default=ROOT / 'target/third-party-notices')
    args = parser.parse_args(argv)
    command = ['cargo', 'metadata', '--format-version', '1', '--locked', '--filter-platform', args.target]
    if args.all_features:
        command.append('--all-features')
    if args.no_default_features:
        command.append('--no-default-features')
    for features in args.features:
        command += ['--features', features]
    if args.offline:
        command.append('--offline')
    try:
        result = subprocess.run(command, cwd=ROOT, check=True, capture_output=True, text=True)
        metadata = json.loads(result.stdout)
        workspace = Path(metadata['workspace_root'])
        report, copies = make_inventory(metadata, target=args.target,
            all_features=args.all_features, no_default_features=args.no_default_features,
            features=args.features, requested=args.package,
            lock_bytes=(workspace / 'Cargo.lock').read_bytes())
        if args.collect:
            collect_notices(args.notices_dir, report, copies)
            report['notice_collection'] = str(args.notices_dir)
        write_atomic(args.output, (json.dumps(report, indent=2, sort_keys=True) + '\n').encode())
        write_atomic(args.markdown, markdown(report).encode())
        issues = sum(bool(p['review_issues']) for p in report['packages'])
        print(f"Inventoried {report['package_count']} Cargo packages; {issues} packages have review issues.", file=sys.stderr)
        print(f'JSON: {args.output}\nMarkdown: {args.markdown}', file=sys.stderr)
        if args.collect:
            print(f'Collected notices: {args.notices_dir}', file=sys.stderr)
        return 0
    except subprocess.CalledProcessError as error:
        print(error.stderr or str(error), file=sys.stderr)
        return 1
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f'license inventory: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
