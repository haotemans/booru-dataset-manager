"""Work with a Git-versioned BDM catalog and download its originals on demand."""

from __future__ import annotations

import argparse
import base64
from contextlib import closing, redirect_stdout
from hashlib import file_digest, sha256
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import sys
from tempfile import TemporaryDirectory
from uuid import uuid4

from filelock import FileLock, Timeout

from modelscope_backup import (
    BackupError, DEFAULT_CONFIG, HASH, ModelScope, configured_dataset, configured_site,
    file_info, valid_dataset, windows_path_spelling, windows_token,
)


BRANCH = 'bdm-catalog'
TRACKING = f'refs/remotes/modelscope/{BRANCH}'
ITEM_ID = re.compile(r'[0-9a-f]{32}')
ENTRY_PATH = re.compile(r'entries/([0-9a-f]{32})\.json')
REVISIONS = re.compile(r'[0-9a-f]{40}')
EXTENSIONS = {'jpg', 'png', 'webp'}
REVIEWS = {'unreviewed', 'approved', 'rejected'}
HOSTS = {'modelscope.cn': 'www.modelscope.cn', 'modelscope.ai': 'www.modelscope.ai'}


def image_extension(path):
    with path.open('rb') as stream:
        header = stream.read(12)
    if header.startswith(b'\x89PNG\r\n\x1a\n'):
        return 'png'
    if header.startswith(b'\xff\xd8\xff'):
        return 'jpg'
    if header.startswith(b'RIFF') and header[8:] == b'WEBP':
        return 'webp'
    raise BackupError('Choose an original JPEG, PNG or WebP image.')


def original_info(entry):
    return {key: entry['original'][key] for key in ('sha256', 'size')}


def object_path(original):
    digest = original['sha256']
    return f"catalog-originals/{digest[:2]}/{digest}.{original['extension']}"


def entry_path(entry_id):
    if not ITEM_ID.fullmatch(entry_id or ''):
        raise BackupError('Invalid catalog entry ID.')
    return f'entries/{entry_id}.json'


def read_entry(raw, path):
    try:
        entry = json.loads(raw)
        entry_id = ENTRY_PATH.fullmatch(path).group(1)
        original = entry['original']
        if (entry['id'] != entry_id or not all(isinstance(entry[field], str)
                for field in ('name', 'labels', 'collection', 'caption'))
                or entry['review'] not in REVIEWS or not HASH.fullmatch(original['sha256'])
                or type(original['size']) is not int or original['size'] <= 0
                or original['extension'] not in EXTENSIONS):
            raise ValueError
        return entry
    except (AttributeError, KeyError, TypeError, ValueError):
        raise BackupError(f'Invalid catalog record: {path}') from None


