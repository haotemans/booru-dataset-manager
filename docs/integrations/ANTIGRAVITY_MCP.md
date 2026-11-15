# Antigravity MCP for dataset curation

The existing native desktop executable also serves local MCP over stdio. The
model selected in Antigravity performs visual reasoning. No separate model API,
API key, Python MCP runtime or second executable is required. This service uses
the official Rust MCP SDK. It does not pick or hard-code a Gemini model name.

```json
{
  "mcpServers": {
    "booru_dataset_manager": {
      "command": "D:\\project\\booru-dataset-manager\\dist\\bdm-desktop.exe",
      "args": [
        "--mcp-server",
        "--catalog", "D:\\project\\booru-dataset-manager\\data\\catalog.sqlite3",
        "--project-root", "D:\\project\\booru-dataset-manager",
        "--library-root", "D:\\project\\booru-dataset-manager\\data\\library",
        "--allow-writes"
      ],
      "timeoutSeconds": 600
    }
  }
}
```

Add this entry to Antigravity's current MCP config, preserving its other entries.
The documented global location is `~/.gemini/config/mcp_config.json`; the client
can show its actual config path under MCP settings. Refresh/restart the MCP
server in the client after configuration or an executable update. Paths must be
absolute and the catalog/library must already exist. Omit `--allow-writes` for
inspection-only use. The server itself exposes no network listener. Client
permission prompts remain controlled by Antigravity's permission mode.

## Durable jobs and optional headless CLI

Protocol v3 adds `bdm_agent` for durable batches, leased work, job controls, operation-schema discovery, download queues, bucket planning and local health checks. Existing configuration above remains valid. The optional `bdm.exe mcp-serve` entry serves the same tools without linking the desktop UI; `bdm.exe --help` documents standalone JSON commands. See [the implemented control-plane contract](../agent-control-plane.md) for exact commands, restart behavior and current limits.

## Project annotation skill

The workspace skill [booru-visual-annotation](../../.agents/skills/booru-visual-annotation/SKILL.md)
guides visual evidence, caption/tag separation, text-region inspection and saved-result
verification. Open this repository as the Antigravity workspace and request
`Use booru-visual-annotation to annotate the selected collection` in a new conversation.
It includes a read-only local audit helper; it does not replace visual review or fix
server-side formatting bugs. Creating/loading the skill does not itself run annotation.

## Tools

| Tool | Purpose |
| --- | --- |
| `bdm_policy` | Read the curation workflow and preservation constraints |
| `bdm_collections` | Find existing artist, character or query collections |
| `bdm_list_media` | Page IDs, current hashes and annotation/review states |
| `bdm_view_image` | View the current image, verified complete original or a detail tile |
| `bdm_crop` | Preview, then apply a native-pixel crop with 32 alignment |
| `bdm_restore_original` | Deliberately replace an existing edit with the full source before expanding a crop |
| `bdm_save_annotation` | Save visual captions/tags and text-region details with local identity formatting |
| `bdm_assess` | Save explained aesthetic/training scores and keep/reject/defer decisions |
| `bdm_scan_duplicates` | Analyze up to 64 explicit IDs using existing local pHash/DINOv3 tooling |
| `bdm_resolve_duplicates` | Keep both images, or queue one duplicate while designating its survivor |

Collection and image lists are paginated with `after_id`/`next_after_id`; the
collection tool also accepts a name substring in `collection`. Responses use
compact metadata to fit the client's inline tool budget. Long existing text
can be shortened in inspection results with `content_truncated=true`; stored
TXT bytes are never shortened. Duplicate scans return at most 24 pending pairs
per response; resolve them and rescan the cached scope for remaining pairs.

The model must view the full current image before a mutation. An exact crop
preview is required before applying that crop. Bboxes use normalized
`[left, top, right, bottom]` coordinates in 0..1000 relative to the current image,
not the thumbnail. Inspection displays may be resized to 1024 pixels; saved
images retain the selected native pixels and are center-trimmed only for 32
alignment. No JPEG re-encoding, upscaling or automatic 1024/2048 downsampling is
performed. The existing 576-short-side/550000-pixel floor still applies.

Mutations require the inspected SHA-256, a reason and the client's model label.
Stale hashes fail. Files are restricted to the configured library. Database
mutations share an OS file lock with the GUI. The GUI periodically refreshes
agent results while idle; close an active image-edit panel to refresh it.

Rejecting an image queues it in the existing deletion review. Neither an
aesthetic score nor a duplicate scan permanently deletes images. Final review
checks stale decisions and transfers duplicate collection associations to the
survivor. Human cancellation/overrides persist. Every agent action is logged.
Scores are subjective and may be null; they are stored separately from training
text. Annotation updates retain those scores and raw source data. General tags
are filtered through locally known/source-verified categories. Artist/character
prefixes are merged locally, artist first, with underscores replaced by spaces.
Successful or manually edited text is preserved unless `overwrite=true` is
explicitly requested. A crop marks old annotation results for review.

Only the complete original, current edited PNG and thumbnail are retained.
Crop previews are temporary and removed automatically. Decision reports contain
metadata, not image backups. Missing DINOv3 weights/runtime are reported; a
pHash-only result is not labelled successful DINO analysis.

