# Booru Dataset Manager

A Windows desktop application for managing booru-sourced image datasets, visual identity, metadata sidecars, dataset export, and versioned ModelScope backups.

> **Project status:** Danbooru static-original download, SQLite v3 media/source persistence, character hierarchy, protected tag normalization, text-only AI optimization, local pHash/DINOv3 candidate analysis and dual-signal deduplication, coordinated deletion and basic dataset export are connected. See [implementation status](docs/planning/IMPLEMENTATION_STATUS.md) for verification and remaining work.

## Run the desktop UI

The current built application is `target/debug/bdm-desktop.exe`; use this single entry for all features, including review, inpainting and cropping. Older named builds are archived under `target/archived-builds/`. Cargo's other files and directories under `target/debug/` are build caches, not alternative application versions.

Image review defaults to thumbnails. System Settings → **图片查看 → 审核时自动加载原图** lets you enable background original loading on opening review or switching images; the setting defaults off and persists across restarts. Click **加载原图** or **使用缩略图** to override the current image; switching images follows the saved default. Closing review releases the original. **打开原图** opens the managed file in the system viewer. Crop/inpainting selections on thumbnails use original dimensions and modify the original when saved; zoom/pan only affect the view. Local DINOv3 and inpainting inference use CPU even when CUDA is available.

In the review toolbar’s **更多** menu, **重新下载原图** immediately opens a status popup. Downloading, validation failures and completion remain visible above review; close the result popup to continue. Saved crop/inpainting edits can be replaced with a validated source original, restoring its dimensions and retaining the previous file in a `.redownload-backup-*` file beside the image. Unsubmitted previews must be confirmed or discarded first. This action is independent of the image-edit second-confirmation preference.

Image review gives the image the full width by default. Open **标签与描述** or **局部修复** only when needed. Choose **裁剪比例** beneath the image to create a crop; its action row then offers **预览裁剪**, **保存裁剪**, adjustment and exit. Preview never saves, and Save explicitly replaces the current image, regardless of the inpainting confirmation setting. **更多** also contains **恢复编辑前版本** and the inpainting confirmation setting. Viewing controls and review decisions stay below the artwork.

Use Rust 1.88 or newer and a native platform linker (MSVC Build Tools on Windows, or a configured MinGW toolchain).

Runtime diagnostics are saved locally in `data/logs/desktop.log`; Rust panics also produce `data/logs/crash-<timestamp>-<pid>.log` with the source location and a forced backtrace. Recent crop actions include the media ID, image dimensions, ratio and selection bounds. Logs include startup/build identity and normal exit; an abrupt native/OS termination may leave only the last recorded action. Logs are not uploaded. The previous runtime log is retained as `desktop.previous.log` when the file exceeds 5 MiB at startup. Keep the Actions package's `bdm_desktop.pdb` beside `bdm-desktop.exe` for Windows stack symbols.

```powershell
cargo run --locked -p bdm-desktop
# Explicitly enter the in-memory sample preview:
cargo run --locked -p bdm-desktop -- --preview
```

First normal startup creates an empty collection catalog; later launches restore saved artist/character records. On either collection page, query a complete Danbooru Tag, review its category and API count, then confirm addition. Queries run in the background; duplicate additions reuse the existing record. Select cards or enter a collection gallery and click **下载图片**. Downloads run in the background with bounded retries, cancellation, saved cursors and explicit resume. See the [collection workflow](docs/features/COLLECTION_CATALOG.md).

Developers can use `--preview` to load in-memory samples for cards, grid/masonry layouts, filters, selection and image review, including a rejected-image list and explicit batch-deletion confirmation; there is no sample-mode button or sample suffix in the window title. Sample deletion affects only the current session; production deletion uses a durable cleanup journal for originals, sidecars and database references.

AI API profiles support masked API Key input and secure Windows Credential Manager storage, isolated by stable profile ID; ordinary settings contain only non-secret metadata. Saving does not make network requests; text-only AI jobs are available from **优化**; explicit connection tests remain pending. The [UI design and interaction guide](docs/overview/UI_DESIGN.md) records page ownership, scope rules, preference storage and native screenshot commands.

In **系统设置 → Danbooru**, enter a username and API Key, then save. Windows stores the identity/key pair in Credential Manager; the ordinary preferences file stores only the username. A blank key preserves the saved key for the same account; changing accounts requires a new key. Saving does not make a network request. Explicit Tag queries use saved credentials, or anonymous access when no account is saved. The latest public Tag smoke passed; the saved account failed explicit profile verification. The Add dialog offers inline credential correction and user-selected anonymous retry; System Settings has a Verify Connection action. Successful authenticated access is not verified. Public original ingestion is verified; authenticated access still depends on valid saved credentials.

