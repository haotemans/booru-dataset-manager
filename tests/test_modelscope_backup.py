import json
import os
from importlib.util import find_spec
from pathlib import Path
import sqlite3
import sys
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch

PROJECT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT / 'scripts'))
import modelscope_backup as backup

try:
    from modelscope_hub.errors import FileSizeMismatchError
except ModuleNotFoundError:
    FileSizeMismatchError = None


@unittest.skipIf(FileSizeMismatchError is None, 'Optional ModelScope SDK is required')
class ModelScopeRemoteTests(unittest.TestCase):
    def test_international_site_checks_tree_after_empty_response(self):
        expected = {'size': 12, 'sha256': 'a' * 64}
        key = backup.object_key(expected)
        path = f'bdm/{key}'
        with (TemporaryDirectory() as directory,
              patch.dict(os.environ, {'MODELSCOPE_BACKUP_SITE': 'modelscope.ai'}),
              patch('modelscope_hub.HubApi') as create_api):
            api = create_api.return_value
            api.get_repo.return_value.visibility = 'private'
            api.get_repo.return_value.gated = False
            api.download_file.side_effect = FileSizeMismatchError('Empty response')
            remote = backup.ModelScope('owner/data', 'master', 'test-token')
            create_api.assert_called_once_with(token='test-token', endpoint='https://www.modelscope.ai')
            target = Path(directory) / 'readback'
            api.legacy.list_repo_files.return_value = []
            self.assertFalse(remote.fetch(key, target, expected, missing_ok=True))
            api.legacy.list_repo_files.assert_called_once_with(
                'owner/data', 'dataset', revision='master', root='bdm/objects/aa',
            )
            api.legacy.list_repo_files.return_value = [{'Path': path}]
            with self.assertRaises(FileSizeMismatchError):
                remote.fetch(key, target, expected, missing_ok=True)
            with self.assertRaises(FileSizeMismatchError):
                remote.fetch(key, target, expected)


class MemoryDataset:
    def __init__(self):
        self.files = {}
        self.uploads = []
        self.fail_manifest = False

    def fetch(self, key, destination, info=None, *, missing_ok=False):
        if missing_ok and key not in self.files:
            return False
        destination.write_bytes(self.files[key])
        if self.fail_manifest and key.endswith('/backup.json'):
            destination.write_bytes(b'corrupt')
        if info is not None and backup.file_info(destination) != info:
            raise backup.BackupError('readback failed')
        return True

    def upload(self, key, source):
        self.uploads.append(key)
        self.files[key] = source.read_bytes()


