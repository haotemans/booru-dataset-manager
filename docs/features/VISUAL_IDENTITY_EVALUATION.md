# Visual identity evaluation protocol

DINOv3 visual similarity is used to propose candidate relations, not to assert identity automatically without evaluation. Dataset identity policy must distinguish same image, resized/recompressed copies, crops, alternate frames/poses, same character, same artist and merely similar composition. These are different relation labels and must not be collapsed into one binary duplicate label.

## Evaluation dataset

Create a versioned, manually labeled set with positive and hard-negative pairs. Include exact duplicates, resize/recompression, crop, near-identical images, same character different scene, same artist different subject, style similarity and unrelated negatives. Split by source/identity group to reduce leakage between tuning and evaluation sets. Document label definitions and adjudication for ambiguous pairs.

## Metrics

Report precision, recall, F1, precision-recall curve, candidate recall at K, false-positive rate for high-impact auto-actions, latency percentiles, throughput, memory and index size. Threshold selection should prioritize the user's error-cost preference; merge mistakes are more destructive than missed suggestions, so automatic merging should be conservative or disabled until evidence supports it.

## Versioning

Persist model identifier/hash, embedding dimension, preprocessing version, distance metric, index configuration, threshold policy version and evaluation dataset version. A model or preprocessing change invalidates comparability unless explicitly migrated/re-embedded. Never mix embeddings from incompatible versions in the same search index without an explicit compatibility layer.

## Human controls

Present candidates with similarity score and reason/context; allow accept, reject, defer, merge and split with audit history. Merge/split preserves source provenance and tags. Similarity score must not be displayed as a calibrated probability unless calibration has actually been measured.


## Automation gate for near-duplicate grouping

Automatic grouping is allowed only for the near-duplicate relation class (for example resize/recompression) and only after a versioned threshold policy has been evaluated on a labeled dataset containing hard negatives such as crops, different poses/scenes, same-character alternatives, similar style/composition and different identities. Report false-positive rates separately for each hard-negative class; do not use aggregate precision alone to justify automation. If the policy fails the predefined acceptance gate, keep all candidate relations in human review. Store model/preprocessing/policy versions with each automated decision and preserve a reversible audit trail.
