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


## Difference-set and association evaluation

Include source-linked difference/alternate-artwork sets in the labeled data. Evaluate the relation types separately: exact duplicate, near-duplicate, crop/content variation, difference set, alternate pose/frame, same character across scenes, same artist, style/composition similarity, and unrelated. Metrics must identify false merges between meaningful difference variants and unrelated/identity-distinct images. Source-supplied relations are recorded as provenance and can be shown as candidate associations, but must not be mislabeled as byte duplicates. Representative-ranking labels must specify the eligible interchangeable subset, not merely the broad association group.


## Difference-set representative selection evaluation

For source-confirmed difference sets, evaluate automatic representative ranking separately from relationship detection. Collect human preferred-image labels within each set and report top-1 agreement, pairwise preference agreement, disagreement rates, and cases where technical defects or visible text affect ranking. Compare automatic and manual selection workflows. If the ranking service returns an invalid/unavailable result, the system must preserve the set and request manual selection rather than silently dropping all variants. Record selection mode and ranking-policy version for reproducibility.


## Aesthetic-first representative ranking

For each source-confirmed difference set, the default is automatic selection of the single most aesthetically appealing image. Evaluate agreement with human preferences using top-1 accuracy/agreement, pairwise preference agreement, and disagreement analysis grouped by content type and common failure mode. Aesthetic quality and degree of completion are primary; distraction, text and technical defects are supporting evidence. Do not treat resolution, source priority, file size or embedding similarity as proxies for aesthetic appeal. If the model cannot produce a sufficiently reliable ranking, leave the representative unset and request manual selection. Human overrides must be preserved and excluded from being silently overwritten by later runs.


## Retrieval evaluation and scale

Evaluate each retrieval layer separately: exact-hash correctness, source-relationship ingestion, near-duplicate recall/precision, and broad cross-source candidate recall with review workload. Include same-image variants, difference sets, same-character different-pose/scene cases, same-style hard negatives and unrelated images. Benchmark at realistic library sizes (including 100k and 1M items), measuring query latency, memory use, index build/rebuild time, and incremental update behavior. Candidate top-k and latency budgets should be chosen from measured results, not assumed. Verify that stale or failed indexes are surfaced as incomplete analysis.
