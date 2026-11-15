# ADR-006 / W05 draft: Duplicate detection, visual relations, survivor decisions and review routing

**Status:** PARTIALLY ACCEPTED — owner selected **retain the aesthetically best eligible image as the representative/survivor of each true duplicate group** on 2026-11-15. Definitions of qualifying groups, scoring/ranking machinery, and physical deletion policy below remain proposals pending review. PR #11 is design-only; no executable changes. Decisions in ADR-001 (independent daemon) and ADR-002 (Agent-pull work allocation) apply.
**Dependencies:** [ADR-003 node catalog](AGENT_WORKFLOW_NODE_CATALOG.md), [ADR-004 aesthetics](AGENT_AESTHETIC_EVALUATION_WORKFLOW.md), [ADR-005 comic review](COMIC_PAGE_EXCLUSION_AND_OPTIONAL_LABELING.md), [existing DINOv3 design](../features/DINOV3_VISUAL_IDENTITY.md).

## 1. Verified current code (main: 2026-11-15)

- `crates/domain/src/dedup.rs` defines `DuplicateCandidate` fields first/second IDs, pHash distance, cosine, model version and state. `dual_confirmed()` returns true when pHash distance <=2 **and** compatible DINO cosine >=0.997 with known model identity.
- `crates/infrastructure/src/visual.rs` computes pHash/optional DINO features, queries candidate features, stores pending pairs. **For dual-confirmed pairs if `job.kind != "agent-dedup"`**, the existing visual job calls `repo.delete_media` immediately and marks `automatic_removed`. The `agent-dedup` path instead queues candidate pairs and does not perform the same immediate auto-delete.
- `crates/infrastructure/src/agent/tools.rs` exposes `bdm_scan_duplicates` (at most 64 explicit media IDs; visual job invoked synchronously) and `bdm_resolve_duplicates`. It holds a shared media lock during scan computation, and uses a global catalog mutex through the Agent server adapter.
- `crates/infrastructure/src/agent/state.rs` provides Agent review results and a GUI-confirmed deletion mechanism, including validation against current SHA, plus human cancel/defer.
- Existing product design preserves distinct alternate poses, clothing, expressions, scene changes, composition variants, and source associations. DINO similarity is not enough to declare them interchangeable.
- These are **code/doc observations**, not a new policy adoption. A key design question is whether to preserve the normal ingestion auto-deletion path or route **all non-exact** deletion candidates through the same existing deletion-review collection used for comics and Agent rejects. Do not silently change that policy.

## 2. Distinct relation classes

| Type | Example | Local evidence | Default suggested action |
| --- | --- | --- | --- |
| `exact_file` | identical SHA/MD5-verified source bytes | cryptographic equality + file verification | one managed asset, merge valid source/collection associations during import; no redundant physical copy |
| `visually_same` | resized/recompressed same composition | pHash+compatible DINO candidates, visual confirmation for ambiguous | policy-controlled reviewed/automatic handling; preserve valid higher-quality source |
| `crop_or_color_variant` | same illustration, different crop/filter/text overlay | pHash/DINO plus Agent/human comparison | keep relationship and report; not automatically interchangeable |
| `difference_set` | same work, changed expression, costume, censorship, elements | provider-confirmed relation where available or visual/human review | retain variants; group, then optional representative selection for eligible export |
| `same_subject_different_art` | same character with different pose/background | semantic similarity only | preserve both; do not class as near duplicate |
| `similar_style_or_layout` | analogous palette, composition or brushwork | embedding similarity | preserve separately; no deletion suggestion solely from similarity |
| `uncertain` | insufficient/model-incompatible evidence | partial signals | defer with reason, no destructive change |

**Source image association and the physical file are different entities**. A media ID may belong to several artists/collections. Treat 'remove one dataset association', 'choose representative for export', and 'physically delete a managed image' as three different intents and APIs.

## 3. Node-by-node W05 DAG (all interfaces proposed)

