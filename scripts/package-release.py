#!/usr/bin/env python3
"""Package only the binary, trust store, documentation and redistribution notices."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--target', required=True)
    args = parser.parse_args()
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['package']['version']
    windows = 'windows' in args.target
    binary = 'gdirect.exe' if windows else 'gdirect'
    executable = ROOT / 'target' / args.target / 'release' / binary
    if not executable.exists():
        raise SystemExit(f'Missing binary: {executable}')
    name = f'gdirect-{version}-{args.target}'
    dist = ROOT / 'dist'
    dist.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory() as temporary:
        folder = Path(temporary) / name
        folder.mkdir()
        shutil.copy2(executable, folder / binary)
        for document in ['README.md', 'README.zh-CN.md', 'LICENSE']:
            shutil.copy2(ROOT / document, folder / document)
        for url, destination in [
            ('https://curl.se/ca/cacert.pem', 'cacert.pem'),
            ('https://raw.githubusercontent.com/openssl/openssl/openssl-3.0.0/LICENSE.txt', 'OPENSSL-LICENSE.txt'),
            ('https://www.mozilla.org/media/MPL/2.0/index.txt', 'CA-LICENSE.txt'),
        ]:
            with urllib.request.urlopen(url, timeout=60) as response:
                (folder / destination).write_bytes(response.read())
        info = {'version': version, 'target': args.target, 'aria2': '1.37.0',
                'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip()}
        (folder / 'BUILD-INFO.json').write_text(json.dumps(info, indent=2) + '\n')
        if windows:
            archive = dist / f'{name}.zip'
            with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as output:
                for file in sorted(folder.iterdir()):
                    output.write(file, f'{name}/{file.name}')
        else:
            archive = dist / f'{name}.tar.gz'
            with tarfile.open(archive, 'w:gz') as output:
                output.add(folder, arcname=name)
        # Smoke test the packaged layout, including the adjacent trust store.
        output = subprocess.check_output([str(folder / binary), '--version'], text=True).strip()
        assert output == f'gdirect {version}', output
    digest = hashlib.file_digest(archive.open('rb'), 'sha256').hexdigest()
    (dist / f'{archive.name}.sha256').write_text(f'{digest}  {archive.name}\n')
    print(archive.name)


if __name__ == '__main__':
    main()
