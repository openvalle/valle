# Asset library reference

The asset library is a collection of reusable files at `$VALLE_HOME/assets`, normally
`~/.valle/assets`. Files have ordinary names and folders. Commands use library-relative
paths such as `sfx/whoosh.wav`; results also include `absolutePath` for `project add`.

```sh
valle assets add whoosh.wav --to sfx --tag 转场 --note "短促的风声" --json
valle assets search 风声 --kind audio --json
valle assets show sfx/whoosh.wav --json
valle project add ~/.valle/assets/sfx/whoosh.wav -C promo --json
```

Project imports are independent copies. Library notes help select a clip or trim point;
they are not copied into the project. Motion's `asset://` slots remain component parameters.

## Files and organization

`add` clones files on supported filesystems and otherwise copies them. Imports are independent
files, never hard links. Default folders are `fonts/`, `music/`, `videos/`, `images/`, `3d/`,
`shaders/`, `components/`, `data/`, and `other/`. Audio includes music and sound effects;
use `--to sfx` for sound effects. Directories are created as needed and have no special meaning.
Use `--to brand/acme` or organize files in Finder yourself.

```sh
valle assets add sound-pack --to sfx --tag licensed --json
valle assets add cover.png --name hero.png --to brand/acme --json
valle assets edit brand/acme/hero.png --name poster.png --to images --json
```

Directory imports retain the source directory name and its subdirectories. For example,
`sound-pack/whoosh/short.wav` becomes `sfx/sound-pack/whoosh/short.wav` with `--to sfx`.
`--name` accepts exactly one file. Component imports use the Motion compiler's import parser;
a `.motion.tsx` importing other local modules is rejected before copying. Copy a multi-file
component tree manually into a project, preserving relative imports.

Repeated imports of the same content return an existing file and merge tags and notes.
Different content with the same filename gets `-2`, `-3`, etc.; comparisons ignore case.
An explicit `--name`, rename, or move that collides reports an error. Different folders may
contain the same filename. Filenames retain spaces and colons on filesystems that support them.
Types come from extensions and file signatures, independently of the directory. Unrecognized
files can still be stored, tagged, annotated, and searched. Unsupported or unreadable media
facts are `null` and do not prevent keeping the file.

A batch returns an item for each success or failure. Any failure produces `status: partial`
and a nonzero exit code; successful imports remain. Retry the same add to fill in failures
without duplicating files or identical notes.

## Find files and edit knowledge

```sh
valle assets list --kind font --json
valle assets list brand/acme --tag approved --json
valle assets search "轻快音乐" --kind audio --filter min-dur=30,max-dur=120 --json
valle assets search opening --in videos --filter orientation=portrait --json
valle assets tag music/upbeat.mp3 --add energetic --rm draft --json
valle assets annotate music/upbeat.mp3 --at 45 --text "进高潮" --json
valle assets annotate videos/take.mp4 --range 1,3 --text "可用片段" --json
valle assets annotate videos/take.mp4 --id 2 --text "更新的备注" --json
valle assets annotate videos/take.mp4 --rm 2 --json
```

Search covers filenames, directories, tags, notes, and font family names. It ranks matches
using local FTS5, including normalized Chinese and Japanese bigrams. Search never runs a model.
Filters support `min-dur`, `max-dur` (seconds), and `orientation=portrait|landscape|square`.
`--limit` defaults to 50 and is capped at 1000. Lists return up to 1000 items with a total count.
Notes have text and optional seconds (`--at` or `--range START,END`). Note IDs are numbers
from `show`; `--id` replaces a note and `--rm` deletes one. Identical text and times are deduplicated.
Tags belong to files, not notes. For transcription, shot detection, or other analysis, use
[media tools](media.md) on the file's absolute path.

Every command scans visible library files first. Unchanged files reuse their digest and facts.
New files are indexed automatically; moved files retain tags and notes when file identity and
content match, or when a missing file has a unique matching digest. Separate duplicate files
remain separate entries. Ambiguous moves keep old notes on the missing entry.
Files replaced at the same path retain that entry's notes, like an ordinary overwrite save.

```sh
valle assets list --missing --json
valle assets remove videos/take.mp4 --json
valle assets remove --missing --json
valle assets sql 'SELECT path, kind, missing FROM assets' --json
```

Missing files retain their tags and notes and are excluded from normal list/search results.
`remove` deletes the entry, tags, and notes, then moves the file to system Trash.
`remove --missing` deletes all missing entries after making a backup. Inspect the current state
with `show` before retrying an interrupted edit or remove. SQL is read-only; tables include
`assets`, `tags`, `notes`, and derived `facts` and `asset_search`. SQL mutations, attachments,
and pragmas are refused.

## Database safety and recovery

`.valle/catalog.db` owns file entries, tags, and notes. It is primary data, not a disposable
index. Derived media facts and full-text search may be rebuilt, but user tables are never
automatically dropped or replaced. An unreadable or unsupported catalog stops commands with
a recovery hint. A missing catalog with existing backups is not silently recreated.

Before the day's first write, SQLite's backup API creates a dated standalone database in
`.valle/backups/`. Bulk missing-entry removal also creates a backup. The newest seven backup
files are retained. Move the whole library, including `.valle/`, when changing machines; do
not copy it while commands are writing. Hidden files/directories and symlinks are not scanned.

```sh
valle assets recover --json
valle assets recover 2026-10-24.db --json
```

Recovery can list backups even if the current database is corrupt. **Close Studio and other
library commands before restoring.** The selected backup is validated first. Recovery then
archives the current `catalog.db`, `-wal`, and `-shm` as raw files under a unique
`backups/replaced-…/` directory, installs a standalone copy, and scans normally. Replaced
catalogs are also listed and can be selected by name; they are not automatically deleted.
A bad backup leaves the current catalog in place. If interrupted, rerun recovery.
Files overwritten at the same path since the backup can inherit old notes; review them.
Older notes outside the retained backups need your own whole-library backup.

File writes use temporary files and exclusive publication, and database writes use SQLite
transactions. Add publishes the file before saving metadata; retrying add merges omitted
knowledge. Edit moves the file before updating its entry, so scanning recognizes the move.
Remove deletes metadata before trashing; a leftover file is scanned as a fresh entry without
old notes. No additional filesystem lock or operation journal is used.

Old content-addressed libraries are not migrated. If an old `index.db` or object tree is
present, move the old library aside before using the new one. URL downloads, thumbnails, and
waveforms live under `~/.valle/cache/{url,thumbs,waveforms}`; `VALLE_CACHE_DIR` overrides the
cache root. `VALLE_HOME` otherwise takes precedence over `HOME` or `USERPROFILE`. Model
weight locations are unchanged.