class Catalog:
    def __init__(self, workspace, dataset, site, *, git_url=None, remote=None, token=None):
        if not valid_dataset(dataset) or site not in HOSTS:
            raise BackupError('Choose a valid ModelScope site and dataset ID.')
        namespace = sha256(f'{site}/{dataset}'.encode()).hexdigest()[:16]
        self.root = workspace / namespace
        self.worktree = self.root / 'git'
        self.cache = self.root / 'originals'
        self.dataset = dataset
        self.site = site
        self.git_url = git_url or f'https://{HOSTS[site]}/datasets/{dataset}.git'
        self.remote = remote
        self.token = token
        self.git_executable = shutil.which('git')
        if not self.git_executable and os.name == 'nt':
            installed = Path(os.getenv('ProgramFiles', 'C:/Program Files')) / 'Git/cmd/git.exe'
            if installed.is_file():
                self.git_executable = installed
        if not self.git_executable:
            raise BackupError('Install Git to work with the shared catalog.')

    def git(self, *args, allow_failure=False, network=False):
        env = os.environ.copy()
        for key in tuple(env):
            if key.startswith(('GIT_', 'GCM_')):
                env.pop(key)
        env.update(GIT_TERMINAL_PROMPT='0', GCM_INTERACTIVE='Never', GIT_LFS_SKIP_SMUDGE='1')
        options = {'user.name': 'BDM', 'user.email': 'bdm@localhost', 'core.autocrlf': 'false',
                   'commit.gpgSign': 'false', 'credential.helper': '', 'core.hooksPath': os.devnull}
        if network and self.token:
            credential = base64.b64encode(f'oauth2:{self.token}'.encode()).decode()
            options[f'http.https://{HOSTS[self.site]}/.extraHeader'] = f'Authorization: Basic {credential}'
            options['http.followRedirects'] = 'false'
        env['GIT_CONFIG_COUNT'] = str(len(options))
        for index, (name, value) in enumerate(options.items()):
            env[f'GIT_CONFIG_KEY_{index}'] = name
            env[f'GIT_CONFIG_VALUE_{index}'] = value
        result = subprocess.run(
            [self.git_executable, '-C', str(self.worktree), *args], env=env,
            stdin=subprocess.DEVNULL, capture_output=True, encoding='utf-8', errors='replace',
            creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0,
        )
        if result.returncode and not allow_failure:
            raise BackupError(f'Git {args[0]} failed. Check repository access and the local workspace.')
        return result

    def run(self, action, args=None, *, include_view=False):
        self.root.mkdir(parents=True, exist_ok=True)
        try:
            with FileLock(self.root / '.lock', timeout=0):
                self.worktree.mkdir(exist_ok=True)
                if not (self.worktree / '.git/HEAD').is_file():
                    self.git('init', '--object-format=sha1', '--initial-branch', BRANCH)
                # A record is the unit of merge, including edits to different fields.
                attributes = self.worktree / '.git/info/attributes'
                if not attributes.exists():
                    attributes.parent.mkdir(exist_ok=True)
                    attributes.write_text('entries/*.json -merge\n', encoding='utf-8')
                operations = {
                    'status': self.status, 'list': self.list_entries, 'add': self.add,
                    'edit': self.edit, 'import': self.import_media, 'delete': self.delete,
                    'commit': self.commit,
                    'pull': self.pull, 'push': self.push, 'history': self.history,
                    'original': self.download, 'clear-cache': self.clear_cache,
                    'inspect': self.inspect_commit, 'resolve': self.resolve, 'revert': self.revert,
                    'view': self.view,
                }
                if action not in operations:
                    raise BackupError('Unknown shared catalog operation.')
                result = operations[action](**(args or {}))
                if include_view and action != 'view':
                    result['view'] = self.view()['view']
                return result
        except Timeout:
            raise BackupError('Another BDM window is working with this catalog.') from None

    def revision(self, ref='HEAD'):
        result = self.git('rev-parse', '--verify', '--quiet', ref, allow_failure=True)
        if result.returncode == 1:
            return None
        if result.returncode or not REVISIONS.fullmatch(result.stdout.strip()):
            raise BackupError('Cannot read the catalog commit.')
        return result.stdout.strip()

    def conflicts(self):
        paths = self.git('diff', '--name-only', '--diff-filter=U', '-z').stdout.split('\0')
        return [match[1] for path in paths if (match := ENTRY_PATH.fullmatch(path))]

    def status(self):
        local, upstream = self.revision(), self.revision(TRACKING)
        ahead, behind = 0, 0
        if local and upstream:
            ahead, behind = map(int, self.git('rev-list', '--left-right', '--count',
                                            f'{local}...{upstream}').stdout.split())
        elif local:
            ahead = int(self.git('rev-list', '--count', local).stdout)
        elif upstream:
            behind = int(self.git('rev-list', '--count', upstream).stdout)
        changes = self.git('status', '--porcelain=v1', '-z', '--no-renames', '--', 'entries').stdout
        pending = []
        for change in changes.split('\0'):
            match = ENTRY_PATH.fullmatch(change[3:])
            if match:
                pending.append({'id': match[1], 'change': change[:2].strip()})
        return {'head': local, 'upstream': upstream, 'ahead': ahead, 'behind': behind,
                'changes': pending, 'conflicts': self.conflicts(),
                'merge_in_progress': self.revision('MERGE_HEAD') is not None}

    def list_entries(self, query=''):
        unresolved = set(self.conflicts())
        entries = []
        query = query.casefold()
        for path in sorted((self.worktree / 'entries').glob('*.json')):
            if path.stem in unresolved:
                continue
            entry = read_entry(path.read_bytes(), f'entries/{path.name}')
            if query in ' '.join(entry[field] for field in ('name', 'labels', 'collection', 'caption', 'review')).casefold():
                entries.append(entry | {'cached': self.cache_path(entry).is_file()})
        return {'entries': entries}

    def view(self):
        state = self.status()
        details = [{'id': entry_id, 'local': self.conflict_version(entry_id, '2'),
                    'remote': self.conflict_version(entry_id, '3')}
                   for entry_id in state['conflicts']]
        entries = self.list_entries()['entries']
        for entry in entries:
            if entry['cached']:
                entry['cached_path'] = str(self.cache_path(entry))
        return {'view': state | self.history() | {'entries': entries, 'conflict_details': details}}

    def assert_editable(self):
        if self.conflicts():
            raise BackupError('Resolve the conflicting entries first.')

    def assert_clean(self):
        state = self.status()
        if state['changes'] or state['conflicts'] or state['merge_in_progress']:
            raise BackupError('Commit or resolve the local changes first.')

    def cache_path(self, entry):
        return self.cache / Path(object_path(entry['original'])).name

    def store_original(self, path, *, source_sha='', source_md5=''):
        path = Path(path).resolve()
        if not path.is_file():
            raise BackupError('The selected original file was not found.')
        self.cache.mkdir(exist_ok=True)
        with TemporaryDirectory(dir=self.cache) as scratch:
            temporary = Path(scratch) / 'original'
            shutil.copyfile(path, temporary)
            original = file_info(temporary) | {'extension': image_extension(temporary)}
            if source_sha and source_sha != original['sha256']:
                raise BackupError('The file does not match the BDM catalog’s verified original.')
            if source_md5:
                with temporary.open('rb') as stream:
                    actual_md5 = file_digest(stream, 'md5').hexdigest()
                if source_md5.lower() != actual_md5:
                    raise BackupError('The file does not match the source image checksum.')
            target = self.cache_path({'original': original})
            if not target.is_file() or file_info(target) != original_info({'original': original}):
                temporary.replace(target)
        return original

    def save_entry(self, entry):
        path = entry_path(entry['id'])
        data = json.dumps(entry, ensure_ascii=False, indent=2, sort_keys=True) + '\n'
        read_entry(data, path)
        target = self.worktree / path
        target.parent.mkdir(exist_ok=True)
        with TemporaryDirectory(dir=self.root) as scratch:
            temporary = Path(scratch) / 'entry.json'
            temporary.write_text(data, encoding='utf-8', newline='\n')
            temporary.replace(target)
        self.git('add', '--', path)
        return {'entry': entry}

    def add(self, file, name='', labels='', collection='', caption='', review='unreviewed'):
        self.assert_editable()
        name = name.strip() or Path(file).stem
        return self.save_entry({'id': uuid4().hex, 'name': name, 'labels': labels,
                                'collection': collection, 'caption': caption, 'review': review,
                                'original': self.store_original(file)})

    def import_media(self, catalog, media_id):
        self.assert_editable()
        catalog = Path(catalog).resolve()
        if type(media_id) is not int or media_id < 1 or not catalog.is_file():
            raise BackupError('Choose an existing BDM catalog and media ID.')
        with closing(sqlite3.connect(catalog.as_uri() + '?mode=ro', uri=True)) as db:
            row = db.execute('SELECT metadata FROM media WHERE id=?', (media_id,)).fetchone()
            if not row:
                raise BackupError(f'BDM media {media_id} was not found.')
            media = json.loads(row[0])
            text = dict(db.execute(
                "SELECT kind,content FROM text_artifacts WHERE media_id=? AND is_current=1",
                (media_id,),
            ))
            collections = [row[0] for row in db.execute(
                'SELECT c.tag FROM collections c JOIN collection_media cm ON cm.collection_id=c.id '
                'WHERE cm.media_id=? ORDER BY c.tag', (media_id,),
            )]
        post = media['post']
        current = Path(windows_path_spelling(media['path']))
        archive = (media.get('edit') or {}).get('source_archive')
        source_sha = media.get('source_sha256', '')
        source_md5 = '' if source_sha else post.get('md5', '')
        if archive and archive['downloaded_original']:
            original = Path(windows_path_spelling(archive['path']))
            source_sha = source_sha or archive['sha256']
        elif post['file_ext'] in {'jpg', 'jpeg', 'png', 'webp'}:
            downloaded = current.with_name(f"downloaded-original.{post['file_ext']}")
            original = downloaded if downloaded.is_file() else current
        else:
            original = current
        if not source_sha and not source_md5:
            raise BackupError(f'BDM media {media_id} has no original checksum.')
        provider = post.get('provider', 'danbooru')
        identifier = sha256(f'bdm:{provider}:{post["id"]}'.encode()).hexdigest()[:32]
        target = self.worktree / entry_path(identifier)
        if target.exists():
            raise BackupError('This BDM source image is already in the shared catalog.')
        return self.save_entry({
            'id': identifier, 'name': f'{provider} {post["id"]}',
            'labels': text.get('tags') or post.get('tag_string', ''),
            'caption': text.get('caption') or '', 'collection': ', '.join(collections),
            'review': 'unreviewed',
            'original': self.store_original(original, source_sha=source_sha, source_md5=source_md5),
        })

    def get_entry(self, entry_id):
        path = entry_path(entry_id)
        target = self.worktree / path
        if not target.is_file() or entry_id in self.conflicts():
            raise BackupError('The catalog entry is missing or conflicting.')
        return read_entry(target.read_bytes(), path)

    def edit(self, entry_id, file=None, **fields):
        self.assert_editable()
        entry = self.get_entry(entry_id)
        for field in ('name', 'labels', 'collection', 'caption', 'review'):
            if fields.get(field) is not None:
                entry[field] = fields[field]
        if file:
            entry['original'] = self.store_original(file)
        return self.save_entry(entry)

    def delete(self, entry_id):
        self.assert_editable()
        path = entry_path(entry_id)
        self.git('rm', '--force', '--', path)
        return {'deleted': entry_id}

    def commit(self, message):
        self.assert_editable()
        message = message.strip()
        if not message:
            raise BackupError('Enter a commit message.')
        state = self.status()
        if not state['changes'] and not state['merge_in_progress']:
            raise BackupError('There are no changes to commit.')
        if (self.worktree / 'entries').is_dir():
            self.git('add', '-A', '--', 'entries')
        self.git('commit', '-m', message)
        return {'head': self.revision()}

    def require_remote(self):
        if self.remote is None:
            if not self.token:
                raise BackupError('Save a ModelScope Access Token first.')
            self.remote = ModelScope(self.dataset, 'master', self.token, site=self.site)
        return self.remote

    def fetch(self):
        self.require_remote()
        full_ref = f'refs/heads/{BRANCH}'
        result = self.git('ls-remote', '--exit-code', '--refs', self.git_url, full_ref,
                          allow_failure=True, network=True)
        if result.returncode == 2:
            if self.revision(TRACKING):
                raise BackupError('The remote catalog branch is missing.')
            return None
        if result.returncode:
            raise BackupError('Could not read the ModelScope catalog branch. Check Git access.')
        self.git('fetch', '--no-tags', self.git_url, f'{full_ref}:{TRACKING}', network=True)
        return self.revision(TRACKING)

    def pull(self):
        self.assert_clean()
        upstream = self.fetch()
        if not upstream:
            raise BackupError('The shared catalog has no published commit. Commit and push first.')
        if not self.revision():
            self.git('merge', '--ff-only', upstream)
        else:
            base = self.git('merge-base', 'HEAD', upstream, allow_failure=True)
            options = ['--allow-unrelated-histories'] if base.returncode == 1 else []
            result = self.git('merge', '--no-edit', *options, upstream, allow_failure=True)
            if result.returncode and not self.conflicts():
                raise BackupError('Could not merge the remote catalog.')
        return {'head': self.revision(), 'conflicts': self.conflicts()}

    def conflict_version(self, entry_id, stage):
        path = entry_path(entry_id)
        version = self.git('show', f':{stage}:{path}', allow_failure=True)
        return None if version.returncode else read_entry(version.stdout, path)

    def resolve(self, entry_id, side):
        if side not in {'local', 'remote'} or entry_id not in self.conflicts():
            raise BackupError('Choose a conflicting entry and the local or remote version.')
        path = entry_path(entry_id)
        stage = '2' if side == 'local' else '3'
        version = self.conflict_version(entry_id, stage)
        if version is None:
            self.git('rm', '--force', '--', path)
        else:
            self.save_entry(version)
        return {'conflicts': self.conflicts()}

    def push(self):
        self.assert_clean()
        local = self.revision()
        if not local:
            raise BackupError('Create a local commit before pushing.')
        upstream = self.fetch()
        if upstream and self.git('merge-base', '--is-ancestor', upstream, local,
                                 allow_failure=True).returncode:
            raise BackupError('The remote catalog has new commits. Pull and merge before pushing.')
        remote = self.require_remote()
        commits = self.git('rev-list', '--parents', local,
                           *([f'^{upstream}'] if upstream else [])).stdout.splitlines()
        verified = set()
        with TemporaryDirectory(dir=self.root) as scratch:
            target = Path(scratch) / 'original'
            for revision in commits:
                commit, *parents = revision.split()
                changes = self.git('diff-tree', '--root', '-m', '--no-commit-id', '--name-only',
                                   '--no-renames', '--diff-filter=AM', '-r', '-z', commit,
                                   '--', 'entries').stdout.split('\0')
                for path in filter(None, changes):
                    entry = read_entry(self.git('show', f'{commit}:{path}').stdout, path)
                    key = object_path(entry['original'])
                    if key not in verified and not self.original_in_parents(path, entry, parents):
                        self.publish_original(remote, entry, key, target)
                        verified.add(key)
        result = self.git('push', self.git_url, f'{local}:refs/heads/{BRANCH}',
                          allow_failure=True, network=True)
        if result.returncode:
            raise BackupError('Git push was rejected. Check write access or pull remote changes first.')
        self.git('update-ref', TRACKING, local)
        return {'head': local, 'verified_originals': len(verified)}

    def original_in_parents(self, path, entry, parents):
        for parent in parents:
            previous = self.git('show', f'{parent}:{path}', allow_failure=True)
            if not previous.returncode:
                previous_entry = read_entry(previous.stdout, path)
                if previous_entry['original'] == entry['original']:
                    return True
        return False

    def publish_original(self, remote, entry, key, target):
        info = original_info(entry)
        try:
            if not remote.fetch(key, target, info, missing_ok=True):
                source = self.cache_path(entry)
                if not source.is_file() or file_info(source) != info:
                    raise BackupError('The local original is missing or has changed.')
                remote.upload(key, source)
                remote.fetch(key, target, info)
            if file_info(target) != info:
                raise BackupError('The remote original failed verification.')
        except BackupError as error:
            raise BackupError(f"{entry['name']} ({entry['id']}): {error}") from None
        except Exception as error:
            code = getattr(error, 'status_code', None)
            detail = {401: 'authentication required', 403: 'access denied',
                      404: 'original is not readable', 429: 'rate limit'}.get(code)
            if detail:
                detail += f', HTTP {code}'
            elif type(code) is int:
                detail = f'HTTP {code}'
            else:
                detail = type(error).__name__
            raise BackupError(
                f"{entry['name']} ({entry['id']}): ModelScope original verification failed ({detail})."
            ) from None
        finally:
            target.unlink(missing_ok=True)

    def download(self, entry_id):
        entry = self.get_entry(entry_id)
        path = self.cache_path(entry)
        info = original_info(entry)
        if not path.is_file() or file_info(path) != info:
            self.cache.mkdir(exist_ok=True)
            with TemporaryDirectory(dir=self.cache) as scratch:
                target = Path(scratch) / 'original'
                self.require_remote().fetch(object_path(entry['original']), target, info)
                if file_info(target) != info:
                    raise BackupError('The remote original failed verification.')
                target.replace(path)
        return {'path': str(path)}

    def clear_cache(self, entry_id):
        entry = self.get_entry(entry_id)
        path = self.cache_path(entry)
        if path.is_file():
            with TemporaryDirectory(dir=self.cache) as scratch:
                target = Path(scratch) / 'original'
                self.require_remote().fetch(object_path(entry['original']), target,
                                            original_info(entry))
            path.unlink()
        return {'cache_cleared': entry_id}

    def history(self):
        if not self.revision():
            return {'commits': []}
        records = self.git('log', '-50', '-z', '--format=%H%x00%P%x00%s').stdout.rstrip('\0').split('\0')
        return {'commits': [{'id': oid, 'parents': parents.split(), 'message': message}
                            for oid, parents, message in zip(records[::3], records[1::3], records[2::3])]}

    def validate_commit(self, commit_id):
        if not REVISIONS.fullmatch(commit_id or ''):
            raise BackupError('Enter a full commit ID from the catalog history.')
        if self.git('merge-base', '--is-ancestor', commit_id, 'HEAD', allow_failure=True).returncode:
            raise BackupError('The commit is outside this catalog history.')
        return self.git('rev-list', '--parents', '-n', '1', commit_id).stdout.split()[1:]

    def inspect_commit(self, commit_id):
        parents = self.validate_commit(commit_id)
        options = ['--name-status', '--no-renames', '-z']
        if parents:
            diff = self.git('diff', *options, parents[0], commit_id, '--', 'entries')
        else:
            diff = self.git('diff-tree', '--root', '--no-commit-id', '-r', *options,
                            commit_id, '--', 'entries')
        records = diff.stdout.rstrip('\0').split('\0')
        changes = []
        for change, path in zip(records[::2], records[1::2]):
            match = ENTRY_PATH.fullmatch(path)
            if match:
                revision = parents[0] if change == 'D' else commit_id
                entry = read_entry(self.git('show', f'{revision}:{path}').stdout, path)
                changes.append({'id': match[1], 'change': change, 'name': entry['name']})
        return {'diff': {'commit_id': commit_id, 'changes': changes}}

    def revert(self, commit_id):
        self.assert_clean()
        parents = self.validate_commit(commit_id)
        mainline = ['-m', '1'] if len(parents) > 1 else []
        result = self.git('revert', '--no-edit', *mainline, commit_id, allow_failure=True)
        if result.returncode:
            self.git('revert', '--abort', allow_failure=True)
            raise BackupError('Could not revert this commit cleanly.')
        return {'head': self.revision()}


