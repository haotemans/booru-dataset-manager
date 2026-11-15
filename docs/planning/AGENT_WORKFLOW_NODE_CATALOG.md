# ADR-003: Composable Agent workflows and independently executable nodes

**Status:** DESIGN DRAFT; review required. The owner requests **both complete workflows and single-node tasks**, and asks to review every flow's details and requirements before implementation. This records that product direction, not approval of every proposed node contract or behavior.
**PR:** #11 (draft/design-only); no runtime code, data migration, or side effects.
**Dependencies:** [ADR-001: persistent daemon](AGENT_DAEMON_AND_INTERFACE_CONTRACT.md), [ADR-002: Agent-pull leases](AGENT_PULL_LEASE_PROTOCOL.md).
**Source baseline:** current Danbooru/Gelbooru static media ingestion, 10 existing MCP tools, image editing, source protection, DINOv3/pHash, export contract. This document explicitly labels *proposed workflow nodes*, not existing CLI/MCP implementations.

## 1. Required execution modes

The same scheduler executes **one node** or **a full directed acyclic workflow**. They are not separate backends.

- `node.run`: one declared operation on a supplied target/scope, possibly many independent media items. It runs only that node and strictly required *side-effect-free* preconditions. If required artifacts are missing, return `PRECONDITION_MISSING` with a structured list of suggested prerequisite nodes. Never silently download, invoke a model, edit images, or export in a single-node request.
- `workflow.run`: an explicit, versioned DAG whose edges describe dependencies. It may execute an entire pipeline (and optional subflows) from start to finish. The daemon schedules local work and creates claimable Agent work; the **client Agent** pulls, inspects and submits decisions. A workflow can wait for Agent/user input and resume from its checkpoint.
- `workflow.run --only ...` or `--until ...`: proposed scoped execution of part of a DAG, using the same node contracts, with dependency verification. All implicit dependencies and side effects appear in `workflow.plan` before execution.
- `workflow.plan --dry-run`: source scope, resolved node graph, required capabilities, expected writes, provider/network/vision transmission consent, data eligibility, estimated counts/bytes when knowable and blockers. An estimate never implies task completion or measured duration.
- **Atomicity:** node invocation may generate several media work items. Each media revision is a commit boundary; a multi-image lease or whole workflow is not globally atomic. Pair decisions use an explicit two-media atomic reservation.
- **Not every node is a separate model call.** A combined Agent reasoning node can emit multiple values (caption, tags, quality, text regions and optional crop proposal) in one pass. Branching and early termination should skip irrelevant nodes.
- **No hidden workflow progression:** when a single node finishes, it does not launch successor nodes unless explicitly part of a submitted workflow; a workflow advances automatically when declared conditions pass.

Examples (all commands proposed; not yet implemented):

~~~bash
bdm workflow plan --profile curate-static --collection redum4 --json
bdm workflow run --profile curate-static --collection redum4 --input approval.json --json
bdm node run feature.extract --media-ids 123,124 --json
bdm node run annotation.validate --media-id 123 --json
bdm work lease --job JOB_ID --capabilities visual,caption --max-items 8 --json
bdm work submit --input @- --json
~~~

For an Agent-dependent node, `node.run` queues durable Agent work and returns a `job_id`; it does not infer using an unknown remote Agent or assume the calling CLI is the model. A short local read-only node may complete immediately but still returns a standard operation receipt.

## 2. Entity model and typed contracts

- **Node Definition** (`node_type@version`): reusable schema, deterministic prerequisites, executor kind, resource class, permissions, cache policy, input/output types, retry/timeout policy, data-preservation rules and optional human approval.
- **Node Run**: one instantiation with immutable input/scope/config snapshot, inputs at versioned revisions, attempt history and outcome; standalone runs are simply a one-node job.
- **Workflow Definition** (`workflow_type@version`): an immutable DAG of node definitions, guarded edges, `foreach(media)` expansions and optional joins, with explicit named outputs. Changing a profile produces a new workflow version.
- **Workflow Run / Job**: durable top-level invocation and event stream; the daemon is sole scheduler owner.
- **Work Item**: usually (node_run, media_id, media_revision); for pair review: two IDs and two expected revisions; for collection/export: scope snapshot and immutable plan. Batches are *lease envelopes* only.
- **Lease Attempt**: active Agent ownership with expiry/fencing generation/idempotency; not a permanent reservation or a database transaction.