| Step | Executor | Input | Output | Schedule/skip rule |
| --- | --- | --- | --- | --- |
| D01 `dedup.exact_lookup` | Rust | verified source hash + provider identity | canonical stored media/ref + source dedupe receipt | run at import; indexed; do not call vision |
| D02 `feature.extract` | local CPU/persistent worker | media ID, SHA, model/preprocessing version | pHash + DINO embedding (or explicit degraded flag) | cache by SHA/model; batch, reuse worker; no global lock during inference |
| D03 `dedup.candidates` | Rust | compatible feature index, scope, search threshold/profile | bounded, paged scored pairs | search index, no O(N^2) per-pair model calls |
| D04 `dedup.relation_classify` | Agent-pull | candidate pair IDs, both current SHAs, full-image views | typed relation + confidence/evidence | batch pair metadata; show side-by-side pixels only as needed |
| D05 `dedup.select_survivor` | Agent/policy/user | verified **true duplicate group** with complete candidate images + comparable aesthetic evidence | **one aesthetically strongest eligible survivor** (or explicit unresolved) + ranked candidates and reasons | **accepted selection objective**; aesthetic primary, technical validity a gate, human overrides win; meaningful variants remain separate |
| D06 `review.route` | Rust | validated pair/current SHAs/reason/policy + lease | existing pending deletion-review entry or stable group decision | human overrides prevail; no immediate file removal unless owner explicitly selects legacy auto policy |
| D07 `review.confirm_delete` | Human GUI | current pending review and full reference scope | audited physical deletion or cancellation | existing confirmed deletion service only; check all associations and stale hashes |
| D08 `dedup.reconcile` | Rust | workflow/candidate/feature cursor | durable status, reused cache, stale/failed pairs | skip already adjudicated pairs, retain human decisions |

W05 is a composable DAG; D01, D02, D03, D04, D05 or D06 may run alone with strict prerequisites. In `node.run`, **no implicit progression** to downstream delete/resolve nodes. Pair nodes are distinct **two-media work items** with ordered ID acquisition to avoid conflicting edits. The review-route node queues only; it never physically deletes.

**Agent reasoning economy:** if a curated pair is already shown during normal visual annotation, relationship observations can be submitted in the same visual pass when both full images were actually presented. Avoid extra model round trips solely to collect separate similarity vs quality fields. Do not force pair reasoning into the ordinary single-image view when the second image is absent.

## 4. Interface contract proposals

| Agent intent | CLI | MCP | Required response |
| --- | --- | --- | --- |
| Start full W05 | `bdm workflow run --profile dedup-review --input @-` | `bdm_workflow_run` | durable job ID, stages, policy snapshot |
| Just compute embeddings | `bdm node run feature.extract --input @-` | `bdm_node_run` | cache hit/miss, feature completeness, model/version |
| Just search pairs | `bdm node run dedup.candidates --input @-` | `bdm_node_run` | paginated pair handles/metrics, total estimate |
| Claim candidate pairs | `bdm work lease --job JOB_ID --capabilities pair-vision` | `bdm_work_lease` | 2 image IDs, both SHA/revisions, current generation, allowed actions |
| View pair | `bdm work view-pair --lease LEASE --work WORK --output-dir ...` | `bdm_work_view_pair` (or generic bounded multi-image view) | actual 2 image content blocks; view receipts; stable ID mapping |
| Submit relation | `bdm work submit --input @-` | `bdm_work_submit` | typed relation, evidence, per-item ack, idempotency |
| Queue candidate | `bdm node run review.route --input @-` | `bdm_node_run` | existing deletion-review item / defer / keep-both |
| Review/status | `bdm jobs status/events` | `bdm_job_status` | progress, pending review, failures and skips |

Suggested output per pair (never trust as an absolute truth):

~~~json
{
  "work_id": "pair-123-456",
  "first": {"media_id": 123, "expected_sha256": "actual-SHA-A"},
  "second": {"media_id": 456, "expected_sha256": "actual-SHA-B"},
  "relation": "crop_or_color_variant",
  "confidence": "medium",
  "evidence": ["matching underlying artwork", "different visible framing"],
  "interchangeable_for_export": false,
  "proposed_survivor_id": null,
  "route": "keep_both",
  "reason": "meaningful composition difference"
}
~~~

The model's `relation`, `confidence`, and `survivor` are **proposals subject to policy checks**, not authorization to modify image bytes. The daemon validates both hashes, per-image review overrides, pair ordering, eligibility for deletion and reference transfer semantics before queueing. The request uses typed tool output and capped image bytes rather than model-facing Base64 JSON.

## 5. Scheduling/caching/restart design

