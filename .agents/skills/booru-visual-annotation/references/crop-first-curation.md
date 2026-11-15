# 联合筛选：先尝试改善，再按最终画面判断

仅用于用户选定了筛选、裁剪和标注的联合任务。独立打标不启动额外筛选／裁剪。检查 `bdm_policy.capabilities` 包含 `crop_first_assessment`。v3 长批次先用 [durable-work.md](durable-work.md) 创建 curate 任务；下文独立调用的业务参数放入同名 work 操作的 payload，由服务端执行阶段限制。

## 执行顺序

1. 查看完整当前图，初步判断技术质量、内容适配和构图；只提出候选，不在此时生成将被裁掉内容的长 Caption。
2. 发现边缘水印、多余边缘或可通过合理裁剪改善的构图时，用 `bdm_assess(stage="preliminary",crop_candidate=true,decision="defer")` 记录候选和具体理由。初筛分数可为 null，不编确定评分。服务端将其置为待处理、撤下训练通过状态，不能提前加入待删除。
3. 使用现有比例和裁剪工具预览。只在保留主体动作、场景关系、关键道具、光影和有意义留白时应用；原图留存，结果保留原像素 PNG、32 对齐。现有四档和比例沿用，不强制所有图片为 1024；最佳桶优化仍须按实际实现能力判断。
4. 应用后重新查看保存的当前图和新 SHA，调用 `bdm_assess(stage="final",crop_outcome="applied")`，**根据最终图**给出 KEEP／DEFER／REJECT。不得把旧分数或旧框换个 SHA 冒充复评。
5. 预览不改善或损害表达时，不应用该预览；在同一原图 SHA 下调用 `stage="final",crop_outcome="not_improved"`，说明具体判断。服务端要求本连接实际生成过裁剪预览。重连后预览凭据丢失就重新预览；已应用裁剪的来源／父哈希可从持久编辑记录恢复。
6. 最终 KEEP 后才调用 `bdm_save_annotation(require_final_assessment=true)` 生成本次完整流程的 Caption、Tags、区域。复用刚才的最终图视觉依据，无须额外为评分再看一次。裁剪使旧标注失效且当前任务已授权改写时使用 `overwrite=true`；其余保留默认保护。
7. 最终仍不达标才加入现有待删除审核；不确定则 DEFER。两者都不生成该完整流程的最终训练 TXT，也不物理删除文件。原始数据不改写。

没有可改善候选、已经满足所需构图时，可直接在当前完整图上完成 final 判断。确证的不可修复技术问题／不适合当前内容范围可依已有收录政策处理，不为满足流程而制造无意义预览。独立评分与独立打标不受联合流程的隐式扩展影响。

## 返回与续跑

- `agent_review.stage="preliminary"`／`requires_final_assessment=true`：继续裁剪或复评，不能报作最终通过。
- `crop_first_required`：缺少实际裁剪／预览依据，或试图重置未解决候选；按 `next` 继续，不重新下载原图或反复生成 Caption。
- `final_assessment_required`：本次完整流程还没有当前图的 final KEEP；完成复评后再提交标注。
- `image_not_viewed`／`stale_image`：查看实际当前图并更新依据；保留已经保存的非失效结果。
- 最终 REJECT 只进入人工待删除审核，并撤下训练通过状态。任务处理结果与最终物理删除分开统计。

v3 已提供本地持久任务、守护进程、租约和按实际 Kohya 几何复核的最佳桶候选；尚未实现 PR #11 的全部资源调度、RPC 传输和自动部分写入修复。独立工具与任务入口的预览凭据不同：独立工具凭据属于连接，任务凭据持久保存。原像素 PNG 不因档位而缩小；训练器的缩放要求以返回的 bucket_plan 为准。