Required Node Definition fields (proposed): `type`, `version`, `input_schema`, `output_schema`, `executor` (`local|agent|human`), `read_scopes`, `write_scopes`, `preconditions`, `resource_class`, `cache_key`, `idempotency_scope`, `retry_policy`, `timeout_policy`, `effects`, `result_states`. For image-mutating operations include expected SHA, source-retention policy, revision invalidation and audit requirements. Model evidence and subjective scores are separate from immutable source truth.

Node result states: `completed` (effects committed), `no_change` (valid nothing-to-do), `skipped` (documented condition), `awaiting_agent`, `awaiting_user`, `blocked` (missing input/permission), `retry_wait`, `failed`, `cancelled`. No silent treating of skipped/blocked images as successful media. Every result reports IDs, next actions, and reason codes.

Proposed schema snapshot (illustrative):

~~~json
{
  "node_type": "annotation.validate",
  "node_version": 1,
  "mode": "single_node",
  "scope": {"media_ids": [123]},
  "input": {"required_artifact": "current_caption"},
  "precondition_policy": "strict",
  "request_id": "caller-generated-unique-id",
  "dry_run": false
}
~~~

Return a versioned envelope with `job_id`, `node_run_id`, `state`, `completed`, `no_change`, `skipped`, `blocked`, `failed`, `next_cursor`, and per-item `reason_code`. An operation needing Agent visual reasoning returns `awaiting_agent`, **not** fake completed results.

## 3. Catalog: proposed reusable node capabilities

**Labels:** L = daemon-local deterministic work; A = client Agent vision/reasoning lease; H = explicit human approval. Some nodes are mixed, but each step has one executor. Capabilities marked proposed are *not* new implemented MCP tools.

