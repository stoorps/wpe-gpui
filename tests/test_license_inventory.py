import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('license_inventory', Path(__file__).resolve().parents[1] / 'scripts/license-inventory.py')
inventory = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inventory)


def package(root, name, version='1.0.0', source=None, license='MIT OR Apache-2.0'):
    directory = root / name
    directory.mkdir()
    (directory / 'Cargo.toml').write_text('[package]\n')
    return {'id': name + '-' + version, 'name': name, 'version': version,
            'manifest_path': str(directory / 'Cargo.toml'), 'source': source,
            'license': license, 'repository': 'https://example.test/' + name,
            'license_file': None}


def metadata(root, packages, edges):
    return {'workspace_root': str(root), 'workspace_members': [packages[0]['id']],
            'packages': packages, 'resolve': {'nodes': [
                {'id': p['id'], 'features': ['one'],
                 'deps': [{'pkg': packages[i]['id']} for i in edges.get(n, [])]}
                for n, p in enumerate(packages)]}}


def report(data, requested=None):
    return inventory.make_inventory(data, target='x86_64-unknown-linux-gnu',
        all_features=False, no_default_features=False, features=[],
        requested=requested or [], lock_bytes=b'locked source graph')


class LicenseInventoryTests(unittest.TestCase):
    def test_selects_reachable_nodes_and_preserves_license_expression(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            a, b, unused = [package(root, name) for name in ['host', 'dependency', 'unrelated']]
            b['license'] = '(Apache-2.0 OR MIT) AND BSD-3-Clause'
            data = metadata(root, [a, b, unused], {0: [1]})
            result, _ = report(data)
            self.assertEqual(result['package_count'], 2)
            self.assertNotIn('unrelated', [p['name'] for p in result['packages']])
            found = next(p for p in result['packages'] if p['name'] == 'dependency')
            self.assertEqual(found['license_expression'], b['license'])
            self.assertEqual(found['enabled_features'], ['one'])
            self.assertEqual(result['cargo_lock_sha256'], inventory.digest(b'locked source graph'))
            self.assertFalse(result['native_components_included'])

    def test_unknown_root_and_incomplete_graph_fail(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            p = package(root, 'host')
            data = metadata(root, [p], {})
            with self.assertRaises(ValueError):
                report(data, ['not-a-member'])
            data['resolve']['nodes'][0]['deps'] = [{'pkg': 'absent'}]
            with self.assertRaises(ValueError):
                report(data)

    def test_contained_custom_license_is_collected_but_escape_is_not(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            p = package(root, 'host')
            directory = Path(p['manifest_path']).parent
            (directory / 'legal').mkdir()
            (directory / 'legal/terms.txt').write_text('specific terms')
            (directory / 'LICENSE-MIT').write_text('mit')
            (root / 'outside').write_text('must not be copied')
            (directory / 'NOTICE-link').symlink_to(root / 'outside')
            p['license_file'] = 'legal/terms.txt'
            files, issues = inventory.notice_files(p)
            self.assertEqual([name for name, _ in files], ['LICENSE-MIT', 'legal/terms.txt'])
            self.assertTrue(any('escapes' in issue for issue in issues))
            p['license_file'] = '../outside'
            _, issues = inventory.notice_files(p)
            self.assertTrue(any('escapes' in issue for issue in issues))

    def test_missing_metadata_is_flagged_not_inferred(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            p = package(root, 'host', license=None)
            result, _ = report(metadata(root, [p], {}))
            self.assertIsNone(result['packages'][0]['license_expression'])
            self.assertIn('No license expression or license_file declared', result['packages'][0]['review_issues'])
            self.assertTrue(any('not a finding' in s for s in result['limitations']))

    def test_different_sources_get_distinct_collection_directories(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            a, b = package(root, 'host'), package(root, 'other')
            b['name'], b['version'] = a['name'], a['version']
            b['source'] = 'registry+https://example.test/index'
            for p in [a, b]:
                (Path(p['manifest_path']).parent / 'LICENSE').write_text(p['id'])
            result, copies = report(metadata(root, [a, b], {0: [1]}))
            destinations = [name for name, _ in copies if name.startswith('packages/')]
            self.assertEqual(len(destinations), 2)
            self.assertEqual(len(set(destinations)), 2)
            self.assertEqual(result, report(metadata(root, [a, b], {0: [1]}))[0])

    def test_collection_replaces_only_owned_unmodified_files(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            p = package(root, 'host')
            (Path(p['manifest_path']).parent / 'LICENSE').write_text('license')
            result, copies = report(metadata(root, [p], {}))
            dest = root / 'collected'
            inventory.collect_notices(dest, result, copies)
            inventory.collect_notices(dest, result, copies)
            manifest = json.loads((dest / inventory.MANIFEST).read_text())
            self.assertEqual(manifest['generator'], inventory.TOOL)
            self.assertEqual(len(manifest['package_provenance']), 1)
            (dest / 'do-not-delete').write_text('user data')
            with self.assertRaises(ValueError):
                inventory.collect_notices(dest, result, copies)
            self.assertEqual((dest / 'do-not-delete').read_text(), 'user data')
            (dest / 'do-not-delete').unlink()
            notice = dest / copies[0][0]
            notice.write_text('modified by user')
            with self.assertRaises(ValueError):
                inventory.collect_notices(dest, result, copies)
            self.assertEqual(notice.read_text(), 'modified by user')

    def test_unowned_directory_and_symlink_are_never_replaced(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            p = package(root, 'host')
            result, copies = report(metadata(root, [p], {}))
            dest = root / 'existing'
            dest.mkdir()
            (dest / 'important').write_text('keep')
            with self.assertRaises(ValueError):
                inventory.collect_notices(dest, result, copies)
            link = root / 'link'
            link.symlink_to(dest, target_is_directory=True)
            with self.assertRaises(ValueError):
                inventory.collect_notices(link, result, copies)
            self.assertEqual((dest / 'important').read_text(), 'keep')

    def test_markdown_does_not_turn_metadata_into_new_rows(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            p = package(root, 'host', license='MIT | custom\n<script>')
            result, _ = report(metadata(root, [p], {}))
            text = inventory.markdown(result)
            self.assertIn('MIT \\| custom &lt;script&gt;', text)
            self.assertNotIn('<script>', text)


if __name__ == '__main__':
    unittest.main()
