# 复杂画面、OCR 与数据索引

用于出现文字、贴图、叠加素材、独立画框、局部放大图或解剖示意的图片。先看全图中的区域关系；仅在无法辨认的重要细节处请求 tile。不为简单画面机械分区，也不增加一次独立付费分析。

## 逐层覆盖

1. 区分主画面、背景和独立／叠加的辅助区域。识别主体数量、服饰、姿态、道具、构图、光色和可见的皮肤／布料／笔触特征。
2. 每处重要文字、对话框、拟声词、签名、水印、Logo 和印花都要核实。贴纸／装饰图案描述形状、颜色、位置、遮挡关系；不要把画在衣服上的图案误作后贴的素材。
3. 嵌入图片说明额外画面和主图的关系；局部放大框说明放大对象、视角、边框及连线关系，不误算为多一个主体。多个独立小格按视觉重要性依次覆盖。
4. 将这些关系合成连贯英文 Caption，另提交对应通用 Tag。长度随信息量调整；复杂区域不能为满足字数上限而被一句“some details/text”带过。协议仍有 8192 字节上限，确实无法完整表达时记录待复核，不悄悄截掉关键区域。

## OCR 四种情况

| 可读性 | `legibility` | `text` 与正文 |
| --- | --- | --- |
| 清楚可读 | `readable` | 原样转录，保留语言、大小写、主要标点；正文说明内容、位置、字体外观、颜色、排版 |
| 只有一部分可读 | `partial` | 只写确定的原字符；由此字段说明不完整，不补猜测字符 |
| 看不清 | `unreadable` | 精确使用 `unreadable text`；描述所在位置和视觉外观 |
| 装饰性伪文字 | `decorative` | 精确使用 `unreadable text`；描述伪文字／符号，不编可读单词 |

外文保留原文。只有翻译可靠且有用时在 Caption 补充含义；不得用英文翻译替代索引中的原文。对话框仅在归属清楚时说明说话者；拟声词同时描述文字及其视觉效果。

签名、水印、账号正文只描述存在、位置和外观，精确字符串留在 `text_regions`，不把账号当作画师身份重新灌入训练正文。`language` 可用语言名／代码，不能确认时省略；`appearance` 记录可见字体形状、颜色、方向、边框等。`confidence` 为可选 0–1 的模型主观判断，不是经过校准的识别概率，拿不准用 null／省略，不能统一填 1。

## X-ray 与断面是不同结构

- 主体轮廓中叠加、透视显示内部结构：`visual_regions.kind="x_ray"`，`special_elements=["x_ray"]`。对应 Tag 通常拼写为 `x-ray`。
- 独立画框中的切面／解剖示意，或实际可见的剖面结构：`kind="cross_section"`，`special_elements=["cross_section"]`；对应 Tag 通常为 `cross-section`。
- 两种结构同时可见才同时记录；普通局部放大图不自动属于其中任何一种。分别描述区域展示什么、在哪里、与主图如何对应；不能只加 Tag 而遗漏构图关系。
- 成人性行为示意只以客观、非渲染性措辞记录直接可见的结构；`internal cumshot` 等既有词仍须通过本地词表及像素依据双重核验，不能从来源 Tag 推测画外细节。
- 涉及性内容而年龄不明确等收录疑问，填写具体 `review_reasons`，让图片进入 `defer` 复核并撤下训练通过状态。可只提交复核原因，不必先生成性内容标注；返回 `review_saved` 不是新增训练 TXT。不要声称已证实成年，也不自行删除。明确不合适的内容按用户既定收录政策处理。

Tag 拼写以返回的可用词表为准。`x_ray/cross_section` 是协议枚举，`x-ray/cross-section` 是标签形式，二者不能混为一谈。仅在本地已确认对应通用词时允许转换；不联网逐图查词、不创建猜造标签。

## 区域字段示例

以下是假设的英文招牌与装饰贴图，数值不能照抄到实际图片。画师／角色前缀仍由本地统一加上，不在 caption 正文写 `by Artist` 或角色名。

```json
{
  "text_regions": [{
    "text": "HELLO SPRING",
    "bbox_1000_xyxy": [30, 40, 390, 150],
    "kind": "overlay",
    "legibility": "readable",
    "language": "en",
    "appearance": "Large pale-blue decorative uppercase lettering at the upper left.",
    "confidence": null
  }],
  "visual_regions": [{
    "kind": "sticker",
    "bbox_1000_xyxy": [800, 800, 960, 960],
    "description": "Small pink heart-shaped decorations.",
    "relation": "Overlapping the floral background at the lower right without covering the figure.",
    "confidence": null
  }],
  "special_elements": [],
  "review_reasons": []
}
```

`visual_regions` 限 64 个，kind 仅为 `subject/background/sticker/overlay/embedded_image/detail_inset/x_ray/cross_section`。每项有 `bbox_1000_xyxy`、`description`、`relation`，可选 `confidence`。所有框对完整当前图归一化到 0–1000；不对缩略图或 tile 直接记坐标。不得把协议 kind 当作已经核验的训练标签。

## 保存、追溯和导出

同一次 `bdm_save_annotation` 提交 caption、tags 和区域数据；省略字段保留、`[]` 清空。裁剪改变像素后必须按新图重查旧区域；`stale_annotation_details` 不能靠更换 SHA 字符串绕过。检查响应里的区域数量、结构保存状态和 `review_reasons`，不能只看 TXT 保存成功。

现有数据库保留当前区域数据，审计记录保留本次模型、原因、图片哈希及已保存的结构快照。已经由界面记录的人工标注审核会单独标为 human；外部编辑但没有记录的历史不能凭空补造。

导出时在数据集根目录生成 **`annotations.jsonl`**，记录当前区域、OCR、置信度、图像／文本哈希、来源、模型及已记录的修正／人工审核历史。每条索引以导出图片路径为键；同目录重复导出更新已有记录，不叠出副本，不覆盖被用户手改的索引。多分辨率导出中区域坐标明确属于源当前图，并附缩放／裁剪变换，不能冒充变体坐标。

文件命名沿用现有程序：库内自然语言 `.txt`、优化 `.tags.txt`、原始 `.raw_tags.txt`，来源元数据独立保留；训练扁平导出继续是一张图配所选训练 `.txt`。不因示意中的 `.caption.txt/.raw.txt` 改名或多复制一套。JSONL 是数据集级索引，原始 API／原始 TXT 不改写。

仍沿用已确定的 2020–2026 **来源站上传年份**、画师白名单、画风分类与美学筛选标准；标注 Skill 不把这些筛选阶段宣称为已执行。
