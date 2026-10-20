# DINOv3 visual identity

DINOv3 visual identity is a core requirement. Store model identifier/version, preprocessing version, normalized embedding and creation time. Use cosine similarity and a searchable index appropriate to dataset size. Similarity generates candidates but does not prove identity; thresholds require evaluation on labeled positives/negatives. No universal fixed threshold is assumed.

Keep human review and override. Merge/split must preserve media, source provenance, tags, classifications and previous relations. Suggested tables: visual_embeddings, visual_identity_groups, visual_identity_members, duplicate_relations. Validate model export operators, memory and latency before choosing ONNX Runtime. No weights or inference backend are included in this bootstrap.