| ID / node | Executor | Required input/precondition | Durable output | Critical guards and skip behavior |
| --- | --- | --- | --- | --- |
| N01 `collection.resolve` | L | provider, exact tag or saved collection ID | verified collection identity/count snapshot | category/auth/source failures explicit; no download side effect |
| N02 `source.discover` | L | collection/query, source adapter policy, cursor | eligible remote post references and pagination checkpoint | respect auth/rate limits, source type policy, static vs animation; no bypass |
| N03 `media.fetch` | L | remote source reference, policy, authorized credentials | validated temporary bytes + transfer receipt | bounded network/bytes/retries, resume, no raw keys in logs |
| N04 `media.verify` | L | bytes or existing managed image | actual format/static/animation check, dimensions, hashes | reject corrupt/unsupported, no upscaling, preserve original identity |
| N05 `media.import` | L | verified media and source metadata | managed media ID, immutable source associations, raw tags | exact-hash reuse, journaled file/DB commit, no duplicates |
| N06 `tags.normalize` | L | source raw tags, category rules/version | normalized versioned local tags/identity relations | protect artist, character, ambiguous tags; never rewrite raw data |
| N07 `feature.extract` | L | current image SHA and pHash/DINO config | cached feature records and compatibility metadata | single-flight, model unavailable != success, invalidate on revision |
| N08 `dedup.candidates` | L | compatible features, scope, thresholds | typed candidate pairs / similarity evidence | candidate != identity != permission to delete; cache and index |
| N09 `dedup.review_pair` | A | both media IDs/revisions, full views, candidate evidence | keep both / select survivor proposal and explanation | distinct poses/styles kept; stale pair denied; pending human gate for destructive path |
| N10 `quality.prefilter` | L | image metadata/format/size/integrity | eligibility and technical skip reasons | do not use resolution alone to infer artistic quality |
| N11 `vision.curate` | A | full-current-image view receipt, source hints, consent | explainable quality scores, caption, generic tags, text bboxes, optional crop proposal | **one Agent pass** where possible, protected identity merge local, image evidence primary |
| N12 `crop.preview` | L | current image revision + bbox; authorization to preview | native crop preview handle/fingerprint/dimensions | bbox bound to current revision, no pixel upscale or hidden save |
| N13 `crop.apply` | L | verified exact preview receipt, current SHA, scoped edit permission | new image revision/PNG, provenance/audit | align dimensions to 32, preserve verified source, invalidate affected annotations |
| N14 `image.restore` | L | verified source original, expected current SHA, edit permission | new current-image revision | destructive replacement of current edit must be explicit, not automatic |
| N15 `annotation.commit` | L | current revision + A outputs + receipt/policy | versioned TXT, tags, text regions, review state | per-image atomic/journaled write, preserve reviewed/manual text by default |
| N16 `review.route` | L/H | recorded quality/duplicate/uncertainty and policy | Keep / Defer / pending human decision | Agent reject only queues; final delete exclusively human-approved |
| N17 `export.plan` | L | immutable scope, accepted current images, destination/profile | preview: eligible/excluded/conflicts/estimated bytes | reject missing text, stale review state, unowned output and unsafe destination |
| N18 `export.write` | L | approved immutable export plan | verified image/TXT pairs, manifest, per-item receipt | no unapproved overwrite or image upscaling; safe incremental resume |
| N19 `backup.plan` | L | approved backup scope, remote config, verified local state | snapshot plan/manifest and credential-safe metadata | ModelScope remote support remains specified/unverified |
| N20 `backup.upload_verify` | L | authorized destination, snapshot plan, provider adapter | hash-verified remote version and pointer (if supported) | never advance LATEST before full readback verification; currently future implementation |
| N21 `audit.reconcile` | L | selected catalog/job/output | integrity/audit/recovery report | distinguish missing/corrupt/changed, never auto-delete to "repair" |
| N22 `image.repair` | L/A/H | selected region, explicit consent, available inpaint worker | preview and journaled image update if approved | optional/manual; a defect cannot be erased purely because model dislikes style |
| N23 `aesthetic.evaluate` | A | current-image full view, SHA/revision, aesthetic profile, client consent | separate versioned aesthetic report (overall/dimensions/confidence/evidence) | **score-only by default**, no implicit Keep/Reject, crop, caption, tag, deletion or export; see [ADR-004](AGENT_AESTHETIC_EVALUATION_WORKFLOW.md) |
| N24 `content.classify` | L/A | available source tags + optional authenticated Agent full-image view | `standalone_illustration / sequential_comic / uncertain` with evidence and version | source-tag fast path and optional fused Agent visual classification; standalone read-only; **workflow routes confirmed managed comics into existing pending deletion review** per [ADR-005](COMIC_PAGE_EXCLUSION_AND_OPTIONAL_LABELING.md) |

Notes: existing code already implements parts of ingestion, hashing, pHash/DINOv3, editing, annotations and export through other entry points. The **standalone Node API and composed workflow engine are not implemented**. Default source ingestion is **static PNG/JPEG/WebP**, explicitly excluding GIF/video/animated assets. Animated pixel dataset collection is a separate proposed opt-in media workflow requiring new adapters/validators, not a flag that silently bypasses current source-type gates.

## 4. Catalog: complete workflow templates (proposed)

A workflow is an explicit graph of nodes with configured scope and exit criteria. All templates are selectable; no Agent is forced to run the entire graph.

