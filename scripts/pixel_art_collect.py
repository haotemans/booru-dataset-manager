#!/usr/bin/env python3
"""Incremental, conservative, reviewable Danbooru pixel-art acquisition.

Only general-rated public media are considered. Captions and source permissions
require separate human review before any training or redistribution.
"""
from __future__ import annotations

import argparse
import collections
import hashlib
import io
import json
import os
import random
import re
import tarfile
import time
from datetime import datetime, timezone
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode, urlparse
from urllib.request import Request, urlopen

from PIL import Image, ImageOps, UnidentifiedImageError

Image.MAX_IMAGE_PIXELS = 50_000_000
BASE = 'https://danbooru.donmai.us'
MAX_IMAGE = 15 * 1024 * 1024
MAX_GIF = 30 * 1024 * 1024
FORBIDDEN_TAGS = {
    'screenshot', 'screencap', 'manga', 'comic', 'low_quality',
    'jpeg_artifacts', 'watermark', 'text_focus', 'gore',
}
TAG_TEXT = {
    '1girl': 'one girl', '2girls': 'two girls', '1boy': 'one boy',
    'solo': 'a single character', 'scenery': 'scenery',
    'landscape': 'a landscape', 'animal': 'an animal', 'cat': 'a cat',
    'dog': 'a dog', 'bird': 'a bird', 'flower': 'flowers',
    'forest': 'a forest', 'house': 'a house', 'sky': 'the sky',
    'night': 'nighttime', 'tree': 'a tree', 'water': 'water',
    'cloud': 'a cloud', 'indoors': 'an interior', 'outdoors': 'an outdoor scene',
}


