# Project editing reference

`valle project` saves a Timeline as a sequence of immutable revisions. Use it when
an edit needs history, concurrent-save checks, restoration, or a persistent Studio
session. Use `valle timeline` to check or render an ordinary JSON file directly.

Read this guide offline with `valle docs project`. The [Timeline reference](timeline.md)
(`valle docs timeline`) defines the document itself; the [CLI guide](cli.md)
covers shared output modes, installation and runtime setup.

## Contents

- [First project](#first-project)
- [Commands and identifiers](#commands-and-identifiers)
- [Read and save a complete timeline](#read-and-save-a-complete-timeline)
- [History and restore](#history-and-restore)
- [Resources and storage](#resources-and-storage)
- [Preview and render](#preview-and-render)
- [Results and troubleshooting](#results-and-troubleshooting)

## First project

With `valle` on PATH, use a fresh demo store in a POSIX shell:

```sh
project_demo=$(mktemp -d)
export VALLE_HOME="$project_demo/library"
```

`VALLE_HOME` selects both the project store and asset library. Keep it set while
running this example; use your existing value when working on a real project.
The Windows equivalent is setting `$env:VALLE_HOME` to the intended directory.

Save the following as `blue.timeline.json` in your working directory. It needs no
external media or models:

```json
{
  "canvas": { "width": 640, "height": 360, "fps": 30 },
  "tracks": { "visual": [{ "clips": [
    { "kind": "solid", "color": "#2563eb", "start": 0, "duration": 3 }
  ] }] }
}
```

```sh
valle project create demo --timeline blue.timeline.json --intent "Initial blue card" --json
valle project show demo -o edit.timeline.json --json
```

Creation writes revision 1. `show` returns the current snapshot and writes just
its Timeline to the new `edit.timeline.json` file. Change that file's color to
`#0d9488`, keeping the complete document, then submit it:

```sh
valle project apply demo --base-revision 1 --timeline edit.timeline.json --intent "Use teal" --json
valle project history demo --json
valle project render demo --frame 0 -o teal.png --json
```

The changed document creates revision 2. The PNG should show the teal card. Saving
an identical document returns `unchanged` and keeps the current revision number.
For existing projects, read and use the returned revision instead of assuming it
is 1 or 2.

## Commands and identifiers

| Command | Required input | Result / behavior |
| --- | --- | --- |
| `create PROJECT_ID --timeline FILE` | New ID and complete Timeline | Initial snapshot at revision 1 |
| `show PROJECT_ID` | Existing ID | HEAD snapshot, or `--revision N`; optional `-o FILE` exports Timeline JSON |
| `apply PROJECT_ID --base-revision N --timeline FILE` | Revision originally read and complete edited Timeline | Commit, unchanged, stale base or validation rejection |
| `history PROJECT_ID` | Existing ID | Newest-first revision page; optional `--cursor N`, `--limit N` |
| `restore PROJECT_ID --base-revision N --revision OLD` | Current revision and revision to restore | Append the old content as a new revision, or report unchanged/stale base |
| `studio PROJECT_ID` | Existing ID | Local Studio URL; optional initial `--revision N`, `--port N` |
| `render PROJECT_ID -o FILE` | Existing ID and new output path | Render HEAD, or `--revision N`; `--frame N` selects a PNG |

Project IDs contain 1–128 ASCII letters, digits, hyphens or underscores. An ID is
not a directory path; spaces, slashes and non-ASCII characters are rejected.
`create` does not overwrite an existing project.

Revisions are project-local integers from 1 through 9007199254740991. They count
saved changes, independently of the Valle executable version. Public Timeline
JSON has no root `version` or `revision` field.

`--intent TEXT` is available on `create`, `apply` and `restore`. It records a short
explanation in history; it does not direct an AI edit or modify the document.
The current public CLI has no project listing, rename, delete, branching or
clip-by-clip patch commands.

## Read and save a complete timeline

`show --json` returns three fields:

| Field | Meaning |
| --- | --- |
| `projectId` | Project being read |
| `revision` | Snapshot revision to use as the base of a subsequent edit |
| `timeline` | Complete public Timeline document |

`show -o FILE` writes only `timeline`, while stdout still contains the snapshot.
The output file must be new; there is no overwrite flag. Edit the exported file,
or extract `timeline` from the JSON result. Do not pass the snapshot envelope as
`apply --timeline` input.

An edit replaces the complete document against a known base:

```sh
valle project show demo -o next.timeline.json --json
# Edit next.timeline.json, preserving the other tracks and resources.
valle project apply demo --base-revision 2 --timeline next.timeline.json --json
```

Here 2 assumes HEAD is still revision 2 from the example. There is no implicit
merge: dropping a track or resource from the submitted document drops it from the
new snapshot. Preserve fields unrelated to the requested edit.

### Handle the save result

| `outcome` | Exit | Meaning and next step |
| --- | --- | --- |
| `committed` | 0 | A new revision was saved; use returned `revision` for the next edit |
| `unchanged` | 0 | Document equals HEAD; no revision appended, even if intent differs |
| `staleBase` | 1 | Someone saved since your base; returned `revision` is current HEAD |
| `rejected` | 1 | Validation failed; inspect `errors` and correct the document |

Examples of the compact response shapes:

```json
{ "outcome": "committed", "revision": 2 }
```

```json
{ "outcome": "staleBase", "revision": 3 }
```

On `staleBase`, fetch the latest snapshot, reconcile your intended change with the
new content, and submit that complete document against its revision. Merely
changing `--base-revision` on an old full document can erase another editor's work.
A rejected save does not advance HEAD. Error entries include `code`, `path` and
optional `details`; paths are JSON Pointers rooted at the edit request, commonly
under `/timeline`. Invalid CLI arguments and unreadable/malformed input files can
fail before an `outcome` is available.

Project saving validates and compiles the Timeline authoring document. It does
not decode every referenced file, run every Motion frame or test an encoder.
Use `timeline check` on the candidate file and render a representative frame when
validating an edit that changes resources or visual behavior.

## History and restore

```sh
valle project history demo --limit 1 --json
valle project history demo --cursor 2 --limit 1 --json
valle project show demo --revision 1 --json
```

History returns `projectId`, `revisions` and `nextCursor`. Entries include revision,
parent revision, creation time, actor, cause and optional intent. Pages run from
newest to oldest; `--cursor N` continues **after** revision N. Pass the returned
`nextCursor` unchanged for the next page, stopping when it is null. `--limit`
defaults to 50 and must be positive.

To restore blue after the example's teal edit:

```sh
valle project restore demo --base-revision 2 --revision 1 --intent "Return to blue" --json
valle project show demo --json
```

If HEAD was revision 2, this creates revision 3 with revision 1's content. Earlier
revisions remain readable. Restoring content already equal to HEAD returns
`unchanged`. Restore uses the same stale-base protection as apply.

## Resources and storage

Project history lives under `$VALLE_HOME/projects` (normally `~/.valle/projects`).
Keep using the same application root to find previously created projects. Models
use a separate cache selected by `VALLE_MODEL_CACHE`.

On `create` and `apply`, relative entries in the Timeline's `resources` map become
absolute paths based on the submitted JSON file's directory. URI locators and
absolute paths are preserved. Moving to another working directory therefore does
not change the meaning of the saved local resource paths.

**A saved project revision versions the Timeline document, not all external file
contents.** Creating a project does not copy its footage, fonts or Motion sources
into a self-contained archive. Referenced files must remain available; changing a
file in place can change the result of rendering an old revision. Exporting
`show -o FILE` likewise does not bundle media. When moving a project to another
machine, provide the resources and update their locators deliberately.

For managed media, use `assets add --mode copy`, then `assets resolve ID --json`
and put `data.path` in the Timeline's resource map. A stable content ID and the
resolved file path serve different purposes. See the [asset library reference](assets.md)
(`valle docs assets`) for reference validity and removal behavior.

## Preview and render

```sh
valle project render demo --revision 1 --frame 0 -o original.png --json
valle project render demo -o current.mp4 --events
valle project studio demo --port 0 --events
```

`render` without `--revision` reads current HEAD; specify a revision to choose a
historical document explicitly. Frame indices are zero-based and must fall inside
the document's duration. `--frame` requires PNG; full video delivery requires MP4.
Output paths must be new files.

Dimensions, frame rate, duration and background come from the Timeline. Project
render uses Native Raster and does not expose Motion's `--backend`, `--size`,
`--duration`, `--workers` or encoder-tuning options. MP4 requires an opaque canvas
background and compatible FFmpeg libraries/encoders; decoding footage also needs
FFmpeg. A self-contained solid-color PNG render needs no model or FFmpeg library.

Studio is a persistent local process. `--port` defaults to 9527; 0 chooses a free
loopback port. Use the returned URL, and stop with Ctrl-C. Its readiness result
includes `projectId`, the initial `revision`, URL, port and Web runtime identity.
`--revision N` chooses the initial snapshot; saving still obeys the current HEAD's
base-revision check. Opening a historical revision does not reset project history.
The packaged CLI supplies the Web runtime; `--web-assets-dir PATH` is an explicit
runtime override and must match the CLI's runtime contract.

## Results and troubleshooting

Use `--json` for one result or `--events` for progress and a final report. Studio
emits readiness and continues serving. See the [shared output contract](cli.md#output-contract-for-scripts-and-agents)
for transport and exit codes. A snapshot/history response has no `status` field;
check the process exit code. Save responses use `outcome`; render responses use
`status: "ok"`.

| Symptom | What to check |
| --- | --- |
| Project not found | Project ID and `VALLE_HOME`; a different root is a different store |
| Export file already exists | Choose a new `show -o` or render output path |
| Save returned unchanged | The complete parsed document equals HEAD; an intent change alone does not create a revision |
| Save returned staleBase | Re-read HEAD and reconcile the requested change before retrying |
| Save returned rejected | Follow the error paths and codes in the result |
| Saved document fails during rendering | Resource availability, source ranges, Motion bindings and runtime/encoder setup |
| Old revision renders differently | External files may have changed; history stores their locators, not immutable copies |
| Studio appears to hang after readiness | It is serving the local UI; use its URL or stop it with Ctrl-C |