| Workflow | Node graph outline | Output and completion gate | Partial/single-node use |
| --- | --- | --- | --- |
| W01 Collection discovery | N01 | existing/verified collection IDs, category and source | resolve one tag only |
| W02 Download & ingest | N01? -> N02 -> N03 -> N04 -> N05 -> N06? | durable originals, raw provenance, per-post counts and cursors | fetch/verify/import standalone with explicit inputs |
| W03 Metadata cleanup | N06 -> validation | protected identity tags, normalized sidecars | normalize one image without AI |
| W04 Technical audit | N04/N10 -> N21? | pass/fail/unknown per image, no aesthetic claim | audit one media without modification |
| W05 Exact / near-duplicate review | N07 -> N08 -> N09? -> aesthetic group ranking -> N16 | **P1 accepted:** true interchangeable duplicate group retains the aesthetically best technically eligible survivor; **all other verified members enter existing pending-deletion review for human batch confirmation**, never automatically physically deleted | feature-only, candidate-only, pair review, or group-ranking-only; meaningful alternate artwork remains outside deletion-eligible groups |
| W06 Visual quality selection | N10 -> N11 (+ W16 assessment when requested/cached) -> N16? | **Aesthetic-first premium gate accepted:** clearly poor aesthetics **or** other proven training-unsuitability can enter existing pending-deletion review **only with approved style-aware policy, high-confidence evidence and explicit workflow authorization**; borderline images are deferred and excluded from premium export | standalone scoring stays non-destructive; optional W07 shares visual pass; human confirms physical deletion |
| W07 Joint visual caption + tags | N11 -> N15 | current-image-consistent caption, tags and optional text boxes | one image, one reasoning pass, no crop |
| W08 Composition / crop | N11? -> N12 -> N13 -> N11? -> N15? | current native-resolution edit and invalidation; optional post-crop labeling | preview only, apply only after preview, restore only |
| W09 Image repair | N22 -> N11? -> N15? | reviewed edited image with stale annotation repair | preview repair without commit; explicit approval |
| W10 Review & deletion confirmation | N09?/N16 -> human confirmation -> N21 | human-approved outcomes, audit and association preservation | list/reconsider one pending image; no silent delete |
| W11 Training dataset export | N17 -> N18 -> N21 | manifest-verified, resumable approved artifact set | export existing eligible content only, no inferred annotation |
| W12 Remote backup/restore | N19 -> N20 -> verify; restore as a separate gated flow | verified remotely readable snapshot and rollback-safe restore | backup-plan-only; remote side remains unverified |
| W13 Ongoing collection update | N02 delta -> N03..N06 only for new/changed -> W05? -> W07? -> W11? | incremental checkpoint; skip unchanged, no new model calls without need | new-source discovery only |
| W14 Full static-image curation | W02? -> N24 content.classify -> N16 review.route for confirmed imported comics -> W03/W04/W05/(W06+W07 fused)/W08?/W10?/W11? only on eligible images | confirmed managed comics enter **existing GUI pending deletion review** (no immediate delete), unimported tagged comics skip download, uncertain images never auto-rejected | any constituent node or subflow alone; classifier-only does not enqueue without routing |
| W15 Pixel-art motion dataset (future) | separate animated discover -> animation validation -> sampling/quality -> human/Agent annotation -> export | GIF/video frame-consistent training artifacts | intentionally not supported by current static-only ingestion |
| W16 Independent aesthetic judgment | N23 -> optional ranking or review recommendation | versioned subjective score/profile and non-destructive explanation; comic/manga narrative pages excluded in v1 | one image, media batch or full collection; skip unsupported manga pages without blocking independent nodes; can fuse W07 but persist separately |

`?` = conditional or optional and must be declared in the workflow plan. W14 is a reusable template, not a hardcoded "always execute everything" sequence. W13 may run on an explicitly requested schedule; this RFC does not create a recurring automatic download.

**Comic-review routing:** identified comics **already present in the managed library** must be queued in the **existing pending deletion review list**, with a typed comic reason and fresh SHA; no immediate file deletion. Pre-download tagged comics have no `media_id` and only enter source-skip history. `content.classify` alone is read-only; the explicitly composed `review.route` node performs the queue write. See [ADR-005](COMIC_PAGE_EXCLUSION_AND_OPTIONAL_LABELING.md).

**Implementation migration required for accepted W05 P1:** current normal DINO strict dual-signal dedup physically removes images in some workflows. The *future implementation* must replace that path with the same pending-deletion-review routing as Agent W05, then validate both code paths. The draft PR does **not** make that runtime change. Immutable raw tags, preserved source originals, human-rejected duplicate overrides, and non-overwrite of reviewed text remain invariants.

