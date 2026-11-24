# Media tools

`valle media` works on media files by themselves, apart from timelines and projects: what is
said in a video or sound file, with each word's time. Its results are files to read and use
in a timeline as the edit needs.

```sh
valle media model list                                     # what is installed, and where
valle media model install qwen3-asr-0.6b qwen3-forced-aligner-0.6b
valle media transcribe interview.mp4 -o interview.transcript.json
```

## Models

The tools run models on the CPU, installed once on the machine. Nothing is downloaded until
`valle media model install` is run: a command that needs a model that is not installed fails
with code `model`, and its `hint` is the command that installs it.

| Model | What it is for | Size |
| --- | --- | --- |
| `qwen3-asr-0.6b` | Speech to text, in the language given or the one it detects | 1.88 GB |
| `qwen3-forced-aligner-0.6b` | Each word's time in the text | 1.84 GB |

Both are Qwen's, under Apache-2.0, at the revisions this Valle was tested with.

| Command | What it does |
| --- | --- |
| `valle media model list` | Each model's purpose, size, and whether it is installed and where |
| `valle media model install MODEL…` | Downloads the models, by default from Hugging Face |
| `valle media model install MODEL --source S` | From `huggingface`, `hf-mirror` or `modelscope` |
| `valle media model install MODEL --from FOLDER` | From a folder that holds the model's files |

Models are kept in `~/.valle/models`, or the folder `VALLE_MODELS_DIR` names, one folder for
each revision of each model; delete a model's folder to remove it. Every file is checked
against its SHA-256 before it is installed, wherever it came from.

A download that stops, or fails, picks up where it was when the same command is run again.
`huggingface` downloads from `HF_ENDPOINT` when it is set, as Hugging Face's own tools do.
`modelscope` serves the same files, but only its newest: if they ever change there, the
check fails and nothing is installed.

`--from` takes a folder of a model downloaded before, such as a Hugging Face snapshot
(`~/.cache/huggingface/hub/models--Qwen--Qwen3-ASR-0.6B/snapshots/<revision>`) or a
ModelScope one. Its files are linked in where the disk allows, so they take no more space,
and copied where it does not. A linked file is shared: do not change it in place.

## Transcribing

`valle media transcribe FILE` reads the sound of any file FFmpeg reads, so FFmpeg must be
installed as for rendering video (`valle docs cli --section ffmpeg`).

| Option | What it does |
| --- | --- |
| `-o OUT` | Writes the transcript to a `.json` file; without it, the result is the transcript |
| `--language L` | The language spoken, as an English name or ISO code: `zh`, `English`. Without it, each part's is detected |
| `--text-only` | The text without word times; needs only `qwen3-asr-0.6b` |

```json
{
  "model": "qwen3-asr-0.6b",
  "language": "English",
  "duration": 12.48,
  "text": "Welcome back. Today we cut the trailer.",
  "segments": [
    {
      "start": 0.32, "end": 4.1, "language": "English",
      "text": "Welcome back. Today we cut the trailer.",
      "words": [
        { "text": "Welcome", "start": 0.32, "end": 0.71 },
        { "text": "back", "start": 0.71, "end": 1.05 }
      ]
    }
  ]
}
```

- Times are seconds into the file.
- A segment is at most 30 seconds, cut at a pause near its end where there is one; it is a
  unit of recognition, not a caption.
- Words carry no punctuation; the segment's `text` has it. Chinese and Cantonese words are
  single characters.
- Word times are given in Chinese, Cantonese, English, French, German, Spanish, Italian,
  Portuguese and Russian. Other languages fail with code `media`; transcribe them with
  `--text-only`.
- Long files take a while: on an Apple silicon laptop, about a third of their length, and
  longer on older CPUs. Memory is about 3.5 GB with `--text-only` and 5 GB with word times.

## Captions from a transcript

A caption track takes the transcript's words as timed `runs`
(`valle docs timeline --section captions`). For a clip of the file:

1. Break the words into captions where the pause to the next word is over 0.5 s, and before
   a caption passes about 42 characters (16 in Chinese) or 6 seconds. Words carry no
   punctuation, as Chinese captions are written; where a caption wants it, the segment's
   `text` has it.
2. Keep the words inside the part of the file the clip plays, `trimStart` to
   `trimStart + duration × rate`, and turn their times into the clip's:
   `(time − trimStart) / rate`.
3. Each caption starts at its first word, anchored to the clip so it moves with it, and its
   runs' times are from its start:

```json
{ "start": { "clip": "interview", "offset": 0.32 }, "duration": 3.8, "runs": [
  { "text": "Welcome ", "start": 0, "end": 0.39 },
  { "text": "back.", "start": 0.39, "end": 0.73 }
] }
```

Read the text before it is shown: names and terms can be misheard. A clip with a `timeMap`
plays its source unevenly; place its captions by the map, or ask the user.