class ModelScopeBackupTests(unittest.TestCase):
    def setUp(self):
        temp = TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.data = self.root / 'source'
        self.default_library = self.data / 'library'
        self.custom_library = self.root / '自定义目录'
        image = self.custom_library / 'originals' / 'media_1' / 'original.png'
        image.parent.mkdir(parents=True)
        thumbnail = image.with_name('thumb.jpg')
        archive = image.with_name('downloaded-original.png')
        for file, data in [(image, b'current'), (thumbnail, b'thumb'), (archive, b'source'),
                           (image.with_suffix('.raw_tags.txt'), b'raw'),
                           (image.with_suffix('.tags.txt'), b'tags'),
                           (image.with_suffix('.txt'), b'caption')]:
            file.write_bytes(data)
        (image.parent / 'unreferenced.png').write_bytes(b'excluded')
        self.image = image
        self.data.mkdir()
        self.db = sqlite3.connect(self.data / backup.DATABASE)
        self.addCleanup(self.db.close)
        for migration in sorted((PROJECT / 'crates/infrastructure/src/migrations').glob('*.sql')):
            self.db.executescript(migration.read_text(encoding='utf-8'))
        self.db.execute('PRAGMA journal_mode=WAL')
        item = {'path': str(image), 'thumbnail': str(thumbnail),
                'sha256': backup.digest(image), 'post': {'file_ext': 'png'},
                'source_sha256': backup.digest(archive)}
        with self.db:
            self.db.execute('INSERT INTO media (md5, metadata) VALUES (?, ?)', ('md5', json.dumps(item)))
            self.db.execute('INSERT INTO visual_features VALUES (1, ?, ?, ?)', ('dino', 'pHash', '[0.2]'))
            self.db.execute("INSERT INTO text_artifacts (media_id, kind, content, sha256, provenance, state) "
                            "VALUES (1, 'caption', 'caption', 'hash', 'manual', 'complete')")
        self.remote = MemoryDataset()

    def write(self):
        return backup.write_backup(self.data, [self.default_library, self.custom_library], self.remote)

    def test_restore_and_reuse_objects(self):
        first = self.write()
        self.remote.uploads.clear()
        second = self.write()
        self.assertNotEqual(first, second)
        self.assertEqual(self.remote.uploads, [f'backups/{second}/backup.json', backup.LATEST])
        restored = self.root / 'restored'
        self.assertEqual(backup.restore(restored, self.remote, None), second)
        restored_image = restored / 'library-1' / self.image.relative_to(self.custom_library)
        self.assertEqual(restored_image.read_bytes(), b'current')
        self.assertEqual(restored_image.with_suffix('.txt').read_bytes(), b'caption')
        self.assertEqual(restored_image.with_name('downloaded-original.png').read_bytes(), b'source')
        self.assertFalse(restored_image.with_name('unreferenced.png').exists())
        db = sqlite3.connect(restored / backup.DATABASE)
        self.addCleanup(db.close)
        metadata = json.loads(db.execute('SELECT metadata FROM media').fetchone()[0])
        self.assertEqual(Path(metadata['path']), restored_image)
        self.assertEqual(db.execute('SELECT embedding FROM visual_features').fetchone()[0], '[0.2]')
        self.assertEqual(db.execute('SELECT content FROM text_artifacts').fetchone()[0], 'caption')

    def test_failed_manifest_does_not_advance_latest(self):
        self.write()
        latest = self.remote.files[backup.LATEST]
        self.remote.fail_manifest = True
        with self.assertRaisesRegex(backup.BackupError, 'readback failed'):
            self.write()
        self.assertEqual(self.remote.files[backup.LATEST], latest)

    def test_deleted_media_is_excluded_from_new_restore(self):
        first = self.write()
        self.db.execute('PRAGMA foreign_keys=ON')
        with self.db:
            self.db.execute('DELETE FROM media')
        self.write()
        restored = self.root / 'restored'
        backup.restore(restored, self.remote, None)
        self.assertEqual({path.name for path in restored.iterdir()}, {backup.DATABASE, 'library'})
        self.assertEqual(list((restored / 'library').iterdir()), [])
        previous = self.root / 'previous'
        backup.restore(previous, self.remote, first)
        self.assertTrue((previous / 'library-1' / self.image.relative_to(self.custom_library)).is_file())

    def test_corrupt_object_leaves_no_restored_directory(self):
        self.write()
        self.remote.files[backup.object_key(backup.file_info(self.image))] = b'corrupt'
        destination = self.root / 'restored'
        with self.assertRaisesRegex(backup.BackupError, 'readback failed'):
            backup.restore(destination, self.remote, None)
        self.assertFalse(destination.exists())

    @unittest.skipUnless(find_spec('filelock'), 'Optional ModelScope SDK dependency is required')
    def test_receive_reuses_a_version_and_preserves_previous_local_edits(self):
        self.remote.site = 'modelscope.ai'
        self.remote.repo = 'owner/data'
        inbox = self.root / 'received'
        first = self.write()
        version, directory = backup.receive_latest(inbox, self.remote)
        self.assertEqual(version, first)
        self.assertTrue((directory / backup.DATABASE).is_file())
        local_edit = directory / 'local-edit.txt'
        local_edit.write_text('keep', encoding='utf-8')
        self.assertEqual(backup.receive_latest(inbox, self.remote), (first, directory))

        second = self.write()
        self.remote.fail_manifest = True
        with self.assertRaisesRegex(backup.BackupError, 'readback failed'):
            backup.receive_latest(inbox, self.remote)
        self.remote.fail_manifest = False
        version, new_directory = backup.receive_latest(inbox, self.remote)
        self.assertEqual(version, second)
        self.assertNotEqual(new_directory, directory)
        self.assertEqual(local_edit.read_text(encoding='utf-8'), 'keep')

    def test_snapshot_removes_cached_and_deleted_lease_tokens(self):
        jobs = (PROJECT / 'crates/infrastructure/src/agent/jobs.rs').read_text(encoding='utf-8')
        schema = jobs[jobs.index('CREATE TABLE IF NOT EXISTS agent_jobs'):].split('";', 1)[0]
        token = 'lease-token-for-this-claim'
        deleted = 'token-from-an-earlier-claim'
        self.db.executescript(schema)
        self.db.execute('PRAGMA secure_delete=OFF')
        with self.db:
            self.db.execute("INSERT INTO agent_claims VALUES ('1', 'print', ?)", (token,))
            self.db.execute("INSERT INTO agent_claims VALUES ('2', 'print', ?)", (deleted,))
            self.db.execute("DELETE FROM agent_claims WHERE request_id='2'")
        self.write()
        latest = json.loads(self.remote.files[backup.LATEST])
        manifest = json.loads(self.remote.files[f'backups/{latest["id"]}/backup.json'])
        snapshot = self.remote.files[backup.object_key(manifest['files'][backup.DATABASE])]
        self.assertNotIn(token.encode(), snapshot)
        self.assertNotIn(deleted.encode(), snapshot)
        self.assertEqual(self.db.execute('SELECT response FROM agent_claims').fetchone()[0], token)

    def test_cli_uses_bdm_config_and_saved_token(self):
        config = self.root / 'modelscope.json'
        backup.save_settings(config, 'owner/old')
        backup.save_settings(config, 'owner/shared')
        self.assertEqual(json.loads(config.read_text(encoding='utf-8')), {'dataset_id': 'owner/shared'})
        with (patch.dict(backup.os.environ, {'MODELSCOPE_DATASET_ID': '', 'MODELSCOPE_TOKEN': ''}),
              patch.object(sys, 'argv', ['backup', '--config', str(config), 'check']),
              patch.object(backup, 'windows_token', return_value='dummy-token') as saved_token,
              patch.object(backup, 'ModelScope') as remote):
            backup.main()
        saved_token.assert_called_once_with('owner/shared')
        remote.assert_called_once_with('owner/shared', 'master', 'dummy-token')

    @unittest.skipUnless(os.name == 'nt', 'Rust extended Windows path spelling')
    def test_catalog_paths_written_by_native_windows_mcp_restore(self):
        item = json.loads(self.db.execute('SELECT metadata FROM media').fetchone()[0])
        for name in ('path', 'thumbnail'):
            item[name] = '\\\\?\\' + str(Path(item[name]).resolve())
        with self.db:
            self.db.execute('UPDATE media SET metadata=?', (json.dumps(item),))
        restored = self.root / 'restored-extended'
        version = backup.write_backup(self.data, [self.default_library, self.custom_library.resolve()], self.remote)
        self.assertEqual(backup.restore(restored, self.remote, None), version)
        relocated = restored / 'library-1' / self.image.relative_to(self.custom_library)
        self.assertEqual(relocated.read_bytes(), b'current')


if __name__ == '__main__':
    unittest.main()
