import importlib.util
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('installer', Path(__file__).with_name('install.py'))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class InstallerTest(unittest.TestCase):
    def test_install_upgrade_and_uninstall_preserve_settings(self):
        with tempfile.TemporaryDirectory(prefix='spectra install ') as tmp:
            base = Path(tmp)
            prefix, config = base / 'local', base / 'config'
            saved = config / 'graphic-eq/gpu-settings.json'
            saved.parent.mkdir(parents=True)
            saved.write_text('{"eq_on": false}')
            binary = base / 'binary'
            binary.write_text('#!/bin/sh\nprintf "test-version"\n')
            installer.install(prefix, config, binary, True)
            launcher = prefix / 'bin/spectraforgeeq'
            self.assertEqual(subprocess.check_output([str(launcher)], text=True), 'test-version')
            auto = config / 'autostart/spectraforgeeq.desktop'
            self.assertIn('--background', auto.read_text())
            if shutil.which('desktop-file-validate'):
                subprocess.run(['desktop-file-validate', str(prefix / 'share/applications/spectraforgeeq.desktop'), str(auto)], check=True)
            installer.install(prefix, config, binary, False)
            self.assertFalse(auto.exists())
            installer.uninstall(prefix, config)
            self.assertFalse(launcher.exists())
            self.assertFalse((prefix / 'lib/spectraforgeeq/spectraforgeeq').exists())
            self.assertEqual(saved.read_text(), '{"eq_on": false}')

    def test_launcher_handles_shell_characters(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            prefix = base / 'space $dollar `literal` "quote" %percent'
            binary = base / 'binary'
            binary.write_text('#!/bin/sh\nprintf "%s" "$1"\n')
            installer.install(prefix, base / 'config', binary, True)
            self.assertEqual(subprocess.check_output([str(prefix / 'bin/spectraforgeeq'), 'hello'], text=True), 'hello')
            if shutil.which('desktop-file-validate'):
                subprocess.run(['desktop-file-validate', str(prefix / 'share/applications/spectraforgeeq.desktop')], check=True)


if __name__ == '__main__':
    unittest.main()