def atomic_json(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(path.name + '.new')
    temp.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    os.replace(temp, path)


def new_state() -> dict:
    return {'version': 1, 'targets': {'image': 10000, 'gif': 1000},
            'cursor': {'image': None, 'gif': None}, 'entries': [],
            'rejections': {}, 'last_run': None}


def load_state(path: Path) -> dict:
    if not path.exists():
        return new_state()
    data = json.loads(path.read_text(encoding='utf-8'))
    if data.get('version') != 1 or not isinstance(data.get('entries'), list):
        raise ValueError('Unsupported/corrupt progress index; refusing to overwrite')
    ids = [str(e['id']) for e in data['entries']]
    if len(ids) != len(set(ids)):
        raise ValueError('Duplicate post IDs in progress index')
    if any(e.get('kind') not in ('image', 'gif') for e in data['entries']):
        raise ValueError('Unexpected kind in progress index')
    return data


def allowed_media_url(url: str) -> bool:
    parts = urlparse(url)
    return parts.scheme == 'https' and (parts.hostname == 'donmai.us' or
                                        (parts.hostname or '').endswith('.donmai.us'))


class Fetcher:
    def __init__(self, delay: float = 1.05):
        self.delay = delay
        self.previous = 0.0
        self.headers = {'User-Agent': 'BooruDatasetManager-PixelLoopSampler/0.2 '
                       '(https://github.com/haotemans/booru-dataset-manager)',
                        'Accept': '*/*'}

    def get(self, url: str, max_bytes: int) -> bytes:
        if not allowed_media_url(url):
            raise ValueError('Unexpected media host')
        for attempt in range(4):
            pause = self.delay - (time.monotonic() - self.previous)
            if pause > 0:
                time.sleep(pause)
            self.previous = time.monotonic()
            try:
                with urlopen(Request(url, headers=self.headers), timeout=35) as response:
                    if 'text/html' in response.headers.get('Content-Type', ''):
                        raise ValueError('HTML instead of media/API payload')
                    result = response.read(max_bytes + 1)
                if len(result) > max_bytes:
                    raise ValueError('Media exceeds size limit')
                return result
            except HTTPError as exc:
                if exc.code not in (429, 500, 502, 503, 504) or attempt == 3:
                    raise
                retry_after = exc.headers.get('Retry-After', '')
                try:
                    pause = min(60.0, max(2.0, float(retry_after)))
                except ValueError:
                    pause = min(40.0, 2 ** (attempt + 2))
                time.sleep(pause + random.random())
            except (URLError, TimeoutError):
                if attempt == 3:
                    raise
                time.sleep(min(20, 2 ** (attempt + 2)))
        raise RuntimeError('unreachable')


def md5_url(post: dict) -> str:
    url = post.get('file_url') or ''
    if url and allowed_media_url(url):
        return url
    digest = str(post.get('md5') or '')
    ext = str(post.get('file_ext') or '')
    if not re.fullmatch('[0-9a-f]{32}', digest) or not re.fullmatch('[a-z0-9]{2,5}', ext):
        raise ValueError('Invalid source checksum/extension')
    return f'https://cdn.donmai.us/original/{digest[:2]}/{digest[2:4]}/{digest}.{ext}'


def caption(post: dict, kind: str) -> str:
    tags = set(post.get('tag_string_general', '').split())
    terms = [phrase for tag, phrase in TAG_TEXT.items() if tag in tags][:5]
    base = 'A pixel-art animation' if kind == 'gif' else 'A pixel-art illustration'
    return base + (', depicting ' + ', '.join(terms) if terms else '') + '.\n'


def dhash_and_thumbnail(img: Image.Image) -> tuple[str, str]:
    rgb = ImageOps.exif_transpose(img).convert('RGB')
    gray = rgb.convert('L').resize((9, 8), Image.Resampling.LANCZOS)
    p = list(gray.tobytes())
    bits = 0
    for y in range(8):
        for x in range(8):
            bits = (bits << 1) | int(p[y * 9 + x] > p[y * 9 + x + 1])
    # Quantized 8x8 RGB patch; tiny to keep Git metadata index small.
    thumb = rgb.resize((8, 8), Image.Resampling.BOX).tobytes()
    quantized = bytes((v // 16) for v in thumb)
    return f'{bits:016x}', quantized.hex()


def media_info(payload: bytes, kind: str, min_short: int = 128) -> dict:
    with Image.open(io.BytesIO(payload)) as img:
        width, height = img.size
        if min(width, height) < min_short or max(width, height) > 8192:
            raise ValueError('resolution_out_of_range')
        if max(width / height, height / width) > 3.5:
            raise ValueError('extreme_aspect_ratio')
        if kind == 'image':
            if getattr(img, 'is_animated', False):
                raise ValueError('animated_static_format')
            if img.format not in ('PNG', 'JPEG', 'WEBP'):
                raise ValueError('unexpected_static_format')
            img.load()
            color = img.convert('RGB')
            if all(hi - lo < 2 for lo, hi in color.getextrema()):
                raise ValueError('blank_image')
            dh, thumb = dhash_and_thumbnail(color)
            return {'width': width, 'height': height, 'dhash': dh,
                    'thumb': thumb, 'frames': 1, 'duration_ms': 0}
        if img.format != 'GIF':
            raise ValueError('unexpected_animation_format')
        frames = getattr(img, 'n_frames', 1)
        if frames < 6 or frames > 600:
            raise ValueError('bad_frame_count')
        duration = 0
        unique = set()
        for i in range(frames):
            img.seek(i)
            duration += max(20, int(img.info.get('duration') or 100))
            if i % max(1, frames // 24) == 0:
                # Coarse fingerprint for mostly static animations.
                unique.add(hashlib.sha1(img.convert('RGB').resize((12, 12), Image.Resampling.BOX).tobytes()).digest())
        if duration < 600 or duration > 60000 or len(unique) < 4:
            raise ValueError('insufficient_motion_or_duration')
        img.seek(frames // 2)
        dh, thumb = dhash_and_thumbnail(img)
        return {'width': width, 'height': height, 'dhash': dh,
                'thumb': thumb, 'frames': frames, 'duration_ms': duration,
                'unique_sampled_frames': len(unique),
                'training_fps': 8, 'max_training_clip_seconds': 5}


def near_duplicates(existing: list[dict], candidate: dict) -> str | None:
    """Conservative: 64-bit dHash AND matching downsampled colors/aspect."""
    if not existing:
        return None
    target_hash = int(candidate['dhash'], 16)
    target_thumb = bytes.fromhex(candidate['thumb'])
    for entry in existing:
        if entry['kind'] != candidate['kind']:
            continue
        if abs(entry['width'] / entry['height'] - candidate['width'] / candidate['height']) > 0.02:
            continue
        if (target_hash ^ int(entry['dhash'], 16)).bit_count() > 2:
            continue
        base_thumb = bytes.fromhex(entry['thumb'])
        if len(base_thumb) != len(target_thumb):
            continue
        # 0..15 color bins; <=0.4 average bin change = conservative near-identical.
        average_delta = sum(abs(a - b) for a, b in zip(base_thumb, target_thumb)) / len(target_thumb)
        if average_delta <= 0.4:
            return str(entry['id'])
    return None


def preflight(post: dict, kind: str, min_score: int = 4) -> str | None:
    tags = set(post.get('tag_string', '').split())
    general = set(post.get('tag_string_general', '').split())
    if post.get('rating') != 'g' or post.get('is_deleted') or post.get('is_banned'):
        return 'not_general_or_deleted'
    if 'pixel_art' not in general and 'pixel_art' not in tags:
        return 'missing_pixel_art_tag'
    if tags.intersection(FORBIDDEN_TAGS):
        return 'excluded_tag'
    if int(post.get('score') or 0) < min_score and int(post.get('fav_count') or 0) < 2:
        return 'low_community_score'
    ext = str(post.get('file_ext') or '').lower()
    if kind == 'gif' and ext != 'gif':
        return 'unexpected_extension'
    if kind == 'image' and ext not in {'png', 'jpg', 'jpeg', 'webp'}:
        return 'unexpected_extension'
    limit = MAX_GIF if kind == 'gif' else MAX_IMAGE
    if int(post.get('file_size') or 0) > limit:
        return 'over_size_limit'
    if min(int(post.get('image_width') or 0), int(post.get('image_height') or 0)) < 128:
        return 'low_resolution_metadata'
    if not re.fullmatch('[0-9a-f]{32}', str(post.get('md5') or '')):
        return 'missing_md5'
    return None


def run(args: argparse.Namespace, fetcher: Fetcher | None = None) -> dict:
    root = Path(args.root)
    state_path = Path(args.state_file) if args.state_file else root / 'progress.json'
    state = load_state(state_path)
    state['targets'] = {'image': args.target_images, 'gif': args.target_gifs}
    indexed = {str(e['id']) for e in state['entries']}
    by_md5 = {e['md5']: e['id'] for e in state['entries']}
    stats = collections.Counter()
    reject_examples = []
    run_id = args.run_id or (os.environ.get('GITHUB_RUN_ID') or datetime.now(timezone.utc).strftime('%Y%m%d-%H%M%S'))
    if not re.fullmatch('[a-zA-Z0-9_-]+', run_id):
        raise ValueError('Invalid run ID')
    folder = root / 'batches' / run_id
    if folder.exists() and list(folder.iterdir()):
        raise ValueError('Batch ID already contains data; use a new run ID')
    folder.mkdir(parents=True, exist_ok=True)
    fetcher = fetcher or Fetcher(delay=args.delay)
    new_entries = []
    budgets = {'image': args.images_per_run, 'gif': args.gifs_per_run}
    byte_budget = int(args.max_batch_mib * 1024 * 1024)
    current_bytes = 0
    for kind in ('image', 'gif'):
        quota = max(0, min(budgets[kind], state['targets'][kind] - sum(e['kind'] == kind for e in state['entries'])))
        scanned = 0
        for page_num in range(args.max_pages):
            if quota <= 0 or current_bytes >= byte_budget:
                break
            terms = 'pixel_art rating:g' + (' filetype:gif' if kind == 'gif' else '')
            query = {'tags': terms, 'limit': 100}
            if state['cursor'].get(kind):
                query['page'] = 'b' + str(state['cursor'][kind])
            url = BASE + '/posts.json?' + urlencode(query)
            posts = json.loads(fetcher.get(url, 8_000_000))
            if not isinstance(posts, list):
                raise ValueError('Unexpected Danbooru API response')
            if not posts:
                stats[f'{kind}_search_exhausted'] += 1
                break
            for post in posts:
                pid = post.get('id')
                if not isinstance(pid, int) or pid <= 0:
                    continue
                # Move the cursor only past processed/skipped candidates.
                previous_cursor = state['cursor'][kind]
                state['cursor'][kind] = pid
                scanned += 1
                if str(pid) in indexed:
                    stats['existing_post'] += 1
                    continue
                reason = preflight(post, kind, args.min_score)
                if reason:
                    stats[reason] += 1
                    if len(reject_examples) < 80:
                        reject_examples.append({'id': pid, 'kind': kind, 'reason': reason})
                    continue
                if post['md5'] in by_md5:
                    stats['exact_duplicate'] += 1
                    continue
                expected_size = int(post.get('file_size') or 0)
                if expected_size and expected_size + current_bytes > byte_budget:
                    state['cursor'][kind] = previous_cursor
                    stats['batch_byte_budget_reached'] += 1
                    break
                try:
                    blob = fetcher.get(md5_url(post), MAX_GIF if kind == 'gif' else MAX_IMAGE)
                    if hashlib.md5(blob).hexdigest() != post['md5']:
                        raise ValueError('checksum_mismatch')
                    info = media_info(blob, kind, min_short=args.min_short)
                    match = near_duplicates(state['entries'], {'kind': kind, **info})
                    if match:
                        stats['near_duplicate'] += 1
                        continue
                except (HTTPError, URLError, TimeoutError, ValueError, OSError, UnidentifiedImageError) as exc:
                    stats['download_or_inspection_error'] += 1
                    if len(reject_examples) < 80:
                        reject_examples.append({'id': pid, 'kind': kind, 'reason': str(exc)[:120]})
                    continue
                if current_bytes + len(blob) > byte_budget:
                    state['cursor'][kind] = previous_cursor
                    stats['batch_byte_budget_reached'] += 1
                    break
                ext = 'gif' if kind == 'gif' else str(post['file_ext']).lower()
                stem = str(pid)
                data_dir = folder / kind
                data_dir.mkdir(parents=True, exist_ok=True)
                (data_dir / (stem + '.' + ext)).write_bytes(blob)
                (data_dir / (stem + '.txt')).write_text(caption(post, kind), encoding='utf-8')
                (data_dir / (stem + '.json')).write_text(json.dumps(post, ensure_ascii=False) + '\n', encoding='utf-8')
                entry = {
                    'id': pid, 'kind': kind, 'md5': post['md5'],
                    'sha256': hashlib.sha256(blob).hexdigest(),
                    'width': info['width'], 'height': info['height'],
                    'dhash': info['dhash'], 'thumb': info['thumb'],
                    'score': int(post.get('score') or 0),
                    'fav_count': int(post.get('fav_count') or 0),
                    'frames': info['frames'], 'duration_ms': info['duration_ms'],
                    'training_fps': 8 if kind == 'gif' else None,
                    'max_training_clip_seconds': 5 if kind == 'gif' else None,
                    'batch': run_id, 'file': f'{kind}/{stem}.{ext}',
                    'source_post': f'{BASE}/posts/{pid}',
                    'rights_verified': False, 'caption_verified': False,
                }
                state['entries'].append(entry)
                new_entries.append(entry)
                indexed.add(str(pid))
                by_md5[post['md5']] = pid
                quota -= 1
                current_bytes += len(blob)
                print(f'accepted {kind} #{pid} {info["width"]}x{info["height"]} {len(blob)} bytes', flush=True)
                if quota <= 0:
                    break
            if quota <= 0 or current_bytes >= byte_budget or stats['batch_byte_budget_reached']:
                break
            # Avoid scanning unbounded historical pages in one run.
        print(f'{kind}: accepted {sum(e["kind"] == kind for e in new_entries)}, scanned {scanned}', flush=True)
    total = collections.Counter(e['kind'] for e in state['entries'])
    state['rejections'] = dict(collections.Counter(state.get('rejections', {})) + stats)
    state['last_run'] = run_id
    if new_entries:
        (folder / 'manifest.jsonl').write_text(''.join(json.dumps(e, ensure_ascii=False) + '\n' for e in new_entries), encoding='utf-8')
        (folder / 'rejections.json').write_text(json.dumps({'counts': dict(stats), 'examples': reject_examples}, ensure_ascii=False, indent=2), encoding='utf-8')
        archive_dir = root / 'archives'
        archive_dir.mkdir(exist_ok=True)
        archive = archive_dir / f'pixelloop-{run_id}.tar'
        with tarfile.open(archive, 'w') as tar:
            tar.add(folder, arcname=run_id)
        check = hashlib.sha256()
        with archive.open('rb') as archive_stream:
            for chunk in iter(lambda: archive_stream.read(4 * 1024 * 1024), b''):
                check.update(chunk)
        digest = check.hexdigest()
        (archive.with_suffix('.tar.sha256')).write_text(f'{digest}  {archive.name}\n', encoding='utf-8')
        print(f'ARCHIVE {archive} SHA256 {digest}', flush=True)
    atomic_json(state_path, state)
    report = {'run_id': run_id, 'new_images': sum(e['kind'] == 'image' for e in new_entries),
              'new_gifs': sum(e['kind'] == 'gif' for e in new_entries),
              'total_images': total['image'], 'total_gifs': total['gif'],
              'target_images': args.target_images, 'target_gifs': args.target_gifs,
              'cursor': state['cursor'], 'new_bytes': current_bytes,
              'rejection_counts': dict(stats), 'rights_review_required': True}
    atomic_json(root / 'report.json', report)
    print('REPORT ' + json.dumps(report, ensure_ascii=False), flush=True)
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', default='data/pixelloop-curated')
    parser.add_argument('--state-file')
    parser.add_argument('--run-id')
    parser.add_argument('--target-images', type=int, default=10000)
    parser.add_argument('--target-gifs', type=int, default=1000)
    parser.add_argument('--images-per-run', type=int, default=80)
    parser.add_argument('--gifs-per-run', type=int, default=8)
    parser.add_argument('--max-pages', type=int, default=15)
    parser.add_argument('--max-batch-mib', type=int, default=300)
    parser.add_argument('--min-short', type=int, default=128)
    parser.add_argument('--min-score', type=int, default=4)
    parser.add_argument('--delay', type=float, default=1.05)
    args = parser.parse_args()
    if min(args.target_images, args.target_gifs, args.images_per_run, args.gifs_per_run, args.max_pages, args.max_batch_mib) < 0:
        parser.error('Targets and budgets cannot be negative')
    run(args)


if __name__ == '__main__':
    main()