def run_request(config, workspace, action, args, *, include_view=False):
    dataset = os.getenv('MODELSCOPE_DATASET_ID') or configured_dataset(config)
    if not dataset:
        raise BackupError('Set the ModelScope dataset in BDM first.')
    site = os.getenv('MODELSCOPE_BACKUP_SITE') or configured_site(config) or 'modelscope.cn'
    token = None
    if action in {'pull', 'push', 'original', 'clear-cache'}:
        token = os.getenv('MODELSCOPE_TOKEN') or windows_token(dataset)
    try:
        return Catalog(workspace, dataset, site, token=token).run(action, args,
                                                                include_view=include_view)
    except (OSError, TypeError, ValueError, sqlite3.Error) as error:
        raise BackupError(f'The catalog operation failed ({type(error).__name__}). Check the request and workspace.') from None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, default=DEFAULT_CONFIG)
    parser.add_argument('--workspace', type=Path)
    parser.add_argument('--view', action='store_true', help='Return the updated catalog view too.')
    parser.add_argument('action')
    parser.add_argument('arguments', nargs='?', default='{}', help='JSON object. Use - to read stdin.')
    args = parser.parse_args()
    workspace = args.workspace or args.config.resolve().parents[1] / 'data/modelscope-catalog'
    try:
        request = json.load(sys.stdin) if args.arguments == '-' else json.loads(args.arguments)
        if not isinstance(request, dict):
            raise BackupError('Expected a JSON object with operation arguments.')
        with redirect_stdout(sys.stderr):
            result = run_request(args.config, workspace, args.action, request,
                                 include_view=args.view)
    except BackupError as error:
        print(json.dumps({'error': str(error)}))
        return 1
    except Exception as error:
        # SDK errors can contain credentials and signed URLs.
        print(json.dumps({'error': f'ModelScope operation failed ({type(error).__name__}).'}))
        return 1
    print(json.dumps(result, ensure_ascii=True))
    return 0


if __name__ == '__main__':
    sys.exit(main())
