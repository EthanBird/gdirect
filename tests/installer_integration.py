#!/usr/bin/env python3
"""Install the actual package into a path with spaces, then reject a bad checksum."""
import argparse
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib

root = Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser()
p.add_argument('--target', required=True)
p.add_argument('--with-mount', action='store_true')
args = p.parse_args()
version = tomllib.loads((root/'Cargo.toml').read_text())['package']['version']
name = f'gdirect-{version}-{args.target}.tar.gz'
with tempfile.TemporaryDirectory(prefix='gdirect installer ') as temporary:
    temp = Path(temporary)
    artifacts = temp/'artifacts'
    artifacts.mkdir()
    shutil.copyfile(root/'dist'/name, artifacts/name)
    shutil.copyfile(root/'dist'/f'{name}.sha256', artifacts/'SHA256SUMS')
    prefix = temp/'install with spaces'
    command = ['bash', str(root/'scripts/install.sh'), '--version', version,
               '--from-dir', str(artifacts), '--prefix', str(prefix)]
    if args.with_mount:
        command.append('--with-mount')
    subprocess.run(command, check=True)
    assert subprocess.check_output([prefix/'bin/gdirect', '--version'], text=True).strip() == f'gdirect {version}'
    assert 'client-secret' in subprocess.check_output([prefix/'bin/gdirect', 'login', '--help'], text=True)
    assert (prefix/f'libexec/gdirect/{version}/cacert.pem').is_file()
    assert (prefix/f'libexec/gdirect/{version}/docs/configuration/README.md').is_file()
    if args.with_mount:
        rclone = prefix/'bin/rclone'
        subprocess.run([str(rclone) if rclone.exists() else 'rclone', 'version'], check=True)
    # No executable should be installed when an archive fails verification.
    (artifacts/'SHA256SUMS').write_text('0'*64 + f'  {name}\n')
    bad_prefix = temp/'rejected'
    result = subprocess.run(['bash', str(root/'scripts/install.sh'), '--version', version,
        '--from-dir', str(artifacts), '--prefix', str(bad_prefix)], capture_output=True, text=True)
    assert result.returncode != 0 and 'Checksum mismatch' in result.stderr
    assert not (bad_prefix/'bin/gdirect').exists()
print('Installer package, paths, bundled CA/docs and checksum rejection passed')
