import importlib.util
import json
import hashlib
import io
import os
import shutil
import subprocess
import sys
import tarfile
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('native_runtime_update', ROOT / 'scripts/update-native-runtime.py')
updater = importlib.util.module_from_spec(spec)
spec.loader.exec_module(updater)


class NativeRuntimeUpdateTests(unittest.TestCase):
    def setUp(self):
        self.current = updater.load_metadata()
        self.api = {'pkgname': 'wpewebkit', 'repo': 'extra', 'arch': 'x86_64',
                    'epoch': 0, 'pkgver': '2.54.3', 'pkgrel': '1',
                    'filename': 'wpewebkit-2.54.3-1-x86_64.pkg.tar.zst'}
        # Fixed fixtures remain independent of the repository's future pin.
        self.current.update(epoch=0, version='2.54.2', release='1',
                            archive_url=updater.archive_url('2.54.2', '1'), sha256='a' * 64)

    def test_patch_update_hashes_official_constructed_url(self):
        digest = Mock(return_value='b' * 64)
        result = updater.candidate_metadata(self.current, self.api, digest)
        self.assertEqual(result['version'], '2.54.3')
        self.assertEqual(result['sha256'], 'b' * 64)
        digest.assert_called_once_with(updater.archive_url('2.54.3', '1'))

    def test_current_package_is_idempotent_and_does_not_download(self):
        self.api.update(pkgver='2.54.2', filename='wpewebkit-2.54.2-1-x86_64.pkg.tar.zst')
        digest = Mock(side_effect=AssertionError('Unexpected download'))
        self.assertEqual(updater.candidate_metadata(self.current, self.api, digest), self.current)

    def test_package_release_and_numeric_version_order(self):
        self.api.update(pkgver='2.54.2', pkgrel='2', filename='wpewebkit-2.54.2-2-x86_64.pkg.tar.zst')
        self.assertEqual(updater.candidate_metadata(self.current, self.api, lambda _: 'c' * 64)['release'], '2')
        self.assertGreater(updater.version_key(0, '2.54.10', '1'), updater.version_key(0, '2.54.9', '1'))

    def test_downgrade_or_prerelease_is_rejected(self):
        for version in ['2.54.1', '2.55.1-beta', '../other', 'not-a-version']:
            with self.subTest(version=version):
                self.api['pkgver'] = version
                with self.assertRaises(ValueError):
                    updater.candidate_metadata(self.current, self.api)

    def test_remote_identity_filename_and_checksum_are_validated(self):
        for key, value in [('repo', 'testing'), ('arch', 'aarch64'), ('pkgname', 'other'), ('filename', '../../payload')]:
            api = dict(self.api, **{key: value})
            with self.subTest(key=key), self.assertRaises(ValueError):
                updater.candidate_metadata(self.current, api, lambda _: 'b' * 64)
        with self.assertRaises(ValueError):
            updater.candidate_metadata(self.current, self.api, lambda _: 'invalid')

    def test_untrusted_current_url_is_rejected(self):
        current = dict(self.current, archive_url='https://example.test/payload')
        with self.assertRaises(ValueError):
            updater.validate_metadata(current)

    def test_check_and_failure_leave_file_unchanged_then_update_is_atomic(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'native-runtime.json'
            original = json.dumps(self.current, indent=2) + '\n'
            path.write_text(original)
            self.assertTrue(updater.update(path, lambda: self.api, lambda _: 'b' * 64, check=True))
            self.assertEqual(path.read_text(), original)
            with self.assertRaises(OSError):
                updater.update(path, lambda: self.api, Mock(side_effect=OSError('offline')))
            self.assertEqual(path.read_text(), original)
            self.assertTrue(updater.update(path, lambda: self.api, lambda _: 'b' * 64))
            self.assertEqual(json.loads(path.read_text())['version'], '2.54.3')
            self.assertFalse(path.with_suffix('.json.tmp').exists())


class LocalInstallerTests(unittest.TestCase):
    """Exercise replacement/failure behavior with local archives and fake transport.

    Signature cryptography remains pacman-key's responsibility; the fake command
    lets this suite prove that verification is required before replacement.
    """
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / 'scripts').mkdir()
        for filename in ['setup-local-arch.sh', 'update-native-runtime.py']:
            shutil.copyfile(ROOT / 'scripts' / filename, self.root / 'scripts' / filename)
        self.archive = self.root / 'fixture.pkg.tar.zst'
        with tarfile.open(self.archive, 'w') as tar:
            for filename, content in {
                'usr/lib/wpe-webkit-2.0/WPEWebProcess': b'fixture process; never executed',
                'usr/lib/pkgconfig/wpe-webkit-2.0.pc': b'prefix=/usr\nlibdir=/usr/lib\n'
            }.items():
                info = tarfile.TarInfo(filename)
                info.size = len(content)
                tar.addfile(info, io.BytesIO(content))
        metadata = updater.load_metadata()
        metadata['sha256'] = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        (self.root / 'native-runtime.json').write_text(json.dumps(metadata))
        old = self.root / '.deps/usr'
        old.mkdir(parents=True)
        (old / 'old-runtime-marker').write_text('old')
        fake_bin = self.root / 'bin'
        fake_bin.mkdir()
        fake_curl = fake_bin / 'curl'
        fake_curl.write_text(f'#!{sys.executable}\n' + """import os,pathlib,shutil,sys
output=sys.argv[sys.argv.index('--output')+1]
if output.endswith('.sig'): pathlib.Path(output).write_text('fixture signature')
else: shutil.copyfile(os.environ['WPE_TEST_ARCHIVE'],output)
""")
        fake_key = fake_bin / 'pacman-key'
        fake_key.write_text(f'#!{sys.executable}\n' + """import os,sys
sys.exit(1 if os.environ.get('WPE_TEST_BAD_SIGNATURE') else 0)
""")
        fake_curl.chmod(0o755)
        fake_key.chmod(0o755)
        self.env = dict(os.environ, PATH=str(fake_bin)+os.pathsep+os.environ['PATH'],
                        WPE_TEST_ARCHIVE=str(self.archive))

    def install(self):
        return subprocess.run(['bash', 'scripts/setup-local-arch.sh'], cwd=self.root,
                              env=self.env, text=True, capture_output=True)

    def test_verified_package_replaces_old_files_and_relocates_metadata(self):
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.root / '.deps/usr/old-runtime-marker').exists())
        self.assertTrue((self.root / '.deps/usr/lib/wpe-webkit-2.0/WPEWebProcess').exists())
        pc = (self.root / '.deps/usr/lib/pkgconfig/wpe-webkit-2.0.pc').read_text()
        self.assertIn(f'prefix={self.root}/.deps/usr', pc)
        self.assertTrue((self.root / '.deps/native-runtime-installed.json').exists())
        self.assertEqual(list((self.root / '.deps').glob('.wpe-extract.*')), [])

    def test_bad_signature_keeps_installed_runtime(self):
        self.env['WPE_TEST_BAD_SIGNATURE'] = '1'
        self.assertNotEqual(self.install().returncode, 0)
        self.assertTrue((self.root / '.deps/usr/old-runtime-marker').exists())
        self.assertFalse((self.root / '.deps/native-runtime-installed.json').exists())

    def test_bad_checksum_keeps_installed_runtime(self):
        self.archive.write_bytes(b'corrupt archive')
        self.assertNotEqual(self.install().returncode, 0)
        self.assertTrue((self.root / '.deps/usr/old-runtime-marker').exists())


if __name__ == '__main__':
    unittest.main()
