"""Back up a stopped BDM catalog to a private ModelScope dataset, or restore it."""

from __future__ import annotations

import argparse
from contextlib import closing
from datetime import datetime, timezone
from getpass import getpass, GetPassWarning
import json
import os
from pathlib import Path, PurePosixPath, PureWindowsPath
import re
import shutil
import sqlite3
import sys
from tempfile import NamedTemporaryFile, TemporaryDirectory
from uuid import uuid4
import warnings

from pixel_art_publish import digest


DATABASE = 'catalog.sqlite3'
LATEST = 'LATEST.json'
VERSION = 1
REMOTE_PREFIX = 'bdm'
DEFAULT_CONFIG = Path(__file__).resolve().parents[1] / 'config' / 'modelscope.json'
HASH = re.compile(r'[0-9a-f]{64}')
BACKUP_ID = re.compile(r'[0-9]{8}T[0-9]{6}Z-[0-9a-f]{12}')


class BackupError(Exception):
    pass


def configured_dataset(path):
    try:
        value = json.loads(path.read_text(encoding='utf-8-sig'))
    except FileNotFoundError:
        return None
    except (OSError, ValueError):
        raise BackupError(f'Cannot read the BDM ModelScope configuration: {path}') from None
    dataset = value.get('dataset_id') if isinstance(value, dict) else None
    if not isinstance(dataset, str) or not valid_dataset(dataset.strip()):
        raise BackupError(f'Invalid dataset_id in the BDM ModelScope configuration: {path}')
    return dataset.strip()


def valid_dataset(repo):
    return (len(repo) <= 200 and re.fullmatch(r'[\w.-]+/[\w.-]+', repo, re.ASCII) is not None
            and not any(part in {'.', '..'} for part in repo.split('/')))


def credential_module():
    try:
        from win32ctypes.pywin32 import pywintypes, win32cred
    except ModuleNotFoundError:
        raise BackupError('Install pywin32-ctypes to use the token saved in BDM settings.') from None
    return pywintypes, win32cred


def windows_token(repo):
    if os.name != 'nt':
        return None
    pywintypes, win32cred = credential_module()
    try:
        # Matches NativeCredentialStore::modelscope and the Windows keyring target convention.
        credential = win32cred.CredRead(f'modelscope-{repo}.Booru Dataset Manager', win32cred.CRED_TYPE_GENERIC)
    except pywintypes.error as error:
        if error.winerror == 1168:
            return None
        raise BackupError('Could not read the ModelScope token from Windows Credential Manager.') from None
    if credential is None:
        return None
    blob = credential['CredentialBlob']
    if isinstance(blob, bytes):
        blob = blob.decode('utf-16-le')
    dataset, _, token = blob.partition('\n')
    if dataset != repo or not token.strip():
        raise BackupError('The saved ModelScope credential does not match the configured dataset.')
    return token


def save_settings(path, repo, token=None):
    if not valid_dataset(repo):
        raise BackupError('Use a ModelScope dataset ID in owner/name format.')
    if token is not None:
        if not token or len(token) > 1024 or any(not c.isprintable() or c.isspace() for c in token):
            raise BackupError('The Access Token is empty or invalid.')
        if os.name != 'nt':
            raise BackupError('Set MODELSCOPE_TOKEN in the process environment on this platform.')
        pywintypes, win32cred = credential_module()
        try:
            win32cred.CredWrite({
                'Type': win32cred.CRED_TYPE_GENERIC,
                'TargetName': f'modelscope-{repo}.Booru Dataset Manager',
                'UserName': f'modelscope-{repo}',
                'CredentialBlob': f'{repo}\n{token}',
                'Persist': win32cred.CRED_PERSIST_LOCAL_MACHINE,
            })
        except pywintypes.error:
            raise BackupError('Could not save the token in Windows Credential Manager.') from None
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = NamedTemporaryFile(mode='w', encoding='utf-8', dir=path.parent, delete=False)
    scratch = Path(pending.name)
    try:
        with pending:
            json.dump({'dataset_id': repo}, pending, indent=2)
            pending.write('\n')
        scratch.replace(path)
    finally:
        scratch.unlink(missing_ok=True)