## 5. Worked example: full workflow vs just one node

**Complete curation** (e.g. `redum4`, static images): explicit job snapshots collection, source policy, filter, overwrite mode, image transmission consent, and export profile. The daemon pipelines local discovery/verification/features. Agent requests a bounded visual lease, sees images and existing annotations, reasons once for both quality and caption when valid, conditionally previews/applies crop, re-inspects changed images, commits caption/tags to final SHA, and leaves ambiguous deletion for human approval. Export may run only for approved eligible images and only if the workflow explicitly includes it.

**Single feature node** (`feature.extract` on media 123): compute/reuse pHash/DINO features for the current SHA; return feature completeness, cache state, model/version and error. Do **not** schedule caption, crop, duplicate resolution, deletion or export. If DINO runtime missing, return an explicit incomplete/degraded output; not success.

**Single Agent node** (`vision.curate` on media 123): create a one-node job awaiting Agent; Agent pulls it, retrieves the full current image, submits selected output fields. If only a quality score is requested, leave caption and tags untouched. If caption+tags are requested, validate against current SHA and identity rules; no hidden crop. Return the committed results and a compact receipt.

**Single export node** (`export.write`): fail with `PRECONDITION_MISSING` unless a valid, unexpired/export-approved plan is supplied. Never silently generate missing captions or delete an unowned destination file.

## 6. Common processing rules per flow/node

A specific flow definition is not complete unless these fields are reviewed:

| Required decision | Questions to freeze |
| --- | --- |
| Trigger & scope | user/Agent/manual/schedule; artist/character/query/media IDs; provider and media types |
| Input contract | required artifacts, SHA/revision, optional defaults, precondition behavior |
| Output contract | typed artifacts, manifest/evidence, version and success/skip/fail semantics |
| Executor & authority | local/Agent/human; read/write/approve scopes; image-provider consent |
| Dependencies & branching | which predecessor is required, conditional skip, fusion with another Agent step |
| Concurrency & fairness | per-provider network, CPU, DINO, file IO, DB writes, Agent slots, memory budget |
| Caching & invalidation | keys include current image + policy/model/profile versions, safe skip detection |
| Atomicity & side effects | per-item transaction/journal, immutable original/raw tags, review/edit overrides |
| Failure handling | retries, backoff, resumability, lease expiration, degradation, repair conditions |
| Observability | progress counted by completed stage, not cursor; failure IDs, p50/p95 benchmark |
| Acceptance tests | synthetic/fault-injection, stale writes, cross-interface parity, no accidental deletes |

Default `strict` prerequisite behavior for node-only mode avoids surprise side effects; selected composite workflows may explicitly opt into automatic prerequisite nodes with a plan and permission gate. A node that returns `blocked` must include exact missing inputs and actionable next node suggestions.

## 7. Workflow graph/rules representation (proposal)

~~~yaml
workflow: curate-static
version: 1
scope:
  collection: redum4
  media_policy: static_only
mode: workflow
nodes:
  - id: source
    type: source.discover
  - id: fetch
    type: media.fetch
    needs: [source]
  - id: validate
    type: media.verify
    needs: [fetch]
  - id: import
    type: media.import
    needs: [validate]
  - id: features
    type: feature.extract
    needs: [import]
    on_unavailable: mark_degraded
  - id: candidates
    type: dedup.candidates
    needs: [features]
  - id: curate
    type: vision.curate
    needs: [import]
    foreach: media
    condition: missing_current_valid_review
    executor: agent_pull
  - id: commit
    type: annotation.commit
    needs: [curate]
    condition: current_image_unchanged_and_output_requested
  - id: export_plan
    type: export.plan
    needs: [commit]
    enabled: false  # explicitly opt in
  - id: export
    type: export.write
    needs: [export_plan]
    enabled: false
~~~

This is a **conceptual partial DAG**, not ready for direct execution. The default full curation template must also declare conditional crop, pair review and human-review branches; never infer destructive approval from a successful analysis. A DAG with an optional disabled node must have well-defined downstream skip semantics.

