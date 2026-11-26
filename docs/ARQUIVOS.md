# POP Harness files (`crates/poph-files`)

Receiving files from the customer (HR-23): validates them, reads documents and writes the org's blob. No HTTP,
no real LLM, and no `attachments` table (that is HR-24). No `unsafe` in this crate.

## What the crate does

1. **Size:** rejects anything above 16 MB (configurable, up to 256 MB) before anything else. Nothing is written.
   Exactly 16 MB passes this check; the type decides after it.
2. **Real type:** decides by content (file signature), not by name. The name is only used for two things: rejecting
   program or script extensions and catching a misleading file.
3. **Accepted:** JPEG, PNG, WebP and GIF (sent to the LLM as images); PDF, Word (`.docx`), Excel (`.xlsx` and `.xls`)
   and PowerPoint (`.pptx`) (turned into text).
4. **Rejected with a message:** executable programs, scripts, audio, video, old Word and PowerPoint (`.doc` and
   `.ppt`), compressed archives (zip, rar, etc.), plain text, unknown formats.
5. **Isolated reading:** PDF and Office files are read in a separate process (`poph-extrator`), with a memory
   ceiling and a time limit (see below).
6. **Text:** cut at 20,000 characters, with a warning to the LLM when it is cut.
7. **PDF with no text** (scanned): does not invent content; the LLM receives the instruction to tell the customer it
   could not read the file.
8. **Blob:** written to `<blobs folder>/<org id>/<sha256>`, once per content. Repeating the same file reuses the blob.
   The name the customer sent never becomes a disk path. Each org has its own folder: another org never reuses
   a blob, even with the same content, and it does not appear as "already existed" for it.

## How the text reaches the LLM

All extracted text goes into an **untrusted block**:

```
<<ARQUIVO_NAO_CONFIAVEL>>
...file text...
<<FIM_ARQUIVO_NAO_CONFIAVEL>>
```

Before the block comes a notice from us (trusted text) saying that this is data, not instructions.
The file text cannot close the block: the sequence `<<` inside it becomes `‹‹`.
`ArquivoRecebido::para_llm()` returns the parts in order: notice, (truncation notice, if any), block.
Images go out as image parts.

**Attention:** the turn engine's composer (HR-13) must use the same markers and treat the block as data. This crate
does not know the composer (which is still in `poph-core`).

## Isolated reading (why a process and not a thread)

- The main process calls `poph-extrator <formato> <limite de caracteres> <teto de memória>`, sends the bytes through
  standard input and reads one JSON line from standard output.
- Inside the child, the first thing is the memory ceiling (RLIMIT_AS, default 512 MB) and turning off the core dump,
  so that a crash does not write the document's content to disk.
- The parent controls the time limit (default 5 s) and kills the child if it goes over. The child's environment is
  empty, and the child's error is not sent to the log.
- Measured in the spike (HR-02): a 149 KB PDF reached 2.5 GB of memory. With the 512 MB ceiling, the child is stopped
  by the system in ~0.4 s (release build); the main process carries on.
- Zip bomb (Word/Excel/PowerPoint): if any uncompressed XML block exceeds 32 MB or the total exceeds 64 MB, the read is
  refused before decompressing (tested with the 200 MB fixture, in ~3 ms).

## Dependencies with a reason

- **PDF, Excel and Office readers:** chosen in the HR-02 spike (`RELATORIO_HR02.md` §1 and §5). Do not replace them
  without consulting the maintainer.
- **`zip` with `deflate` only, and no default features:** pure Rust. The default features pull `bzip2` and `zstd`
  written in C, which breaks the ARM build.
- **`rlimit`:** sets the memory ceiling of the reading process (RLIMIT_AS), so the crate itself needs no `unsafe` code.

## Configuration

`FileConfig::default(blob_folder)`:

| Field | Default | Note |
|---|---|---|
| `byte_limit` | 16 MB | rejects above this, without writing |
| `character_limit` | 20,000 | text cut with a warning |
| `extraction_time` | 5 s | after this, the reader is killed |
| `extraction_memory_bytes` | 512 MB | minimum 128 MB |
| `extractor_path` | next to the executable, `poph-extrator` | the build must ship the program with it |

Linking to `dados.blobs` (the data section of `HarnessConfig`, in `poph-types`) is done by the `poph` crate (HR-06 and HR-24).

## `poph-extrator` program

