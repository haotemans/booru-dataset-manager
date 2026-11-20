import base64
import json
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from modelscope_backup import BackupError, file_info
from modelscope_catalog import Catalog, object_path


PNG = base64.b64decode(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jPZsAAAAASUVORK5CYII='
)


class RemoteOriginals:
    def __init__(self):
        self.files = {}
        self.readable = True
        self.downloads = []
        self.uploads = []

    def fetch(self, key, target, expected, *, missing_ok=False):
        self.downloads.append(key)
        if key not in self.files:
            if missing_ok:
                return False
            raise BackupError('Original is not visible on the remote.')
        if not self.readable:
            raise BackupError('Original is not visible on the remote.')
        target.write_bytes(self.files[key])
        assert file_info(target) == expected
        return True

    def upload(self, key, path):
        self.uploads.append(key)
        self.files[key] = path.read_bytes()


class SharedCatalogTests(unittest.TestCase):
    def setUp(self):
        self.temporary = TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.branch = self.root / 'remote.git'
        git = shutil.which('git')
        if not git:
            self.skipTest('Git is required')
        subprocess.run([git, 'init', '--bare', str(self.branch)], check=True, capture_output=True)
        self.remote = RemoteOriginals()
        self.a = self.clone('a')
        self.b = self.clone('b')
        self.image = self.root / 'source.png'
        self.image.write_bytes(PNG)

    def clone(self, name):
        return Catalog(self.root / name, 'owner/shared', 'modelscope.cn',
                       git_url=str(self.branch), remote=self.remote)

    def run_on(self, client, action, **args):
        return client.run(action, args)

    def publish_one(self):
        entry = self.run_on(self.a, 'add', file=str(self.image), labels='red',
                            collection='Examples')['entry']
        self.run_on(self.a, 'commit', message='Add original')
        self.run_on(self.a, 'push')
        return entry

    def test_pulls_metadata_only_and_fetches_one_verified_original(self):
        entry = self.publish_one()
        key = object_path(entry['original'])
        self.assertEqual(self.remote.files, {key: PNG})
        self.remote.downloads.clear()
        self.run_on(self.b, 'pull')
        entries = self.run_on(self.b, 'list', query='examples')['entries']
        self.assertEqual(entries, [entry | {'cached': False}])
        self.assertEqual(self.remote.downloads, [])
        path = self.run_on(self.b, 'original', entry_id=entry['id'])['path']
        self.assertEqual(Path(path).read_bytes(), PNG)
        self.run_on(self.b, 'original', entry_id=entry['id'])
        self.assertEqual(self.remote.downloads, [key])
        self.run_on(self.b, 'clear-cache', entry_id=entry['id'])
        self.assertFalse(Path(path).exists())
        self.assertEqual(self.run_on(self.b, 'list')['entries'], [entry | {'cached': False}])
        self.run_on(self.b, 'edit', entry_id=entry['id'], labels='changed')
        head = self.run_on(self.b, 'commit', message='Only metadata')['head']
        diff = self.run_on(self.b, 'inspect', commit_id=head)['diff']
        self.assertEqual(diff['changes'], [{'id': entry['id'], 'name': entry['name'], 'change': 'M'}])
        self.remote.downloads.clear()
        self.run_on(self.b, 'push')
        self.assertEqual(self.remote.downloads, [])

    def test_unreadable_original_does_not_publish_or_discard_commit(self):
        self.publish_one()
        head = self.run_on(self.b, 'pull')['head']
        self.image.write_bytes(PNG + b'different original')
        added = self.run_on(self.a, 'add', file=str(self.image))['entry']
        pending = self.run_on(self.a, 'commit', message='Add another original')['head']
        self.remote.readable = False
        with self.assertRaisesRegex(BackupError, added['id']):
            self.run_on(self.a, 'push')
        self.assertEqual(self.run_on(self.b, 'pull')['head'], head)
        self.assertEqual(self.run_on(self.a, 'status')['head'], pending)
        self.remote.readable = True
        self.run_on(self.a, 'push')
        self.assertEqual(self.run_on(self.b, 'pull')['head'], pending)

    def test_conflicting_edits_resolve_without_overwriting_remote(self):
        entry = self.publish_one()
        key = entry['id']
        self.run_on(self.b, 'pull')
        self.run_on(self.a, 'edit', entry_id=key, labels='blue')
        self.run_on(self.a, 'commit', message='Change labels')
        self.run_on(self.b, 'edit', entry_id=key, review='approved')
        self.run_on(self.b, 'commit', message='Approve image')
        self.run_on(self.a, 'push')
        with self.assertRaisesRegex(BackupError, 'Pull and merge'):
            self.run_on(self.b, 'push')
        self.assertEqual(self.run_on(self.b, 'pull')['conflicts'], [key])
        self.run_on(self.b, 'resolve', entry_id=key, side='remote')
        self.run_on(self.b, 'commit', message='Resolve review conflict')
        self.run_on(self.b, 'push')
        self.assertEqual(self.run_on(self.b, 'list')['entries'][0]['labels'], 'blue')
        self.assertEqual(len(self.run_on(self.b, 'history')['commits'][0]['parents']), 2)
        self.assertEqual(len(self.remote.uploads), 1)

    def test_deletion_and_revert_preserve_historical_original(self):
        entry = self.publish_one()
        self.run_on(self.a, 'delete', entry_id=entry['id'])
        deleted = self.run_on(self.a, 'commit', message='Remove entry')['head']
        self.run_on(self.a, 'push')
        self.run_on(self.b, 'pull')
        self.assertEqual(self.run_on(self.b, 'list')['entries'], [])
        self.run_on(self.b, 'revert', commit_id=deleted)
        self.run_on(self.b, 'push')
        self.run_on(self.a, 'pull')
        self.assertEqual(self.run_on(self.a, 'list')['entries'][0]['id'], entry['id'])
        self.assertEqual(len(self.remote.files), 1)

    def test_import_uses_only_the_verified_downloaded_original(self):
        current = self.root / 'cropped.png'
        current.write_bytes(PNG + b'local edit')
        downloaded = self.root / 'downloaded-original.png'
        downloaded.write_bytes(PNG)
        db_path = self.root / 'catalog.sqlite3'
        db = sqlite3.connect(db_path)
        self.addCleanup(db.close)
        with db:
            db.executescript("""
                CREATE TABLE media(id, metadata);
                CREATE TABLE text_artifacts(media_id, kind, content, is_current);
                CREATE TABLE collections(id, tag);
                CREATE TABLE collection_media(collection_id, media_id);
                INSERT INTO text_artifacts VALUES(1, 'caption', 'Local description', 1);
                INSERT INTO collections VALUES(1, 'Examples');
                INSERT INTO collection_media VALUES(1, 1);
            """)
            db.execute('INSERT INTO media VALUES(1, ?)', (json.dumps({
                'path': str(current), 'source_sha256': file_info(downloaded)['sha256'],
                'post': {'provider': 'pixiv', 'id': 45, 'file_ext': 'png'},
            }),))
        entry = self.run_on(self.a, 'import', catalog=str(db_path), media_id=1)['entry']
        self.assertEqual(entry['caption'], 'Local description')
        self.assertEqual(entry['collection'], 'Examples')
        self.run_on(self.a, 'commit', message='Import source')
        self.run_on(self.a, 'push')
        self.assertEqual(self.remote.files, {object_path(entry['original']): PNG})
        downloaded.unlink()
        with self.assertRaisesRegex(BackupError, 'verified original'):
            self.run_on(self.b, 'import', catalog=str(db_path), media_id=1)

    def test_independent_commits_merge_and_preserve_intermediate_history(self):
        entry = self.publish_one()
        self.run_on(self.b, 'pull')
        self.run_on(self.a, 'edit', entry_id=entry['id'], labels='shared')
        self.run_on(self.a, 'commit', message='Edit existing entry')
        self.image.write_bytes(PNG + b'new original')
        pending = self.run_on(self.b, 'add', file=str(self.image))['entry']
        self.run_on(self.b, 'commit', message='Add independent entry')
        self.run_on(self.b, 'delete', entry_id=pending['id'])
        self.run_on(self.b, 'commit', message='Delete independent entry')
        self.run_on(self.a, 'push')
        self.assertEqual(self.run_on(self.b, 'pull')['conflicts'], [])
        self.run_on(self.b, 'push')
        self.assertIn(object_path(pending['original']), self.remote.files)
        self.assertEqual(len(self.run_on(self.b, 'list')['entries']), 1)


if __name__ == '__main__':
    unittest.main()
