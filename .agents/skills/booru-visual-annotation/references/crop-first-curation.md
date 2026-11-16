# 看图提出方案，再裁剪；保留元素逐处标框

只执行用户授权的阶段。联合任务使用 [durable-work.md](durable-work.md) 的 curate 任务；独立裁剪仍可用 bdm_crop，独立标注不额外裁剪。

## 视觉决策

先让客户端模型实际看到完整当前图像，检查主体、动作、手脚、发尾、服饰、关键道具、背景关系和有意留白。读取尺寸／标签、成功加载文件或计算分桶候选不等于已经看图。可疑小字再看局部，避免固定给每张切四角。

- **像用户图 3 的边缘版权字**：先提出一个既有比例的方案，确认只去掉无关边缘、不损害腿部等主体和构图，才预览并应用。不能把“文字靠边”直接当作可裁掉的结论。
- **像用户图 2 的画面内英文**：它可能属于原有视觉设计，不能为了去字切掉脸、手势或重要画面。保留时标注为真实文字区域，按实际可读程度记录，不一律称作水印。
- **像用户图 4 的大幅 Sample 水印**：裁掉它会损害手臂、衣服或完整场景时，保留整体、标水印 BBox。水印本身不是拒绝收录、局部重绘或擦除的授权。
- 去水印与训练比例适配分别判断。可提出保留水印的轻微比例适配方案；如果任何现有比例都明显破坏表达，保留当前图并说明原因，不能硬裁或无意义重裁。

禁止把整批图套同一个框、仅因接近某个桶就居中裁剪，或通过循环 native_crop_preview／直接改库绕过视觉方案和预览。几何工具只提供候选，构图取舍由看到图像的 Agent 决定。

## 裁剪方案和执行

1. 读一次 bdm_policy，复用其 crop_ratios；只从已有 17 个比例中选，不新增比例／分桶目录。
2. 看完整当前图并复用其 SHA。没有安全去字方案时直接保留，不为满足流程而制造预览。
3. 可改善时先 work_assess 记录 preliminary crop_candidate=true，然后 work_preview。独立调用用 bdm_crop(apply=false)。方案 payload 必须给 ratio、bbox_1000_xyxy、具体 reason、实际 model；reason 说明保留什么、移除什么以及选择该比例的构图依据。
4. 实际预览可能为 32 网格和 Kohya 几何略调选区。检查返回的原像素 PNG、selection、bucket_plan；固定比例最多容许 2% 的网格宽高比误差。不能只读 JSON 就宣称确认了预览。
5. 已授权批次由 Agent 自行检查预览并连续执行，除非用户另行要求人工批准，不逐张询问。预览确认后应用同一 bbox、ratio，并原样回传 preview_sha256。durable work_apply 必须回传；改比例或框需要新预览。不得自动应用新方案。若预览损害表达则不应用，已预览候选通过 final crop_outcome=not_improved 结束；已知没有候选不必先标记 crop_candidate。
6. 应用后看新保存的完整当前图，以新 SHA 完成 final crop_outcome=applied 复评。原图留存，保存原像素 PNG，不缩小至 1024／2048，不用 JPEG 再编码。

## 最终保留元素与 BBox

final KEEP 的 work_assess 必须提交 retained_elements：text／watermark／sticker／overlay／logo／signature。确实逐图检查后没有这些元素才填 []；不要按来源 Tag、已移除水印或固定模板推断。

| 保留元素 | BBox 存放位置 |
| --- | --- |
| 画面文字 | text_regions；kind 根据真实场景选 overlay／sign／dialog／clothing_print 等，**不是** text |
| 文字水印／署名 | text_regions(kind=watermark)，原样或部分可读 OCR |
| 不含可读文字的水印／图形标识／署名 | visual_regions(kind=watermark／logo／signature)，描述图形与遮挡关系，不伪造 OCR |
| 贴纸／叠加素材 | visual_regions(kind=sticker／overlay)；含可读字时另记对应文字 |

- 每一处独立文字、水印、贴纸分别标框，不用一个覆盖半幅图的大框交差。BBox 贴近元素边界并覆盖完整可见范围；半透明水印和局部遮挡也不能遗漏。
- 所有框是相对**最终完整当前图**的整数 [left,top,right,bottom]，0–1000。看局部 tile 时换算回完整图坐标；不能把截图、缩略图、局部放大图坐标直接存入源图。
- 裁掉的字不再标进最终图；裁后残留的文字、贴图重新定位。省略字段只保留已验证属于当前 SHA 的区域，不能仅更换旧区域哈希。
- 记录文字的实际可读程度，不猜缺字；水印／署名／账号原文只在 OCR 索引，训练正文说明其位置和外观。身份仍只由本地合并。
- final KEEP 后一次提交所需 Caption／Tags／区域，按 [complex-regions.md](complex-regions.md) 核对。若只有区域任务可只保存区域，不重写训练 TXT。完整流程报告了保留元素却没有匹配框时，服务端会拒绝保存，修正本次 payload 即可。

服务端能校验当前哈希、比例、方案回执和已报告元素的 BBox 类型；看图是否准确、是否漏了某一处元素仍需要 Agent 认真复核。审核页“更多 → 显示标注框”可核对当前图的文字／水印／贴纸框；过期 SHA 的区域不叠加到新图，显示框不改变图片像素。