## 8. Unified CLI/MCP surface for nodes and workflows

| Intent | Proposed CLI | Proposed MCP | Contract |
| --- | --- | --- | --- |
| Discover definitions | `bdm node types --json` / `bdm workflow profiles --json` | `bdm_capabilities` or a compact discover tool | versioned names, schemas, effects and permissions |
| Plan | `bdm workflow plan --input FILE` | `bdm_workflow_plan` | exact DAG, eligible counts, missing inputs, consent, risks |
| Run one node | `bdm node run TYPE --input FILE` | `bdm_node_run` | one-node job, no implicit side-effectful predecessors |
| Run workflow | `bdm workflow run --input FILE` | `bdm_workflow_run` | snapshot and durable job ID |
| Inspect progress | `bdm jobs status/events --json[l]` | `bdm_job_status` | one event cursor for node/workflow jobs |
| Claim Agent steps | `bdm work lease/heartbeat/release` | `bdm_work_lease` etc. | ADR-002 fencing, TTL and batch limits |
| Submit Agent results | `bdm work submit --input @-` | `bdm_work_submit` | one image-revision atomic result; mixed batch partial ack |
| Pause/cancel/retry | `bdm jobs pause/resume/cancel` | `bdm_job_control` | preserve committed outputs, explicit blocked reasons |

Existing ten MCP tools remain backward compatible, but a new Agent should **not** need to register all possible per-node action schemas just to run one flow. Use compact definition discovery with schema-on-demand or capability sets, and enforce scopes server-side. Keep all image model/inspection data behind consent and safe size budgets.

## 9. Review order for this PR

Review the **shared node contract and graph semantics** first, then finish each workflow from trigger to persistence with its input/output schemas, authorization, caching, scheduling and tests. Suggested sequence:

1. W02 Download/ingest (establishes canonical media IDs/provenance/sidecars).
2. W05 Duplication (exact vs near vs related and legacy auto-delete compatibility), detailed node-by-node in [ADR-006](AGENT_DUPLICATE_WORKFLOW_RFC.md). **Accepted P1:** keep the aesthetically best valid image in each true interchangeable duplicate group, queue all verified nonwinners in the existing deletion-review collection, final human batch confirmation. Pending discussion: group eligibility, tied aesthetics and differences.
3. W16 independent aesthetic assessment and W06+W07 training quality + joint caption/tag/text-region output; share model inspection when selected, but keep artifacts and mutation rights separate. **Accepted aesthetic-first W06:** apply an approved style-aware premium quality floor; confirmed poor aesthetics as well as other high-confidence unsuitability may enter existing pending-deletion review. Borderline assets defer rather than pass premium export. Independent W16 remains score-only. **Provisional bands and contract:** [ADR-007](AGENT_VISUAL_CURATOR_AND_ANNOTATION_RFC.md).
4. W08/W09 Crop/repair/restore (image revision and invalidation).
5. W10 Human review/cancellation/deletion (safety gate).
6. W11 Export (native dimensions, four optional buckets and manifest).
7. W13 Incremental collection refresh (cache reuse, updated images, completion criteria).
8. W12 Backup/restore; W15 animated dataset only after explicit scope and implementation decisions.

Aesthetic evaluation is separately documented in [ADR-004](AGENT_AESTHETIC_EVALUATION_WORKFLOW.md). **Accepted scope:** manga/comic narrative pages are excluded from automated aesthetic scoring in v1; standalone aesthetics reports a neutral unsupported skip. **When the illustration-curation workflow recognizes an already-imported comic, `review.route` queues it into the normal deletion-review collection**, and only explicit human confirmation physically deletes; this is not an aesthetics-node side effect. **W06 training-quality decisions and W16 aesthetics must not be conflated**: existing `bdm_assess` has Keep/Reject side effects, whereas N23 is score-only by default.

All node and workflow names, schemas and limits in this ADR remain proposals. Confirming "single-node OR full workflow" does not approve silent AI calls, hidden media deletion or merging the draft PR.