## Annotation protocol `bdm-agent-v2`

Read `bdm_policy.protocol` once after refreshing the MCP server. For annotation,
the normal path is one full `bdm_view_image` and one `bdm_save_annotation` per
image. The view includes `tag_vocabulary.source_general` (locally accepted
candidates, still requiring visual evidence) and existing `annotation_details`.
No network category lookup or extra model API is used. Detail tiles are optional
when small text needs inspection. After a crop, inspect the new current image.

Save returns `status`, `saved`, `preserved`, `accepted_tags`, individual
`tag_rejections` codes, `readback` content/hashes/truncation and sidecar
verification in the same response. A rejected unknown term no longer discards
a valid caption. `partial` means some output saved; `needs_correction` means no
new output saved. Saved tag text consists of artists, characters, then accepted
generic terms; source meta/protected terms are not copied back. Omitted
`text_regions` and `special_elements` preserve prior values independently;
explicit `[]` clears the selected field. Do not resend already saved caption
just to correct tags. Replacing saved tags requires deliberate `overwrite=true`
and resubmitting the complete desired generic tag array.

All tool responses include compact `next` guidance. Validation errors carry
`code`, `message`, and a recovery step. `unknown_category` requires a verified
term or prose instead; retrying the same unknown word cannot succeed. Existing
successful/manual outputs preserved by `overwrite=false` are not failures.
Readback validates structure/storage, not visual truth. Only fetch full TXT when
truncation prevents review; do not repeat full view/list calls to confirm a save.

The new behavior requires the rebuilt executable and an MCP server refresh.
Loading the updated Skill alone does not upgrade an already running old server.

### Rich regions and dataset index

Check `bdm_policy.capabilities` for `rich_text_regions`, `visual_regions`,
`review_reasons`, and `dataset_jsonl`. Text regions additionally support optional
`language`, `appearance`, and subjective `confidence` (null or 0..1). Legibility
is `readable`, `partial`, `unreadable`, or `decorative`; the last two require the
exact text sentinel `unreadable text`. Partial text contains only recognized
original characters. Do not infer missing words from source tags.

`visual_regions` records important subjects/backgrounds, stickers, overlays,
embedded images, `detail_inset`, `x_ray` and `cross_section` regions. Each has a
normalized full-current-image box, `description`, `relation` to the main frame,
and optional confidence. A separate cutaway and an internal view superimposed
on the subject are different structures, not interchangeable tag spellings.
Protocol enums use underscores; verified Danbooru tags normally use the
hyphenated `x-ray` and `cross-section`. No automatic per-image network lookup is
introduced. All optional region fields share the existing save call.

Nonempty `review_reasons` defers the image and removes training approval while
preserving files. It can be submitted without caption/tags (`review_saved`), so
uncertain content need not be annotated before deferral. Unresolved reasons
block automatic keep decisions. Clearing reasons does not itself approve it.
Region snapshots include the inspected image hash; stale structured data must
be replaced or cleared after inspecting a changed crop. Audit events retain
saved structures so later corrections are traceable without image backups.

On export, a single root-level `annotations.jsonl` includes region/OCR data,
confidence, source, image/text hashes and recorded agent/human review history.
Existing training TXT naming/layout remains unchanged. Repeated exports update
rows by image path, preserve other rows and protect user-modified index files.
For multiresolution variants, coordinates explicitly refer to the source current
image and the record includes its scale/crop transform. Existing legacy records
with no coordinate hash are marked unknown, not silently treated as verified.

## Crop-first quality curation

Capability `crop_first_assessment` adds optional `stage` (default `final`),
`crop_candidate` (default false) and `crop_outcome` to `bdm_assess`. Preliminary
assessment always defers; a correctable crop candidate cannot be rejected or
reset before it is resolved. Apply a previewed crop, inspect the new current
image, then assess with `stage=final,crop_outcome=applied`. If a preview does not
improve expression, keep the original and assess it with `not_improved`.
Edit-parent or preview evidence is checked by the server. Preview receipts are
connection-local; applied edit provenance and pending assessment persist.

For selected full curation, `bdm_save_annotation(require_final_assessment=true)`
requires a current final KEEP result. Standalone annotation defaults false.
Final rejection removes training approval and queues existing human deletion
review; no source file is deleted. Assessment state, keep flag and audit commit
in one SQLite transaction. This does not implement the proposed scheduler or
automatic best-tier/bucket optimization.

## Example instruction for Antigravity

> 使用 booru_dataset_manager MCP 处理 redum4 收藏，先读 bdm_policy。
> 分批查看图片、评分并扫描重复候选；比较原图后决定保留或加入待删除。
> 保留画师风格和不同构图，不把同一角色的不同作品当重复图。
> 只对确有改善的图片预览并应用裁剪，保持原像素、PNG 和 32 对齐。
> 根据最终裁剪画面生成详细自然语言和通用标签，描述可见文字、水印等细节，
> 不猜看不清的文字。跳过已成功且仍与当前画面一致的标注；确需改写时使用
> overwrite=true。完成后报告实际处理、保留、待删除及失败的 ID 和原因。

Configuration reference: [Google Antigravity MCP documentation](https://antigravity.google/docs/mcp).
SDK reference: [official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk).
