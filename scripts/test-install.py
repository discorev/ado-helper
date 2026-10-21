#!/usr/bin/env python3
"""Exercise installer signing selection and atomic replacement with mock tools."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'scripts').mkdir()
        shutil.copyfile(Path(__file__).with_name('install.sh'), self.root / 'scripts/install.sh')
        (self.root / 'target/release').mkdir(parents=True)
        (self.root / 'target/release/ado').write_text('new helper')
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.destination = self.root / 'installed'
        self.destination.mkdir()
        (self.destination / 'ado').write_text('old helper')
        self.log = self.root / 'calls'
        self.env = {**os.environ, 'PATH': f'{self.bin}:/usr/bin:/bin',
                    'ADO_INSTALL_DIR': str(self.destination), 'TEST_LOG': str(self.log),
                    'TEST_PLATFORM': 'Darwin', 'TEST_IDENTITIES': 'one', 'TEST_SIGN_FAIL': '0',
                    'TEST_VERIFY_FAIL': '0'}
        self.env.pop('ADO_CODESIGN_IDENTITY', None)
        self.mock('uname', 'printf "%s\\n" "$TEST_PLATFORM"')
        self.mock('cargo', 'printf "cargo\\n" >> "$TEST_LOG"')
        self.mock('security', '''printf 'security\\n' >> "$TEST_LOG"
case "$TEST_IDENTITIES" in
one) printf '  1) ABCDEF "Apple Development: Test (TEAM)"\\n     1 valid identities found\\n' ;;
none) printf '     0 valid identities found\\n' ;;
many) printf '  1) ABCDEF "First"\\n  2) FEDCBA "Second"\\n     2 valid identities found\\n' ;;
esac''')
        self.mock('codesign', '''printf 'codesign %s\\n' "$*" >> "$TEST_LOG"
if [ "$1" = '--force' ] && [ "$TEST_SIGN_FAIL" = 1 ]; then exit 1; fi
if [ "$1" = '--verify' ] && [ "$TEST_VERIFY_FAIL" = 1 ]; then exit 1; fi''')

    def mock(self, name, body):
        path = self.bin / name
        path.write_text('#!/bin/sh\nset -eu\n' + body + '\n')
        path.chmod(0o755)

    def run_install(self, success=True):
        result = subprocess.run(['/bin/sh', str(self.root / 'scripts/install.sh')], env=self.env,
                                capture_output=True, text=True)
        self.assertEqual(result.returncode == 0, success, result.stderr)
        self.assertEqual((self.destination / 'ado').read_text(), 'new helper' if success else 'old helper')
        self.assertEqual(list(self.destination.glob('.ado.install.*')), [])
        return self.log.read_text() if self.log.exists() else ''

    def test_selects_sole_identity_and_verifies_before_install(self):
        log = self.run_install()
        self.assertIn('--sign ABCDEF --identifier dev.ollies.ado-helper', log)
        self.assertIn('codesign --verify --strict', log)

    def test_missing_or_ambiguous_identity_leaves_installation_untouched(self):
        for identities in ('none', 'many'):
            with self.subTest(identities=identities):
                self.env['TEST_IDENTITIES'] = identities
                log = self.run_install(success=False)
                self.assertNotIn('cargo', log)
                self.assertNotIn('codesign', log)

    def test_explicit_certificate_overrides_discovery(self):
        self.env['ADO_CODESIGN_IDENTITY'] = 'Apple Development: Test (TEAM)'
        log = self.run_install()
        self.assertNotIn('security', log)
        self.assertIn('--sign Apple Development: Test (TEAM) --identifier', log)

    def test_ad_hoc_requires_explicit_opt_in(self):
        self.env['ADO_CODESIGN_IDENTITY'] = '-'
        log = self.run_install()
        self.assertNotIn('security', log)
        self.assertIn('--sign - --identifier', log)

    def test_signing_failure_keeps_existing_binary(self):
        self.env['TEST_SIGN_FAIL'] = '1'
        self.run_install(success=False)

    def test_verification_failure_keeps_existing_binary(self):
        self.env['TEST_VERIFY_FAIL'] = '1'
        self.run_install(success=False)

    def test_linux_does_not_use_macos_signing_tools(self):
        self.env['TEST_PLATFORM'] = 'Linux'
        log = self.run_install()
        self.assertNotIn('security', log)
        self.assertNotIn('codesign', log)


if __name__ == '__main__':
    unittest.main()
