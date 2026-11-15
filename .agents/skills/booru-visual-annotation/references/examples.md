# 视觉证据与标注校准

以下场景只用于说明判断方法，不是当前图片的标准答案。不能把文案、物体、身份或坐标套用到别的图。

## 自然语言不要退化成标签串

差：`1girl, solo, highres, masterpiece, beautiful detailed anime art.`

较好（只有画面真符合时）：

> A single pink-haired girl crouches beside a dark leafy hedge in a sunlit garden. Her small figure sits near the lower edge beneath a broad cyan sky, with a vine-covered arch to the left, a blue mailbox to the right, and distant buildings between them. A light shirt is layered beneath a blue patterned pinafore. Large, simplified foliage shapes and broken paint-like edges give the greenery a textured appearance. Pale clouds separate the dark foreground leaves from the spacious sky. A faint signature lies along the lower edge; its lettering is too small to read confidently in this view.

这段的价值来自主体比例、左右关系、光色与笔触等可核对事实，不在于长度。若图里没有签名，最后一句必须删除。

## 标签去冗余保留信息

以下是语义判断示例，实际仍只用本地已确认的通用词：

- 已保留 `white shirt` 时，单独的 `shirt` 通常没有增加信息，可以省略。
- `long hair` 和 `blue hair` 分别描述长度与颜色，不能只留一个；服装颜色也不能替代服装款式、材质或穿着状态。
- 不按包含某个单词就认定涵盖关系；`hair ornament` 不能替代头发本身的特征。是否冗余取决于完整概念，而非词面重叠。
- 50 个词精简到 44 个且信息都在，可以正常保存；50 个词删到 15 个重点词会触发异常拦截，先查漏项，不能用“精炼”解释过去。

## 把不确定性变成准确措辞

| 初步判断 | 应怎样写 |
| --- | --- |
| 只看见白色钟形小花，不能确认品种 | `small white bell-shaped blossoms`，不直接断言 lily of the valley |
| 眼睛被头发或阴影遮住 | 描述遮挡，不从原始标签补瞳色 |
| 手放在脸附近，没有清楚看到手指接触嘴唇 | `a hand raised near the face`，不编咬手套／吃东西 |
| 强烈暖色侧光，但看不到太阳、天空或时钟 | 描述暖色侧光，不擅自补“落日时分” |
| 衣物和背景都是红色，边界有遮挡 | 先确认服装轮廓，不把背景花纹写成衣服印花 |
| 报纸可见，但字太小或是画出来的假字 | 描述纸张和印刷状文字，逐字结果使用 unreadable text |
| 同角色的眼神、手势或服装不同 | 是有意义差分，不能仅凭相似度合并 |

## 文字区域示例

下面坐标是假设值，不可复制到真实图片。实际应查看细节并换算到完整当前图片。

```json
{
  "text_regions": [
    {
      "text": "unreadable text",
      "bbox_1000_xyxy": [620, 950, 800, 995],
      "kind": "watermark",
      "legibility": "unreadable"
    }
  ],
  "special_elements": []
}
```

正文应相应提到下边缘的小型签名／水印。不因有水印就自动擦除或拒绝整张作品。

如果纸张上有成片可见而不可读的印刷字，可以把纸面文字区域作为一块 `other` 记录；对话框、招牌、衣服印花则按真实类型记录。不要为了凑 OCR 结果给无字画面添加区域。

## 保存后要抓到的错误

- 原有几十个普通视觉标签，重写后只剩十几个重点词：是漏标，不能以“精简”为由验收。质量／管理词、完全重复词和本地确认的冗余身份由本地清理；普通视觉词允许去冗余，但不能丢失独立信息。`tag_coverage_incomplete` 只拦截异常大幅删减；按返回的删除集合检查覆盖关系并补齐遗漏，不用无关词凑数，也不机械地把每个原词加回来。来源词若确实与图矛盾，应按实际画面纠正。
- Caption 只讲人物、服装和一句背景，却漏掉明显构图、光色、画法或印花：继续描述可见内容，不因字数达到某个数就结束；没有的内容也不凑。

- 输入 `tags` 很干净，但落盘变成 `highres, commentary request, @artist, ...`：服务端合并回流，当前批次不能验收，重写同一段 caption 不能修复它。
- 返回 `preserved=["caption"]`：本次并未改写该描述。只有它仍与当前图一致时才计为已有结果保留。
- 用户只要求修标签：省略 `text_regions` 和 `special_elements` 即保留原值；不要无意传 `[]` 清空。需要修文字框时才显式传入核实后的完整数组。
- 图里确实有签名或报纸文字，但 `text_regions=[]`：回看细节并补齐；不能以 JSON 合法为由标作逐张验收通过。
- 正文写“白裙”，画面只有一块白色物体且服装边界看不清：降低描述粒度，不用流畅语句掩盖证据不足。