Artist and Character lists use one toolbar delete entry: click it, select cards, then confirm deletion of the selected records AND their associated unique images. Cards have no individual trash buttons. Deleting an image also removes its owned data and references in other views; unrelated images and unselected collection records remain. The confirmation lists both record and image counts and affected thumbnails. SQLite v1 also supports confirmed deletion of collection records with zero local images. Real image cleanup is connected; running workflows gate destructive actions and changed scopes require reconfirmation.

Collection/media metadata and preferences are stored under the project directory: `data/catalog.sqlite3` and `data/app.ron`. Default originals and sidecars live in `data/library/`. Development startup discovers the repository from the executable location; a standalone packaged executable uses its own directory. Changing the managed-media root preference affects new imports and does not relocate existing data. `--preview` and `--capture` skip the real catalog, source credentials and Tag requests.

## Planned architecture

- Rust workspace with an `egui`/`eframe` desktop application
- SQLite-backed metadata and migrations
- Managed original media with exact-hash duplicate detection
- DINOv3 embeddings and human-reviewable visual-identity matching
- Three UTF-8 sidecars per image: `*.raw_tags.txt`, `*.tags.txt`, and `*.txt`
- Independent artist, character, optimized-tag, and caption processing targets
- Dataset export profiles and resumable jobs
- Manual, versioned ModelScope dataset backups with SHA-256 verification

## Requirements and safety principles

- Original raw tags are provenance and must not be silently rewritten.
- Unknown or ambiguous domain-specific tags must be preserved for review, not guessed or generalized.
- Artist and character names are protected from AI rewriting.
- AI image upload is opt-in; API keys and session credentials must never be sent to AI providers or committed to source control.
- Source adapters must respect provider authentication requirements and access restrictions.
- A backup is considered current only after its complete manifest and all required objects have been verified.

## Development status

Default acquisition is static images only: skip GIF, videos (WebM/MP4 and other formats), and other animated images. The default configuration, desktop preference compatibility and domain type gate are implemented; original classification, bounded transfer and actual PNG/JPEG/WebP format/dimension/animation validation are connected. Unsupported static formats are skipped explicitly. See [ingestion pipeline](docs/features/INGEST_PIPELINE.md).

The implementation-status document distinguishes specified, implemented, and verified work. Build, platform integration, model inference, provider authentication, and remote-backup behavior must not be marked verified until the relevant checks have actually run.

## License

License selection is pending. Do not assume this project is licensed for redistribution until a license is added.

## Download, optimization and deduplication

- **下载图片:** Select artist/character cards or enter their gallery. Default originals live in the project's `data/library/`; a configured absolute managed root is used for new imports. Images must be static PNG/JPEG/WebP with a default minimum short side of 800 px and at most 64 MiB. System Settings → Download scope lets you save a different minimum (1–16384 px); new jobs use it and resumed jobs retain their original threshold. Raw tags and source identities remain preserved.
- **角色分级:** Expand the Character page hierarchy. Download completion queues cached alias/relationship discovery; manually refresh, confirm/correct base-to-variant relations, and edit mutually exclusive costume groups. Cyclic relations are rejected and manually removed candidates remain rejected. Only confirmed relationships remove redundant base tags; ambiguous variants stay visible in Tag Management.
- **优化:** Choose one target (tags or caption) and a saved OpenAI-compatible profile. Requests are strict JSON, exclude artist/character/unknown protected names, and merge names locally. Caption output requires review. Reviewed text is retained; prior artifact versions remain in SQLite.
- **下载 → 去重流水线:** New commits enter a durable queue while later images continue downloading. One dedup worker consumes batches of up to 64. pHash <= 2 and DINOv3 cosine >= 0.997 follow D-123–132. One signal produces a review candidate; two signals use aesthetic-if-available, resolution, PNG and confirmed watermark-free priority. Deletion is permanent, preserves source/collection associations and records an audit entry. Pause stops automatic continuation; **继续自动去重** resumes queued media. Restart needs explicit resume. Missing-model/failed items remain available for later retry.
- **手动标签处理:** Tag optimization, caption generation and online alias/character discovery start only when you click their controls. Downloads preserve source tags and apply saved deterministic local rules; they never dispatch AI optimization or metadata discovery.
- **导出:** Choose tags or caption plus an absolute destination. The initial layout writes one `media_<id>/` folder per Keep image, selected text as `<basename>.txt`, and an integrity manifest. Existing output must match both image hash and text; conflicts stop the job rather than overwrite.

For local DINOv3 development, run `powershell -File scripts/setup_analysis.ps1`; **系统设置 → 下载模型** downloads the pinned model and checks the published SHA-256. This machine's model is prepared under `models/dinov3/` and the isolated Python runtime under `target/analysis-runtime/`. Release packaging of that runtime remains pending. Images never leave the computer during DINOv3 analysis; this version's AI requests contain text only.
