# Asset library reference

`valle assets` manages a local library of media, titles, tags, annotations,
entities and analysis. Files are identified by content hashes, so identical bytes
can be reused across edits. Importing a file does not place it on a Timeline or
run semantic analysis automatically.

Read this guide with `valle docs assets`. Use the [Project reference](project.md)
for saved edits, the [Media reference](media.md) for file processing, and the
[CLI guide](cli.md) for shared output and runtime setup.

## Contents

- [First import](#first-import)
- [Import modes and content IDs](#import-modes-and-content-ids)
- [Inspect, edit and tag](#inspect-edit-and-tag)
- [Annotations and entities](#annotations-and-entities)
- [Search and use results](#search-and-use-results)
- [Explicit analysis and transcripts](#explicit-analysis-and-transcripts)
- [Remove and restore assets](#remove-and-restore-assets)
- [Maintenance and storage](#maintenance-and-storage)
- [Results and troubleshooting](#results-and-troubleshooting)

## First import

With `valle` on PATH, use a fresh demo library in a POSIX shell:

```sh
assets_demo=$(mktemp -d)
export VALLE_HOME="$assets_demo/library"
valle assets add cover.png --mode copy --title "Product cover" --tag demo --json > "$assets_demo/import.json"
asset_id=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["data"]["items"][0]["content_digest"])' "$assets_demo/import.json")
valle assets show "$asset_id" --json
valle assets annotate "$asset_id" --text "Opening product card" --tag intro --json
valle assets search "Opening" --json
valle assets resolve "$asset_id" --json
```

Supply your own `cover.png`. Python in this shell example only extracts an ID from
the JSON result; it is not a Valle runtime dependency. Check the import's exit
code and per-file results before using an ID in automation.

`VALLE_HOME` selects the application root for both assets and projects. Keep it set
for the example. Use your existing root for real work; on Windows set
`$env:VALLE_HOME` to the intended directory. Model weights live separately under
`VALLE_MODEL_CACHE` or the platform model cache.

## Import modes and content IDs

```sh
valle assets add clip.mp4 music.wav --mode copy --tag production --json
valle assets add source.mp4 --mode reference --json
valle assets add logo.png --title "Brand logo" --tag brand --tag approved --json
```

| `--mode` | Behavior |
| --- | --- |
| `reflink` (default) | Try a copy-on-write clone into managed storage, falling back to an ordinary copy |
| `copy` | Store an independent managed copy |
| `reference` | Register the existing absolute path without copying the file |

Imports take file paths; your shell expands wildcards. `--title` and repeated
`--tag` values apply to the batch's files. Media probing determines kind when
possible. The accepted `--kind` values are `video`, `audio`, `image`, `font`,
`model3d`, `lottie`, `component`, and `other`. Use an explicit kind when automatic
identification cannot recognize the input; it does not convert a file or prove
that a rendering backend supports its contents. Audio/video probing can require
FFmpeg even though semantic models are not needed.

A content digest has the form `sha256:...`. Commands accepting an asset ID also
accept an unambiguous hash prefix. Retain the returned ID; do not invent one from
a filename. Identical bytes reuse an existing ID, even under another filename.
Reimporting a removed asset with the same bytes restores its retained metadata
and knowledge. Explicitly supplied titles/tags can update metadata on reimport.

For reference imports, the original file must remain available and unchanged.
`resolve` checks its recorded size and modification time. If it was moved or
edited, reimport the actual file: identical content keeps its identity; changed
bytes have a different identity. Normal resolution does not silently accept new
bytes under the old ID.

## Inspect, edit and tag

In the commands below, replace `ASSET_ID` with an ID returned by import or search:

```sh
valle assets list --json
valle assets list --kind image --tag demo --json
valle assets show ASSET_ID --json
valle assets edit ASSET_ID --title "Approved product cover" --json
valle assets tag ASSET_ID --add approved --rm draft --json
valle assets stats --json
```

`list` defaults to live entries. `--removed` selects removed entries;
`--stale` selects references marked invalid by maintenance verification. Kind and
tag filters narrow the selection. `show` returns metadata, stored analysis and
annotations. `stats` includes counts, kind distribution, analysis coverage,
accumulated costs and stale entries.

`edit` changes title or audio subkind (`--subkind music` or `sfx`); it does not
rename or transform file contents. Use `tag` with repeated `--add` and `--rm` to
change asset-level tags. Annotation tags describe a particular note or time
window and are separate from the asset's tags.

## Annotations and entities

An annotation can describe the whole asset, one time point or one time range:

```sh
valle assets annotate ASSET_ID --text "Approved opening shot" --tag intro --json
valle assets annotate VIDEO_ID --at 1.5 --text "Product enters" --json
valle assets annotate VIDEO_ID --range 1 3 --text "Close-up of the controls" --tag detail --json
```

`--at` and `--range` are mutually exclusive. Omit both for an asset-level note.
Range syntax is **two space-separated seconds**, unlike Media's comma-separated
`--range START,END`. Supply `0 <= START < END`; time points should be non-negative.
Stored annotation times use milliseconds; search returns second-based anchors.
Provide at least one of `--text`, `--tag` or `--entity` when creating a note.

Read the annotation ID from the returned `data.annotation.id` or `show` result. To replace or
remove an existing note, using `n1` only if that is its actual ID:

```sh
valle assets annotate ASSET_ID --id n1 --text "Revised opening note" --tag intro --json
valle assets annotate ASSET_ID --rm n1 --json
```

`--id` replaces the annotation's content and scope. Resupply any range, tags or
entity links that should remain; omitted fields are not a partial-update request.
Removing a note leaves the asset itself intact.

### Name recurring people, places or products

```sh
valle assets entity add --name "Demo Camera" --kind product --alias "Camera A" --json
valle assets entity list --json
valle assets annotate ASSET_ID --entity e1 --text "Front view" --json
valle assets entity edit e1 --name "Demo Camera Pro" --alias "Camera A" --json
```

Use the actual returned entity ID instead of assuming `e1`. Entity kinds are
free-form labels. Each `--entity` in an annotation must name an existing entity;
repeat it to attach several. Renaming preserves the stable ID and its links.
Supplying aliases to `entity edit` replaces the full alias list; omit aliases to
preserve it. Names and aliases cannot collide with another entity's names/aliases.

## Search and use results

```sh
valle assets search "Opening" --tag demo --limit 10 --json
valle assets search "Camera" --entity e1 --json
valle assets search "" --kind video --filter orientation=portrait,min-dur=5,max-dur=30 --json
```

Search is local full-text retrieval over titles, tags, human notes, entities and
available analysis text. It can return an entire asset or timed evidence inside
one. It does not run an embedding model or a new analysis job. Missing semantic
analysis means less searchable evidence, not that ordinary search is unavailable.

`--filter` accepts comma-separated `key=value` pairs:

| Filter | Accepted values |
| --- | --- |
| `orientation` | `portrait` or `landscape` |
| `min-dur` | Minimum asset duration in seconds |
| `max-dur` | Maximum asset duration in seconds |

`--kind`, `--tag` and `--entity` add filters. An empty query with filters can select
matching assets without a text term. `--limit` defaults to 20 and is capped at 200.

Read `data.results` in JSON mode. A hit includes:

| Field | Meaning |
| --- | --- |
| `asset` | Asset hash usable by other asset commands |
| `unit`, `kind`, `title` | Evidence type and asset description |
| `range` | Null for a whole-asset hit, `[seconds]` for a point or `[start,end]` for an interval |
| `source`, `evidence` | Origin and text snippets supporting the match |
| `path`, `frame` | Stored media location and optional representative-frame path |
| `score` | Ranking value, not a probability or model confidence |

Resolve a selected ID immediately before using its bytes:

```sh
valle assets resolve ASSET_ID --json
```

The result's `data.path` is the resolved file path; `data.content_digest` identifies
the content and `data.location` is `cas` or `reference`. Resolution detects removed
assets, missing managed files and stale references. Use that path in a Timeline's
`resources` map, Motion's `--asset NAME=PATH`, or a Media command's input.

A search hit is not a Timeline clip. To use a timed video hit, set the clip's
`trimStart` from the source range start, choose a duration from that range, and
choose its output `start` separately. See [Timeline time mapping](timeline.md#time-trimming-and-playback).
Media output becomes a managed asset only after an explicit `assets add`.

## Explicit analysis and transcripts

```sh
valle assets analyze VIDEO_ID --with shots --events
valle assets analyze AUDIO_ID --with beats --events
valle assets analyze --kind video --tag production --with shots,beats --events
```

Supply explicit IDs or select live assets with `--all`, `--kind` or `--tag`.
`--with` is required and accepts comma-separated analyzer names. Previously
completed matching analysis is reused; `--force` recomputes it. Unsupported media
kinds are skipped and reported. A selector alone does not imply all analyzers.

| Analyzer | Input | Implementation / prerequisites |
| --- | --- | --- |
| `shots` | Video | Local image-difference detector using FFmpeg frame decoding; no downloaded shot model |
| `beats` | Audio or video with usable audio | Local audio analysis using FFmpeg decoding; no downloaded model |
| `vlm` | Images/video | Explicit external analyzer command; video shot analysis is scheduled first |

**`assets analyze --with shots` and `media shots` are different interfaces.** The
asset analyzer stores its own library analysis with millisecond intervals. Media
uses the selected OmniShotCut/TransNetV2 model and produces canonical shot-list
JSON. To feed `media interpolate --shots`, use the primary output of `media shots`,
not an asset analysis record.

Analysis results are stored in the library and become visible in `show` and
search. Inspect completion counts, `cached`, `skipped_kind`, failures and warnings;
not every selected asset necessarily ran each analyzer. `--budget AMOUNT` is a
per-run cost threshold in CNY based on costs reported by analyzers. Completed
results are kept. It is checked between jobs and cannot cap the cost of an
individual in-flight external request.

### External visual analysis

VLM analysis is available only when `VALLE_VLM_CMD` specifies an installed command
implementing Valle's analyzer JSON protocol. A command receives one JSON request
on stdin and returns one JSON response on stdout, containing structured items and
cost. This is not a service URL or arbitrary chat CLI.

`VALLE_VLM_CMD` is split on whitespace, so shell quoting and paths with spaces are
not interpreted as shell syntax. An optional `VALLE_VLM_SERVER_CMD` starts a
sidecar through `sh -c` for an explicit `--with vlm` run and stops it afterwards.
Configure that integration before requesting VLM; ordinary imports, tags and
search do not start it. For adapter authors, the exact request/response contract is
in [the VLM adapter](../crates/valle-project/src/assets/vlm.rs) and
[external transport](../crates/valle-project/src/assets/transport.rs).

### Existing transcripts

```sh
valle assets transcript ASSET_ID --level word --json
valle assets transcript ASSET_ID --level sentence --json
```

`--level` defaults to `sentence`; accepted values are `word` and `sentence`.
This command reads an existing stored `asr@1` analysis slot and returns text plus
second-based segments. It does not run speech recognition. The current asset
analyzer registry has no ASR runner. Use `media transcribe FILE` for a new
standalone transcript; it is not automatically attached to the asset, and the
current CLI has no transcript-import subcommand. See `valle docs media` for ASR
models and output formats.

## Remove and restore assets

```sh
valle assets remove ASSET_ID --json
valle assets list --removed --json
valle assets add original.png --mode copy --json
```

Normal removal drops library locations and removes managed bytes when they are
not retained by an active or pinned resource package. It keeps metadata, analysis
and annotations, while excluding the removed entry from ordinary list/search.
Removing an already removed asset is harmless. For a reference import, removal
unregisters the location and does not delete the external source file.

Reimporting the same bytes restores the asset with the same ID and retained
knowledge. It cannot reconstruct deleted bytes without a source copy. Files used
by your Timeline/Project must remain available; saving a project document does
not automatically retain every file it references.

`assets remove ASSET_ID --purge` additionally removes metadata, analysis and
annotations. Purging annotated assets requires `--force`; it permits deletion of
those human annotations. Use ordinary removal when the knowledge should remain.

## Maintenance and storage

The library lives under `$VALLE_HOME/assets` (normally `~/.valle/assets`). Managed
objects, metadata, analysis, annotations and entities live there. `index.db` is
a derived search/index database, rebuilt from the authoritative stored records.
Keep referenced external files separately when moving or backing up a library.

```sh
valle assets maintenance verify --json
valle assets maintenance verify --deep --json
valle assets maintenance reindex --json
valle assets maintenance sql "SELECT kind, COUNT(*) AS count FROM assets GROUP BY kind" --json
```

| Maintenance command | Effect |
| --- | --- |
| `verify` | Inspect managed files, reference validity and drift; update stale-reference status |
| `verify --deep` | Also recompute hashes; useful when checking actual content integrity |
| `reindex` | Rebuild the disposable index from metadata, analysis, annotations and entities |
| `sql QUERY` | Read-only SQL inspection; an authorizer enforces SELECT-only access |
| `gc` | Delete orphaned managed blobs and temporary writes while respecting resource retention |

Verification does not repair missing source files. Reindexing does not rerun
models or recover deleted bytes. `gc` is a cleanup operation that deletes files;
it is not needed for normal import/search, and does not replace ordinary asset
removal. Prefer public commands over directly editing the on-disk store.

## Results and troubleshooting

Assets uses `{ "ok", "data", "error", "warnings" }`, not the Media run envelope.
Use `--json` or `--events` and check both the process exit code and relevant result
fields. Batch import can contain successful and failed items in the same report;
inspect `data.items` and `data.failed`. Successful batch items have
`content_digest`; a batch with failures exits nonzero while preserving successes.

| Command | Useful JSON fields under `data` |
| --- | --- |
| `add` | `items`, `failed`; per-item content identity, reuse/revival and errors |
| `list` | `assets`, `count` |
| `show` | `meta`, `analysis`, `annotations` |
| `resolve` | `content_digest`, `path`, `location` |
| `search` | `query`, `results`, `count` |
| `annotate` | `annotation` for creation/replacement, or `removed` for deletion |
| `entity list` | `entities`, `count` |

Argument parsing errors normally exit 2. Asset command failures and failed import
batches exit 1, including library-level validation/dependency failures; do not
apply Media's exit-3/4 meanings to all asset errors. Read the error and hint. The
[CLI output contract](cli.md#output-contract-for-scripts-and-agents) explains event framing.

| Symptom | What to check |
| --- | --- |
| Asset ID not found or ambiguous | `VALLE_HOME`, full content digest or a longer unique prefix |
| Reference is stale | Original path, size and modification time; reimport the current file |
| Asset disappears from ordinary list/search | Check `list --removed`; a removed entry retains knowledge |
| Search misses an item | Title/tags/annotations, query filters, stored analysis and index state |
| Analyzer is not registered | Use supported `--with` names; configure VLM explicitly; ASR uses `media transcribe` |
| Transcript unavailable | A stored ASR slot is required; standalone Media output is not attached automatically |
| Managed file is missing | Run maintenance verification and reimport the same bytes from a source copy |
| Purge refused | Annotated assets require explicit `--force` to delete their knowledge |
