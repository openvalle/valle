# Project folders

A Valle project is an ordinary directory. Edit its Timeline and Motion files directly;
`valle project` imports inputs, checks the edit, saves source snapshots, restores them,
renders outputs and opens Studio. Read this guide offline with `valle docs project`.

## Create and edit

```sh
valle project create promo --width 1920 --height 1080 --fps 30 --json
cd promo
valle project add /path/to/title.motion.tsx /path/to/footage.mp4 --json
```

The default layout is:

```text
promo/
├── project.json       {"name":"promo","timeline":"timeline.json"}
├── timeline.json      Canvas, frame rate, resources and tracks
├── components/        Imported .motion.tsx components
├── media/             Video, audio, images, fonts, data and shaders
├── renders/           Rendered outputs
└── .valle/history/    Source snapshots
```

`project.json` only names the project and its entry Timeline. The directories are defaults;
Timeline resources can reference other local paths or supported URLs. Relative resource
paths resolve from the Timeline's directory. Local names may contain spaces and colons.

Commands find `project.json` by walking up from the current directory. Use
`valle project show -C /path/to/promo` to select another project.

`add` copies each input into the project, using an independent filesystem clone when
available and a regular copy otherwise. Changing or deleting the original does not change
the project. `--link` returns the original's absolute path instead of making a copy.

The result includes a path, suggested alias and `resource` object. Add those entries to
the Timeline's `resources`, then create clips that reference the aliases. **`add` does not
edit the Timeline.** For example:

```json
{
  "canvas": {"width":640,"height":360,"fps":30},
  "resources": {"footage":"media/footage.mp4"},
  "tracks": {"visual":[{"clips":[
    {"kind":"video","src":"footage","start":0,"duration":3}
  ]}]}
}
```

Equal content reuses the existing file. Different content with the same filename gets
`-2`, `-3`, and so on; collisions are checked without regard to case. Batch imports return
one result per input. `status: "partial"` and a nonzero exit code mean some inputs failed;
successful copies remain available and are reused on retry.

Components importing other local modules are rejected before copying. Copy those modules
and the entry component into `components/` together, preserving their relative layout.
The compiler and snapshots support their relative imports.

## Commands

| Command | Behavior |
| --- | --- |
| `project create DIR` | Create an empty project; optional `--width`, `--height`, `--fps` |
| `project show` | Show canvas, tracks, resources, external references, history and latest export |
| `project add FILE… [--link]` | Import files and return resource suggestions |
| `project check` | Validate the entry Timeline and report missing and external inputs |
| `project save [-m MESSAGE]` | Save a source snapshot; unchanged inputs reuse the last snapshot |
| `project history` | List snapshots with their numbers, timestamps and messages |
| `project restore N` | Back up files that will be overwritten, then restore snapshot N |
| `project render [-o PATH]` | Render the entry Timeline; default output is in `renders/` |
| `project studio` | Edit the entry Timeline in Studio; optional `--port`, `--web-assets-dir` |

All commands support `--json` or `--events`. Successful finite results use `status: "ok"`;
errors include a code, message and next-step hint. There is no apply/commit protocol:
editing the files changes the project immediately.

## Snapshots and restore

```sh
valle project save -m "First cut" --json
valle project history --json
valle project restore 1 --json
```

A snapshot contains the inputs actually used by the entry Timeline: the project marker,
Timeline, reachable component modules, data and shader sources. Binary media, images,
fonts and 3D inputs are recorded by content digest. Unreferenced drafts are not included;
use Git to preserve every file.

Restore first backs up the current contents of every source it will overwrite, including
files no longer referenced by the current Timeline. It can therefore restore an earlier
snapshot even when the current Timeline is broken. Files outside the project, symbolic
links and media are not overwritten; differences are reported. New files are not deleted.

If restore stops halfway, rerun it. Its original preimages remain in the first backup
snapshot, listed by `history`. Operations use the project lock only while writing, and
files are published atomically. Direct edits by external tools remain possible: when
Studio is open, reread a file before changing it. Studio retains the existing Timeline
and source conflict checks.

## Studio

`project studio` opens the entry Timeline at the server root `/`. Outside a project it
opens a home page with recent projects, a directory field for opening a project, and
project creation with canvas settings. Home can open projects for as long as its server
process is running; Ctrl-C stops those local Studio windows together.

The Assets panel browses project files and the global library by directory and type.
It supports local search, library tags and timed notes, image/video/audio preview, and
font samples. Timed library notes seek the preview; notes are not copied into projects.
Drop local files into Assets to import them, or drag local/library/project files onto
the Timeline to import and create resources and clips. Non-clip types are added as resources.

A Timeline drop first imports every file. If any import fails, the Timeline is unchanged;
successful copies remain available for retry. A successful batch is validated as one draft
and saved with the existing Timeline save protocol. Save conflicts preserve the draft.
Undo affects the Timeline and leaves copied files in the project.

History lists snapshots, saves the current files, and restores an earlier source snapshot.
Exports lists `renders/` and export records and starts the same native render as the CLI.
These operations save a dirty draft first; a conflict must be resolved before proceeding.
Only the entry Timeline has History and project Exports. Other Timeline files in a project
show its name and Assets and retain their browser file export.

Studio watches referenced modules, data, shaders and media using file metadata. A clean
workspace reloads automatically; an external change while a draft is dirty shows a conflict
and preserves the draft. Reopen or explicitly reload after deciding which version to keep.
Motion's **Save as project** creates a new directory, copies its component and media,
and opens the new entry Timeline in a normal file session. Multi-file components still
need manual copying into a project, and extra `--font` stacks must first become declared assets.

## Render and move

```sh
valle project check --json
valle project render --events
valle project render --frame 0 -o renders/frame.png --json
valle project render -o 'renders/%04d.png' --json
valle project studio --port 0 --events
```

Render accepts the Timeline delivery options: video codecs, individual frames, frame
sequences and storyboards. Full exports automatically save or reuse a source snapshot;
each export has its own record under that snapshot's `exports/` directory. The record
names output and input digests, parameters, warnings and the Valle executable version.
Preview frames and storyboards do not create snapshots.

Rendering and history use the same captured inputs. If original media changes during
rendering, the result and export record warn about it; the correctly rendered captured
copy is still delivered. Snapshots guarantee traceability, while complete reproduction
requires the original media bytes. A killed process can leave temporary outputs or a
record whose output was never published; these are safe to remove manually.

Projects using relative paths and copied inputs can be moved or archived as directories.
`check` lists external references that still depend on their original locations. Include
`.valle/` when preserving history; ignore `.valle/` and `renders/` when using Git for source
control. `VALLE_HOME` selects the global asset library and recent-project list, not the
project's location.