- Compute source/hash lookups as part of existing ingestion, not duplicate a separate Agent step.
- DINO features are CPU-only as in current product design; keep the model worker warm for a bounded batch and release on idle/memory pressure. Do **not** introduce automatic GPU usage without a separate product decision.
- Cache features by current image SHA, preprocessing signature, exact model artifact/runtime version and any compatible reference-space identity. Incompatible embeddings do not produce a numeric cosine candidate.
- Separate feature computation from candidate querying and pair review, each with independent progress and failure/retry state.
- Use single-flight (same image/model/config) and reference-aware candidate caching; backpressure for CPU inference, image decode and review slots.
- Candidate queries use indexed/SQL-limited results, an opaque cursor, bounded top-K and explicit approximation status. Avoid pulling whole DB repeatedly.
- Pair review leases list metadata handles. Full images fetched on demand; both need image-view receipts for mutating decisions. User edits to either image fence stale pair submissions.
- Persist candidate state and human-rejected relation pairs. Human cancellation, manual Keep or choice of variant remains authoritative across rescans/model upgrades.
- No global catalog/media mutation lock during DINO inference, image decoding, model thinking or pair preview generation. Short validate-and-commit transaction/journal only.
- A client disconnect does not halt local feature work. Agent-required pair review waits for a valid new claimant. Explicit release returns uncompleted pairs quickly; lease timeout recovers stalled ones.

## 5A. ACCEPTED principle: keep the aesthetically best image within a true duplicate group

### Scope and selection rules

**Accepted owner requirement:** for a **verified group of interchangeable duplicate images**, keep the aesthetically strongest eligible member as the **survivor / canonical representative**. Other members are removal candidates according to the separately chosen deletion-routing policy; no automatic physical deletion has been authorized merely by selecting a winner.

1. **Group identity comes first.** pHash/DINO features retrieve candidates, but only same-work copies or functionally interchangeable near-identical variants belong to a *deletion-eligible duplicate group*. A collection of the same character, same artist, similar composition, or meaningful expression/costume/pose/crop differences is *not* itself a deletion-eligible group. Source-declared difference sets can remain a separate relation group; any representative there governs **export preference**, not automatic removal of distinct art.
2. **Eligibility gate first, aesthetics rank second.** A candidate with broken bytes, missing verified source, insufficient dimensions for the chosen training scope, unsafe edit state, or known integrity failure cannot be the active training survivor merely due to a high subjective score. Report eligible members separately from technically ineligible/unknown ones. Avoid "more pixels always wins": native dimensions/PNG are **tie-breakers**, not proxies for beauty.
3. **Primary criterion: visual aesthetics** on the current complete image with an agreed, versioned aesthetic Profile. Evaluate composition completeness, harmonious lighting/color, detail/line quality appropriate to style, visible defects/watermarks, and expressive impact. Prefer an **explicit relative comparison within the group** when absolute scores are too close or not calibrated; the output includes why the winner is preferable.
4. **Avoid silent numeric ranking across profiles.** Existing aesthetic reports can be reused only when current SHA, Profile/model/prompt version, evaluation conditions and comparability are valid. Where reports conflict or are absent, Agent gets a bounded ranked-group review lease; it may read full current images side-by-side and submit a comparative winner and confidence. Low-confidence or tied candidates go to `unresolved` / review, not an arbitrary numeric sort.
5. **Tie rules proposed:** if genuinely indistinguishable aesthetically, prefer better preserved full composition / fewer accidental artifacts; then suitable native detail and fidelity; then verified source integrity and lossless format when meaningful; finally deterministic stable ID **only for technically equivalent content and with an audit note**. Do not discard a distinct composition merely because a deterministic tie-break exists.
6. **Survivor and non-survivors:** record the winning media ID, all members and revisions, aesthetic evidence and comparison Profile; **retain/merge valid collection/source associations when deleting**. Non-winners may be queued into the existing deletion-review collection with `duplicate_group_non_survivor` reason, and the final user-confirmed deletion service validates current SHA and shared references. The *policy* of whether strict dual-signal duplicates may bypass the review is **still open (P1/P2/P3 below)** and must not be silently changed by this choice.
7. **Human override:** a user-selected survivor or "keep both" decision has priority over an Agent rerank. Re-evaluation after a new image, edit or Profile version change should not replace human choices without an explicit reset. Other existing queue reasons (e.g. comic) must not be overwritten; aggregate multiple reasons for the same pending media without duplicate cards.
8. **Result and failure:** one valid survivor per confirmed eligible group; zero if unresolved or all items invalid. Group-level compare-and-swap checks every member ID/revision before recording a new winner or queueing removals. Snapshot membership changes invalidate stale ranking. Do not make the group selection transaction wait for model inference.

### Example group outcome (illustrative, not implemented)

