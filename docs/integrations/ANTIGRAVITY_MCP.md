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