def json_file(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
    return path


def file_info(path):
    return {'sha256': digest(path), 'size': path.stat().st_size}


def object_key(info):
    sha = info['sha256']
    return f'objects/{sha[:2]}/{sha}.bin'


class ModelScope:
    def __init__(self, repo, revision, token):
        from modelscope_hub import HubApi
        from modelscope_hub.errors import FileSizeMismatchError, NotExistError

        site = os.getenv('MODELSCOPE_BACKUP_SITE', 'modelscope.cn')
        endpoints = {
            'modelscope.cn': 'https://modelscope.cn',
            'modelscope.ai': 'https://www.modelscope.ai',
        }
        if site not in endpoints:
            raise BackupError('Set MODELSCOPE_BACKUP_SITE to modelscope.cn or modelscope.ai.')
        self.api = HubApi(token=token, endpoint=endpoints[site])
        self.site = site
        self.repo = repo
        self.revision = revision
        self.not_found = NotExistError
        self.size_mismatch = FileSizeMismatchError
        info = self.api.get_repo(repo, repo_type='dataset')
        if info.gated or str(info.visibility).lower() not in {'1', 'private'}:
            raise BackupError('The ModelScope dataset must have private visibility.')

    def fetch(self, key, target, expected=None, *, missing_ok=False):
        remote_path = f'{REMOTE_PREFIX}/{key}'
        with TemporaryDirectory(prefix='.readback-', dir=target.parent) as tmp:
            try:
                path = self.api.download_file(
                    self.repo, 'dataset', remote_path, revision=self.revision,
                    local_dir=tmp, cache_dir=tmp, force=True,
                    expected_sha256=expected['sha256'] if expected else None,
                    file_size=expected['size'] if expected else None,
                )
            except self.not_found:
                if missing_ok:
                    return False
                raise BackupError(f'Remote file is missing: {key}') from None
            except self.size_mismatch:
                if not missing_ok or self.site != 'modelscope.ai':
                    raise
                # A missing path on the international site can return an empty HTTP 200.
                entries = self.api.legacy.list_repo_files(
                    self.repo, 'dataset', revision=self.revision,
                    root=str(PurePosixPath(remote_path).parent),
                )
                if any(entry.get('Path') == remote_path for entry in entries):
                    raise
                return False
            if expected is not None and file_info(Path(path)) != expected:
                raise BackupError(f'Remote file failed SHA-256 or size verification: {key}')
            Path(path).replace(target)
        return True

    def upload(self, key, path):
        result = self.api.upload_file(
            self.repo, 'dataset', path, f'{REMOTE_PREFIX}/{key}', revision=self.revision,
            disable_tqdm=True, commit_message=f'BDM backup: {key}',
        )
        if not result:
            raise BackupError(f'ModelScope did not confirm the upload: {key}')


def catalog_version(db):
    if db.execute('PRAGMA integrity_check').fetchall() != [('ok',)]:
        raise BackupError('SQLite integrity check failed.')
    if db.execute('PRAGMA foreign_key_check').fetchone():
        raise BackupError('SQLite foreign key check failed.')
    for table in ('import_journal', 'deletion_journal'):
        if db.execute('SELECT 1 FROM sqlite_master WHERE name=?', (table,)).fetchone():
            if db.execute(f'SELECT 1 FROM {table} LIMIT 1').fetchone():
                raise BackupError(f'Recover the pending {table} in BDM before backing up.')
    return db.execute('PRAGMA user_version').fetchone()[0]


def clear_agent_sessions(db):
    tables = {row[0] for row in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
    db.execute('PRAGMA secure_delete=ON')
    with db:
        if 'agent_claims' in tables:
            db.execute('DELETE FROM agent_claims')
        if 'agent_work' in tables:
            db.execute('''UPDATE agent_work
                          SET stage=CASE WHEN busy=1 THEN 'recovery_required' ELSE stage END,
                              lease=NULL, owner=NULL, expires=0, viewed=NULL, preview=NULL, busy=0
                          WHERE lease IS NOT NULL OR busy=1''')
        if 'agent_daemon' in tables:
            db.execute('DELETE FROM agent_daemon')
        if 'agent_downloads' in tables:
            db.execute("UPDATE agent_downloads SET state='paused', expires=0 WHERE state IN ('running','pausing')")
    # Rebuild the snapshot so free pages cannot retain earlier lease tokens.
    db.execute('VACUUM')


def media_files(db, locate=lambda path: path):
    for (metadata,) in db.execute('SELECT metadata FROM media ORDER BY id'):
        item = json.loads(metadata)
        image = Path(item['path'])
        if locate(image).parent.joinpath('.repair-journal.json').exists():
            raise BackupError(f'Recover the pending image edit first: {image}')
        yield image, item['sha256']
        yield Path(item['thumbnail']), None
        for extension in ('.raw_tags.txt', '.tags.txt', '.txt'):
            yield image.with_suffix(extension), None
        edit = item.get('edit') or {}
        if edit.get('mask'):
            yield Path(edit['mask']), None
        archive = edit.get('source_archive')
        if archive:
            yield Path(archive['path']), archive['sha256']
        else:
            ext = item['post']['file_ext']
            if ext in {'png', 'jpg', 'jpeg', 'webp'}:
                original = image.with_name(f'downloaded-original.{ext}')
                if locate(original).is_file():
                    yield original, item.get('source_sha256') or None


def windows_path_spelling(value):
    # Rust's canonicalize writes extended Windows paths into BDM's catalog.
    if value[:8].lower() == '\\\\?\\unc\\':
        return '\\\\' + value[8:]
    if value.startswith('\\\\?\\'):
        return value[4:]
    return value


def plan(db, roots):
    files = {}
    def comparable(path):
        return Path(windows_path_spelling(str(path))) if os.name == 'nt' else path

    resolved_roots = [(comparable(root), comparable(root.resolve())) for root in roots]
    for path, sha in media_files(db):
        source = path.resolve()
        source_key = comparable(source)
        path = comparable(path)
        for index, (root, resolved_root) in enumerate(resolved_roots):
            if path.is_relative_to(root) and source_key == resolved_root / path.relative_to(root):
                relative = path.relative_to(root).as_posix()
                prefix = 'library' if index == 0 else f'library-{index}'
                key = f'{prefix}/{relative}'
                break
        else:
            raise BackupError(f'Media path is outside the configured library roots: {path}')
        if not source.is_file():
            raise BackupError(f'Catalog file is missing: {source}')
        info = file_info(source)
        if sha is not None and info['sha256'] != sha:
            raise BackupError(f'Catalog SHA-256 mismatch: {source}')
        files[key] = (source, info)
    return files


def write_backup(source, roots, remote):
    now = datetime.now(timezone.utc)
    backup_id = now.strftime('%Y%m%dT%H%M%SZ-') + uuid4().hex[:12]
    directory = f'backups/{backup_id}'
    database = source / DATABASE
    if not database.is_file():
        raise BackupError(f'Local catalog was not found: {database}')
    with TemporaryDirectory(prefix='.modelscope-backup-', dir=source) as tmp:
        scratch = Path(tmp)
        snapshot = scratch / DATABASE
        with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as live:
            original_version = live.execute('PRAGMA data_version').fetchone()
            pages = live.execute('PRAGMA page_count').fetchone()[0]
            page_size = live.execute('PRAGMA page_size').fetchone()[0]
            if 3 * pages * page_size > shutil.disk_usage(scratch).free:
                raise BackupError('Insufficient disk space to prepare the catalog snapshot.')
            with closing(sqlite3.connect(snapshot)) as db:
                live.backup(db)
                db.execute('PRAGMA journal_mode=DELETE')
                clear_agent_sessions(db)
                schema = catalog_version(db)
                files = plan(db, roots)
            files[DATABASE] = (snapshot, file_info(snapshot))
            if max(info['size'] for _, info in files.values()) > shutil.disk_usage(scratch).free:
                raise BackupError('Insufficient disk space for remote readback verification.')
            size = sum(info['size'] for _, info in files.values())
            print(f'Verifying {len(files)} files ({size:,} bytes)', file=sys.stderr, flush=True)
            verified = set()
            readback = scratch / 'readback.bin'
            for path, info in files.values():
                key = object_key(info)
                if key in verified:
                    continue
                if not remote.fetch(key, readback, info, missing_ok=True):
                    remote.upload(key, path)
                    remote.fetch(key, readback, info)
                readback.unlink()
                verified.add(key)
            with closing(sqlite3.connect(snapshot)) as db:
                if plan(db, roots) != {key: value for key, value in files.items() if key != DATABASE}:
                    raise BackupError('The library changed during backup. Close BDM and retry.')
            if live.execute('PRAGMA data_version').fetchone() != original_version:
                raise BackupError('The catalog changed during backup. Close BDM and retry.')

        manifest_data = {
            'format': VERSION, 'id': backup_id, 'created_at': now.isoformat(),
            'schema': schema, 'roots': [str(root) for root in roots],
            'files': {key: info for key, (_, info) in sorted(files.items())},
        }
        validated_entries(manifest_data, backup_id)
        manifest = json_file(scratch / 'backup.json', manifest_data)
        manifest_info = file_info(manifest)
        remote.upload(f'{directory}/backup.json', manifest)
        remote.fetch(f'{directory}/backup.json', readback, manifest_info)
        latest = json_file(scratch / LATEST, {'id': backup_id, 'manifest': manifest_info})
        remote.upload(LATEST, latest)
        remote.fetch(LATEST, readback, file_info(latest))
    return backup_id


def validated_entries(manifest, backup_id):
    if manifest['format'] != VERSION or manifest['id'] != backup_id:
        raise BackupError('Unexpected backup format or identifier.')
    entries = manifest['files']
    if DATABASE not in entries or not manifest['roots']:
        raise BackupError('Backup manifest is missing its catalog or library roots.')
    libraries = {'library', *(f'library-{i}' for i in range(1, len(manifest['roots'])))}
    seen = set()
    for name, info in entries.items():
        parts = name.split('/')
        if name != DATABASE and (len(parts) < 2 or parts[0] not in libraries):
            raise BackupError(f'Unexpected path in backup: {name}')
        for part in parts:
            if (not part or part[-1] in ' .' or PureWindowsPath(part).is_reserved()
                    or re.search(r'[\\/:*?"<>|\x00-\x1f]', part)):
                raise BackupError(f'Unsafe path in backup: {name}')
        if name.casefold() in seen or not HASH.fullmatch(info['sha256']) or info['size'] < 0:
            raise BackupError(f'Invalid file record in backup: {name}')
        seen.add(name.casefold())
    return entries


def relocate_catalog(stage, destination, manifest, entries):
    roots = [PureWindowsPath(windows_path_spelling(root)) if PureWindowsPath(root).is_absolute()
             else PurePosixPath(root) for root in manifest['roots']]
    libraries = ['library', *(f'library-{i}' for i in range(1, len(roots)))]

    def rebase(value):
        if isinstance(value, dict):
            return {key: rebase(item) for key, item in value.items()}
        if isinstance(value, list):
            return [rebase(item) for item in value]
        if isinstance(value, str):
            for root, library in zip(roots, libraries):
                path = type(root)(windows_path_spelling(value) if isinstance(root, PureWindowsPath) else value)
                if path.is_relative_to(root):
                    parts = path.relative_to(root).parts
                    if '..' not in parts:
                        return str(destination.joinpath(library, *parts))
        return value

    with closing(sqlite3.connect(stage / DATABASE)) as db, db:
        if catalog_version(db) != manifest['schema']:
            raise BackupError('Backup schema version mismatch.')
        for table in ('media', 'ingest_jobs', 'workflow_jobs'):
            if not db.execute('SELECT 1 FROM sqlite_master WHERE name=?', (table,)).fetchone():
                continue
            for rowid, metadata in db.execute(f'SELECT rowid, metadata FROM {table}').fetchall():
                value = rebase(json.loads(metadata))
                if table != 'media' and value.get('state') in ('running', 'queued'):
                    value['state'] = 'interrupted'
                db.execute(f'UPDATE {table} SET metadata=? WHERE rowid=?',
                           (json.dumps(value, ensure_ascii=False), rowid))
        def restored_path(path):
            if not path.is_relative_to(destination) or '..' in path.parts:
                raise BackupError('Restored catalog references a path outside the destination.')
            return stage / path.relative_to(destination)

        for path, sha in media_files(db, restored_path):
            restored_path(path)
            info = entries.get(path.relative_to(destination).as_posix())
            if info is None or (sha is not None and info['sha256'] != sha):
                raise BackupError(f'Restored catalog has an invalid file reference: {path}')
        catalog_version(db)


def restore(destination, remote, backup_id):
    if destination.exists():
        raise BackupError('Restore requires a new destination directory.')
    with TemporaryDirectory(prefix='.modelscope-restore-', dir=destination.parent) as tmp:
        scratch = Path(tmp)
        info = None
        if backup_id is None:
            remote.fetch(LATEST, scratch / LATEST)
            latest = json.loads((scratch / LATEST).read_text(encoding='utf-8'))
            backup_id, info = latest['id'], latest['manifest']
        if not BACKUP_ID.fullmatch(backup_id):
            raise BackupError('Invalid backup identifier.')
        manifest_path = scratch / 'backup.json'
        remote.fetch(f'backups/{backup_id}/backup.json', manifest_path, info)
        manifest = json.loads(manifest_path.read_text(encoding='utf-8'))
        entries = validated_entries(manifest, backup_id)
        if max(entry['size'] for entry in entries.values()) + sum(
                entry['size'] for entry in entries.values()) > shutil.disk_usage(scratch).free:
            raise BackupError('Insufficient disk space to restore the backup.')
        stage = scratch / 'data'
        for name, entry in entries.items():
            target = stage / name
            target.parent.mkdir(parents=True, exist_ok=True)
            remote.fetch(object_key(entry), target, entry)
        relocate_catalog(stage, destination, manifest, entries)
        # The headless BDM server requires its configured library path to exist.
        (stage / 'library').mkdir(exist_ok=True)
        if destination.exists():
            raise BackupError('Restore destination appeared during download.')
        stage.rename(destination)
    return backup_id


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', default=os.getenv('MODELSCOPE_DATASET_ID'), help='Private owner/name')
    parser.add_argument('--config', type=Path, default=DEFAULT_CONFIG, help='BDM ModelScope settings file')
    parser.add_argument('--revision', default='master')
    commands = parser.add_subparsers(dest='command', required=True)
    commands.add_parser('configure', help='Set the dataset and save a hidden token in Windows Credential Manager')
    commands.add_parser('check', help='Verify access and private visibility')
    backup = commands.add_parser('backup', help='Back up a stopped BDM data directory')
    backup.add_argument('--data', type=Path, default=Path(__file__).resolve().parents[1] / 'data')
    backup.add_argument('--library', type=Path, action='append', help='Additional managed library root')
    recovery = commands.add_parser('restore', help='Restore into a new data directory')
    recovery.add_argument('destination', type=Path)
    recovery.add_argument('--id', help='Restore a specific backup instead of LATEST.json')
    args = parser.parse_args()
    try:
        dataset = configured_dataset(args.config) if not args.repo else None
        repo = args.repo or dataset
        if not repo or not valid_dataset(repo):
            parser.error('Set the ModelScope dataset in BDM or specify --repo owner/name.')
        if args.command == 'backup' and not (args.data / DATABASE).is_file():
            raise BackupError(f'Local catalog was not found: {args.data / DATABASE}')
        with warnings.catch_warnings():
            warnings.simplefilter('error', GetPassWarning)
            token = os.getenv('MODELSCOPE_TOKEN')
            if not token and dataset and args.command != 'configure':
                token = windows_token(repo)
            token = token or getpass('ModelScope Access Token: ')
        if not token.strip():
            raise BackupError('Provide a ModelScope Access Token using a local terminal.')
        if args.command == 'configure':
            save_settings(args.config, repo, token.strip())
            print(f'Saved BDM ModelScope settings for {repo}')
            return
        remote = ModelScope(repo, args.revision, token.strip())
        if args.command == 'backup':
            source = args.data.absolute()
            roots = list(dict.fromkeys(path.absolute() for path in [source / 'library', *(args.library or [])]))
            backup_id = write_backup(source, roots, remote)
            print(f'Verified backup: {backup_id}')
        elif args.command == 'restore':
            destination = args.destination.resolve()
            backup_id = restore(destination, remote, args.id)
            print(f'Verified restore: {backup_id} -> {destination}')
        else:
            print(f'Confirmed private ModelScope dataset: {repo}')
    except BackupError as exc:
        parser.exit(1, f'{exc}\n')
    except (GetPassWarning, EOFError):
        parser.exit(1, 'Run in an interactive terminal or provide MODELSCOPE_TOKEN in its environment.\n')
    except ModuleNotFoundError as exc:
        if exc.name != 'modelscope_hub':
            raise
        parser.exit(1, 'Install the ModelScope SDK: python -m pip install "modelscope-hub>=0.7.1,<1"\n')
    except KeyboardInterrupt:
        parser.exit(130, 'Interrupted. Run backup again to verify and reuse any uploaded files.\n')
    except Exception as exc:
        # SDK exception messages may contain credentials or signed URLs.
        parser.exit(1, f'Backup operation failed ({type(exc).__name__}). Check access, paths and network.\n')


if __name__ == '__main__':
    main()