~~~json
{
  "group_id": "dup-group-42",
  "group_type": "interchangeable_near_duplicate",
  "group_revision": 3,
  "members": [
    {"media_id": 123, "expected_sha256": "current-sha-a", "aesthetic": 8.3, "technical_eligible": true},
    {"media_id": 124, "expected_sha256": "current-sha-b", "aesthetic": 9.1, "technical_eligible": true},
    {"media_id": 125, "expected_sha256": "current-sha-c", "aesthetic": 8.6, "technical_eligible": true}
  ],
  "selected_survivor_id": 124,
  "selection_basis": "aesthetic_group_comparison",
  "confidence": "high",
  "reason": "More coherent complete composition, clearer visual hierarchy, fewer unintended artifacts",
  "non_survivor_ids": [123, 125],
  "next_action": "queue_for_deletion_review",
  "files_deleted": 0
}
~~~

This example score is synthetic. `next_action` is a **proposed review-routing default**, not an owner-approved answer to the separate P1/P2/P3 deletion policy question.

### Agent throughput and test plan

- DINO candidate retrieval and group construction are local/cached. Avoid N×(N−1)/2 independent Agent pair calls: triage a small contact sheet, then provide bounded full-current-image comparisons and a **group-level ranking work item**. A group lease binds all current member revisions and group revision/fencing generation; larger groups may use staged comparisons with a final consistency check.
- Fuse W16 aesthetic evaluation and W05 group ranking into **one Agent vision pass** when the images are already being evaluated and pixel/context budget permits; do not require aesthetic scoring for byte-identical source hashes.
- Persist ranked scores and relation evidence independently from delete queue state; only rerank if image/group/Profile/model change materially invalidates evidence.
- Acceptance: a group with A=8.3/B=9.1/C=8.6 picks B; group with distinct scene/pose is split rather than losing meaningful images; technically invalid highest-score image is excluded from survivor selection; low-confidence/ties remain unresolved; user override persists; no revision-stale survivor; correct source/collection association transfer; no file deletion solely by this Agent result.

## 6. Explicit policy choice required before implementation

**P1 — Recommended for safety/consistency:** Import performs exact-byte dedupe by reusing canonical managed assets and merging associations. *All non-exact visual deletions*, even `dual_confirmed`, are queued into the **existing pending deletion review**; no physical deletion until user confirmation. AI can still automatically classify pairs and choose a **proposed** survivor, so no manual pair-by-pair pre-screen is needed. This changes current normal ingestion semantics.

**P2 — Preserve legacy dual-signal auto-delete:** Retain current automatic physical removal where pHash<=2 and DINO cosine>=0.997 (known valid model) for the existing `dedup-auto` path, with survivor/association rules and audit. Keep the Agent-initiated review path non-destructive and route other candidates to review. This maintains today's behavior but risks deleting meaningful variants if the threshold is miscalibrated.

**P3 — Configurable per immutable job policy:** Explicit `review_all` vs `legacy_strict_auto` snapshot, with `review_all` as a proposed safe default. This allows controlled opt-in to prior behavior, but requires policy migration and a UI/CLI warning. No automatic selection of the destructive mode from legacy settings without documented compatibility.

Do **not** interpret the prior decision about *comic candidates entering review* as a decision about *dual-signal duplicate auto-deletion*. It is independent. Owner must select P1, P2 or P3.

## 7. Tests and performance gates

1. Import same exact bytes twice from different collections -> one canonical physical asset, valid multi-source associations.
2. Similar but different expression/censorship/background -> preserve differences; no erroneous physical deletion.
3. Pair with missing DINO model -> pending/degraded, no successful dual-signal claim.
4. Cross-batch and cross-collection candidate retrieval, including approximate-index misses, is measured for recall/throughput.
5. Two Agent workers claim same pair -> single valid pair ownership; both image revisions validated.
6. GUI changes one image between compare and submit -> stale pair rejected.
7. Existing manual "keep both" survives rescan/index rebuild/restart.
8. Existing pending deletion review includes dedup and comic candidates distinguished by reason, with no duplicate cards.
9. Confirm/cancel delete validates SHA and shared references; no unintended loss of source provenance; crash recovery works.
10. Profile and model version mismatch invalidates only necessary feature/ranking outputs; no re-download or unnecessary full Agent rescoring.
11. Measure p50/p95 per stage, bytes processed, peak RAM, lock held time, Agent pair view calls, pairs/second and false-positive reviewed relations on representative ≤50k-image libraries.

### Conversation checkpoint
**Accepted:** in true interchangeable duplicate groups choose the best eligible image by aesthetic judgement (with technical eligibility and human overrides). **Next open policy:** whether strict existing auto-delete should remain (P2), be replaced by common review routing (P1), or be a policy switch (P3). No implementation has started.