It comes from the same crate (`src/bin/poph-extrator.rs`). It must be **next to** the `poph` executable.
`cargo build --workspace --release` builds both. If it is missing, PDF and Office receipt is refused with a clear
message ("file reading program unavailable"); images do not depend on it.

## Errors (what the customer sees, in summary)

| Situation | Error | Message |
|---|---|---|
| Above 16 MB | `Large` | "the file is X MB and the limit is 16 MB; it was not saved" |
| Empty | `Vazio` | "the file is empty" |
| Program, script, audio, `.doc`/`.ppt` | `UnsupportedType` | explains what it is and what to send |
| Name `.pdf`, content `.exe` | `MisleadingExtension` | "the name ends in .pdf, but the content is an executable program" |
| Corrupt PDF, zip bomb, unreadable spreadsheet | `Reading` | short reason, no file content |
| Time limit exceeded | `Time` | "exceeded 5 s and was stopped" |
| Blob not written | `WriteOutcome` | disk failure |

The messages above are the ones the code emits, so they stay in Portuguese.

## Tests

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd apps/pop-harness
cargo test -p poph-files                                  # 13 unit + 26 end-to-end
cargo clippy -p poph-files --all-targets -- -D warnings
cargo test --release -p poph-files --test receiving       # optional
```

The end-to-end tests (`tests/receiving.rs`) run the real `poph-extrator` program as a child process, as in production.

Fixtures in `crates/poph-files/tests/fixtures/`: PDFs (text, table, scanned, broken, bomb and injection), `.docx`
(includes a decompression bomb), `.pptx`, `.xlsx`, `.xls`, `.doc` and `.ppt`. The `.doc`/`.ppt`/`.xls`/injection files
were generated with LibreOffice; the PDF bomb and the Word bomb come from the HR-02 spike.

## Details worth knowing

- **Office without a `word/` folder:** `infer` (the type detector) classifies minimal docx and pptx files as generic ZIP.
  The crate recognizes Office by the package's internal name (`word/document.xml`, `xl/workbook.xml`, `ppt/slides/`),
  read as text from the bytes. If the name is fake, the isolated reader refuses.
- **Spreadsheet:** `.xls` and `.xlsx` are the same group; swapping the name between the two is not refused.
- **Word table:** a cell becomes a tab, a row becomes a line break.
- **XML escaping:** `&amp;`, `&lt;` and numeric references are resolved.
- **Blob write:** the content goes to a temporary file in the same folder and is renamed at the end, so a crash in the
  middle never leaves a partial blob under its final name. If a blob with the same size already exists, it is reused and not rewritten.
- **Blob read:** the hash is checked before the folder is touched. Only 64 lowercase hexadecimal characters are accepted,
  so a hash coming from the database cannot become a path outside the org's folder. The read has a byte limit.
- **Error text:** error messages never carry a disk path or the file content.
- **Envelope:** the text cannot close the untrusted block by itself; a `<<` inside it is written as `‹‹`.

## Files sent to the customer (outbound, MX-48)

The file the attendant sends through the Element goes out to WhatsApp, so it uses a separate list. It is also closed:
only the types listed below are accepted, one by one. Anything else is refused.

- **image:** JPEG, PNG, WebP, GIF (WhatsApp shows them as a photo);
- **document:** PDF, Word (.docx), Excel (.xlsx and .xls), PowerPoint (.pptx);
- **audio:** MP3, M4A, OGG/Opus (the format of a voice note recorded in Element), WAV, AMR, AAC, FLAC, AIFF;
- **video:** MP4, M4V, QuickTime (.mov), WebM, Matroska (.mkv), AVI, MPEG, WMV, FLV.

Refused: programs, scripts, archives, old Office (.doc, .ppt), plain text, and anything the content detector does not recognize.
The type is decided by the content. The file name only serves to catch a misleading name (for example, an image named `.pdf`).

Grouping for the name check:

- audio and video are one group: MP4 is a container for both, so an audio file named `.mp4` is not misleading;
- the four image formats are one group: a `.jpg` with PNG content is accepted;
- documents are fine-grained: a PDF named `.docx` is misleading and is refused.

## What is not in the crate

- Writing to the `attachments` table and the link to the turn engine: HR-24 and HR-13.
- Content validation of images (only the signature is checked; a corrupt image reaches the LLM).
- Cleanup of orphaned blobs and temporary files left by a crash in the middle (retention, HR-55).
