#!/usr/bin/env python3
"""Rename identifiers and modules with the language tool, never by text substitution.

`rust-analyzer` renames Rust (types, functions, files and the `mod`/`use` paths that point at
them) and the TypeScript language service renames the Console; both are asked for a workspace
edit and the edit is applied here. Strings, comments, serialized keys, routes, CLI flags, database
names and the flow language words are therefore never touched, because no language tool sees them
as the same symbol.

The map (`--map`) has one rename per line, applied in the order it lists them:

    # comment
    VarreduraBanco -> DatabaseScan
    VarreduraBanco @ crates/poph-types/src/config.rs -> DatabaseScan
    crates/poph-retention/src/armazem.rs -> crates/poph-retention/src/warehouse.rs

A line whose left side contains `/` is a file (module) rename; everything else is an identifier.
An identifier may be anchored to a path (`@ <path>`, or `@ <path>:<line>`), which is how a name
that exists in more than one crate is renamed in the one the map means: the anchor limits the
candidates, the leftover check and the report to that scope, and a rename outside the anchor is
printed as a reference the same language tool moved. Without an anchor, a name declared in more
than one crate or Node package is refused instead of renamed, because picking one of them would
rename the wrong symbol. The tool refuses, with a message and exit code 3, a rename of a name
published by the contract (table, column, index, trigger, route, operationId, JSON field,
`poph.toml` key, CLI command or flag, environment variable, metric, plugin protocol term, backup
key, flow language word), a new name that is not a valid identifier, a new name that still carries
a Portuguese term, and an identifier no language tool can rename safely. Exit code 4 means a
leftover of the old name was found outside strings and comments after the edit; exit code 5 means
the language tool is missing.

Scope is the source this project writes: everything except `design_assets/`, where only
`design_assets/<asset>/scripts/` counts, so rendered captures and images are never read.

Commands:

- `bash scripts/rename.sh --map scripts/rename-example.map --dry-run`: what would change.
- `bash scripts/rename.sh --map scripts/rename-example.map`: applies the map.
- `bash scripts/rename.sh --map ... --allow-leftover <path>`: accepts a leftover in one path, for
  a file that the language tool cannot see (generated data, a frozen example) and is printed.
"""

import argparse
import json
import os
import queue
import re
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
COMMENTS_SCANNER = ROOT / "scripts" / "strip-comments.py"
GLOSSARY_TOOL = ROOT / "scripts" / "glossary-terms.py"
NAMES_GATE = ROOT / "scripts" / "check_names.py"
TS_HELPER = ROOT / "scripts" / "rename_ts.mjs"

RUST_SUFFIXES = (".rs",)
TS_SUFFIXES = (".ts", ".tsx", ".mts", ".cts")
IDENTIFIER = re.compile(r"^[A-Za-z_$][A-Za-z0-9_$]*$")
RUST_DECLARATION = (
    r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?(?:struct|enum|trait|union|type|const|static|fn|mod|macro_rules!)\s+{name}\b"
)
TS_DECLARATION = (
    r"^\s*(?:export\s+)?(?:default\s+)?(?:declare\s+)?(?:async\s+)?"
    r"(?:function|class|interface|type|enum|const|let|var)\s+{name}\b"
)
TS_MEMBER = r"^\s*(?:readonly\s+|private\s+|public\s+|protected\s+)?{name}\s*[?:(]"

EXIT_OK = 0
EXIT_USAGE = 2
EXIT_REFUSED = 3
EXIT_LEFTOVER = 4
EXIT_TOOL = 5


def load_module(path, name):
    import importlib.util

    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise SystemExit(f"rename: cannot load {path.as_posix()}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


scanner = load_module(COMMENTS_SCANNER, "strip_comments")
glossary = load_module(GLOSSARY_TOOL, "glossary_terms")
names_gate = load_module(NAMES_GATE, "check_names")


class Refused(Exception):
    """The tool or the contract says no; the message is the whole answer."""


class Leftover(Exception):
    """The old name survived outside strings and comments; the edit of the pair was undone."""


class ToolMissing(Exception):
    """A language tool is not usable here."""


def published_paths(root):
    """What the repository publishes: the same scope the two gates use, for any root."""
    try:
        result = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
            capture_output=True,
            check=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return None
    return {name for name in result.stdout.decode("utf-8", "surrogateescape").split("\0") if name}


def scanned_files(root):
    visible = published_paths(root)
    files = []
    for path in scanner.collect_files([str(root)]):
        if scanner.is_skipped(path):
            continue
        relative = path.relative_to(root).as_posix()
        if visible is not None and relative not in visible:
            continue
        if not glossary.in_code_scope(relative):
            continue
        if scanner.language_of(path) in (None, "html"):
            continue
        files.append((relative, path))
    return files


def occurrences(root, name):
    """Every code occurrence of `name`, with the positions both protocols need."""
    found = []
    for relative, path in scanned_files(root):
        language = scanner.language_of(path)
        try:
            tokens = glossary.leitor_termos.tokens(path, language)
        except (OSError, UnicodeError):
            continue
        if name not in tokens:
            continue
        source = path.read_text(encoding="utf-8", errors="replace")
        starts = scanner.line_starts(source)
        cursor = 0
        for token in tokens:
            position = source.find(token, cursor)
            if position < 0:
                position = source.find(token)
            if position < 0:
                continue
            cursor = position + len(token)
            if token != name:
                continue
            line = scanner.locate(starts, position)[0]
            line_start = starts[line - 1]
            prefix = source[line_start:position]
            found.append(
                {
                    "relative": relative,
                    "path": path,
                    "line": line,
                    "utf8": len(prefix.encode("utf-8")),
                    "utf16": len(prefix.encode("utf-16-le")) // 2,
                }
            )
    return found


def declaration_rank(language, source_line, name):
    escaped = re.escape(name)
    if language == "rust":
        if re.match(RUST_DECLARATION.format(name=escaped), source_line):
            return 0
        return 1
    if re.match(TS_DECLARATION.format(name=escaped), source_line):
        return 0
    if re.match(TS_MEMBER.format(name=escaped), source_line):
        return 1
    return 2


def ordered_candidates(root, found, language, name, scope=None):
    """Occurrences that can be renamed, most likely declaration first, inside the anchor only.

    A line that declares `name` itself comes first: that is the symbol the map means. Ranking by
    the file stem instead left this list in path order, so a name declared in another crate could
    be picked first and renamed there — the leak the anchor and the ambiguity refusal close.
    """
    ordered = []
    for hit in found:
        if not in_scope(hit, scope):
            continue
        suffix = hit["path"].suffix.lower()
        if language == "rust" and suffix not in RUST_SUFFIXES:
            continue
        if language == "ts" and suffix not in TS_SUFFIXES:
            continue
        source_line = hit["path"].read_text(encoding="utf-8", errors="replace").splitlines()[hit["line"] - 1]
        ordered.append((declaration_rank(language, source_line, name), hit["relative"], hit))
    ordered.sort(key=lambda entry: (entry[0], entry[1], entry[2]["line"]))
    return [entry[2] for entry in ordered]


def apply_changes(changes, unit, dry_run=False):
    """Applies workspace edit changes; `unit` is the protocol's character unit."""
    label = "utf-16-le" if unit == "utf16" else "utf-8"
    scale = 2 if unit == "utf16" else 1
    by_file = {}
    for change in changes:
        by_file.setdefault(change["file"], []).append(change)
    written = {}
    for file, items in by_file.items():
        path = Path(file)
        text = path.read_text(encoding="utf-8")
        buffer = text.encode(label)
        for item in sorted(items, key=lambda change: change["start"], reverse=True):
            start = item["start"] * scale
            end = start + item["length"] * scale
            if end > len(buffer):
                raise Refused(f"rename: edit out of range in {path.as_posix()}: {item}")
            buffer = buffer[:start] + item["newText"].encode(label) + buffer[end:]
        if not dry_run:
            path.write_text(buffer.decode(label), encoding="utf-8")
        written[file] = len(items)
    return written


class RustAnalyzer:
    """Minimal LSP client: enough to ask rust-analyzer for a rename edit."""

    def __init__(self, root, verbose=False):
        self.root = root
        self.verbose = verbose
        self.pending = {}
        self.next_id = 0
        self.lock = threading.Lock()
        self.messages = queue.Queue()
        self.opened = set()
        environment = dict(os.environ)
        environment["PATH"] = os.path.expanduser("~/.cargo/bin") + ":" + environment.get("PATH", "")
        probe = subprocess.run(
            ["rust-analyzer", "--version"], cwd=str(root), env=environment, capture_output=True, text=True
        )
        if probe.returncode != 0:
            raise ToolMissing(f"rename: rust-analyzer is not usable here: {probe.stderr.strip() or probe.stdout.strip()}")
        self.proc = subprocess.Popen(
            ["rust-analyzer"],
            cwd=str(root),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            env=environment,
        )
        threading.Thread(target=self._reader, daemon=True).start()
        threading.Thread(target=self._dispatch, daemon=True).start()
        self._start()

    def _reader(self):
        stream = self.proc.stdout
        while True:
            headers = {}
            while True:
                line = stream.readline()
                if not line:
                    self.messages.put(None)
                    return
                line = line.decode("ascii", "replace").strip()
                if not line:
                    break
                key, _, value = line.partition(":")
                headers[key.strip().lower()] = value.strip()
            length = int(headers.get("content-length", 0))
            self.messages.put(json.loads(stream.read(length).decode("utf-8")))

    def _dispatch(self):
        while True:
            message = self.messages.get()
            if message is None:
                return
            if "id" in message and "method" not in message:
                with self.lock:
                    slot = self.pending.get(message["id"])
                if slot is not None:
                    slot.put(message)
                continue
            method = message.get("method", "")
            if method == "workspace/configuration":
                items = message.get("params", {}).get("items", [])
                self._send({"jsonrpc": "2.0", "id": message["id"], "result": [None] * len(items)})
            elif method in ("client/registerCapability", "window/workDoneProgress/create"):
                self._send({"jsonrpc": "2.0", "id": message["id"], "result": None})
            elif method == "workspace/applyEdit":
                self._send({"jsonrpc": "2.0", "id": message["id"], "result": {"applied": False}})
            elif method == "experimental/serverStatus" and self.verbose:
                params = message.get("params", {})
                print(f"rename: rust-analyzer {params.get('health')} quiescent={params.get('quiescent')}", file=sys.stderr)

    def _send(self, message):
        body = json.dumps(message).encode("utf-8")
        with self.lock:
            self.proc.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
            self.proc.stdin.flush()

    def notify(self, method, params):
        self._send({"jsonrpc": "2.0", "method": method, "params": params})

    def request(self, method, params, timeout=1800):
        self.next_id += 1
        slot = queue.Queue(1)
        with self.lock:
            self.pending[self.next_id] = slot
        self._send({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params})
        answer = slot.get(timeout=timeout)
        if "error" in answer:
            raise Refused(f"rename: rust-analyzer refused {method}: {answer['error'].get('message', answer['error'])}")
        return answer.get("result")

    def _start(self):
        root_uri = self.root.as_uri()
        self.request(
            "initialize",
            {
                "processId": os.getpid(),
                "clientInfo": {"name": "pop-harness-rename"},
                "rootUri": root_uri,
                "workspaceFolders": [{"uri": root_uri, "name": self.root.name}],
                "capabilities": {
                    "general": {"positionEncodings": ["utf-8"]},
                    "experimental": {"serverStatusNotification": True},
                    "workspace": {
                        "workspaceEdit": {"documentChanges": True},
                        "fileOperations": {"willRename": True, "didRename": True},
                        "workspaceFolders": True,
                    },
                    "textDocument": {"rename": {"prepareSupport": True}},
                },
            },
        )
        self.notify("initialized", {})

    def open_document(self, path):
        uri = path.as_uri()
        if uri in self.opened:
            return
        self.notify(
            "textDocument/didOpen",
            {
                "textDocument": {
                    "uri": uri,
                    "languageId": "rust",
                    "version": 1,
                    "text": path.read_text(encoding="utf-8"),
                }
            },
        )
        self.opened.add(uri)

    def close_documents(self):
        for uri in list(self.opened):
            self.notify("textDocument/didClose", {"textDocument": {"uri": uri}})
        self.opened.clear()

    def can_rename(self, hit):
        self.open_document(hit["path"])
        try:
            result = self.request(
                "textDocument/prepareRename",
                {
                    "textDocument": {"uri": hit["path"].as_uri()},
                    "position": {"line": hit["line"] - 1, "character": hit["utf8"]},
                },
            )
        except (Refused, queue.Empty):
            return False
        return bool(result)

    def rename(self, hit, new_name, attempts=3):
        request = {
            "textDocument": {"uri": hit["path"].as_uri()},
            "position": {"line": hit["line"] - 1, "character": hit["utf8"]},
            "newName": new_name,
        }
        for attempt in range(attempts):
            self.open_document(hit["path"])
            try:
                return workspace_changes(self.request("textDocument/rename", request))
            except Refused as error:
                if "content modified" not in str(error) or attempt + 1 == attempts:
                    raise
                self.close_documents()
                time.sleep(1.0)
        return []

    def will_rename_files(self, old_path, new_path, attempts=8, pause=2.0):
        request = {"files": [{"oldUri": old_path.as_uri(), "newUri": new_path.as_uri()}]}
        for attempt in range(attempts):
            changes = workspace_changes(self.request("workspace/willRenameFiles", request))
            if changes:
                return changes
            if attempt + 1 < attempts:
                time.sleep(pause)
        return []

    def stop(self):
        try:
            self.request("shutdown", {}, timeout=30)
            self.notify("exit", {})
        except (Refused, queue.Empty, OSError):
            pass
        finally:
            if self.proc.poll() is None:
                self.proc.terminate()


def workspace_changes(result):
    if not result:
        return []
    changes = []
    for entry in result.get("documentChanges") or []:
        if "kind" in entry:
            raise Refused(f"rename: the language tool asked for a file operation this tool does not apply: {entry['kind']}")
        file = entry["textDocument"]["uri"]
        if not file.startswith("file://"):
            continue
        for edit in entry.get("edits", []):
            changes.append(
                {
                    "file": file[len("file://"):],
                    "start": edit["range"]["start"],
                    "length": None,
                    "newText": edit["newText"],
                    "range": edit["range"],
                }
            )
    for file, edits in (result.get("changes") or {}).items():
        for edit in edits:
            changes.append(
                {
                    "file": file[len("file://"):] if file.startswith("file://") else file,
                    "start": edit["range"]["start"],
                    "length": None,
                    "newText": edit["newText"],
                    "range": edit["range"],
                }
            )
    return changes


def range_changes_to_offsets(documents, root):
    """Turns (line, character) ranges into byte offsets of the current text."""
    by_file = {}
    for change in documents:
        by_file.setdefault(change["file"], []).append(change)
    converted = []
    for file, items in by_file.items():
        path = Path(file)
        source = path.read_text(encoding="utf-8")
        starts = scanner.line_starts(source)
        encoded = source.encode("utf-8")
        line_bytes = [0]
        for start in starts[1:]:
            line_bytes.append(len(source[:start].encode("utf-8")))
        for item in items:
            start = item["range"]["start"]
            end = item["range"]["end"]
            begin = line_bytes[start["line"]] + start["character"]
            stop = line_bytes[end["line"]] + end["character"]
            if begin > len(encoded) or stop > len(encoded):
                raise Refused(f"rename: rust-analyzer edit out of range in {path.as_posix()}")
            converted.append({"file": file, "start": begin, "length": stop - begin, "newText": item["newText"]})
    return converted


class TypeScriptServer:
    """Drives `scripts/rename_ts.mjs`, which owns the language service of the Console."""

    def __init__(self, root):
        self.root = root
        node = shutil.which("node")
        if node is None:
            raise ToolMissing("rename: node is not on PATH; the Console cannot be renamed")
        if not (root / "console" / "node_modules" / "typescript").exists():
            raise ToolMissing("rename: console/node_modules/typescript is missing; run pnpm install in console/")
        self.proc = subprocess.Popen(
            [node, str(TS_HELPER), "--root", str(root)],
            cwd=str(root),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        answer = self._ask({"op": "ping"})
        if not answer.get("ok"):
            raise ToolMissing(f"rename: the TypeScript language service did not start: {answer.get('error')}")

    def _ask(self, request):
        self.proc.stdin.write(json.dumps(request) + "\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        if not line:
            stderr = self.proc.stderr.read().strip()
            raise ToolMissing(f"rename: the TypeScript language service stopped: {stderr}")
        return json.loads(line)

    def can_rename(self, hit):
        answer = self._ask({"op": "prepare", "file": str(hit["path"]), "line": hit["line"], "character": hit["utf16"]})
        if not answer.get("ok"):
            raise Refused(f"rename: TypeScript language service: {answer.get('error')}")
        return bool(answer.get("canRename"))

    def rename(self, hit, new_name):
        answer = self._ask(
            {"op": "rename", "file": str(hit["path"]), "line": hit["line"], "character": hit["utf16"], "newName": new_name}
        )
        if not answer.get("ok"):
            raise Refused(f"rename: TypeScript language service: {answer.get('error')}")
        return answer.get("changes") or []

    def file_rename(self, old_path, new_path):
        answer = self._ask({"op": "fileRename", "oldFile": str(old_path), "newFile": str(new_path)})
        if not answer.get("ok"):
            raise Refused(f"rename: TypeScript language service: {answer.get('error')}")
        return answer.get("changes") or []

    def stop(self):
        try:
            self._ask({"op": "quit"})
        except (OSError, ToolMissing):
            pass
        finally:
            if self.proc.poll() is None:
                self.proc.terminate()


def parse_scope(anchor):
    """Turns `path` or `path:line` into the scope of one identifier rename."""
    path, line = anchor, None
    head, separator, tail = anchor.rpartition(":")
    if separator and tail.isdigit():
        path, line = head, int(tail)
    return {"path": path.rstrip("/"), "line": line or None}


def in_scope_path(relative, scope):
    path = scope["path"]
    return relative == path or relative.startswith(path + "/")


def in_scope(hit, scope):
    if scope is None:
        return True
    return in_scope_path(hit["relative"], scope) and scope["line"] in (None, hit["line"])


def read_map(path):
    pairs = []
    errors = []
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        if "\t" in line:
            old, new = (part.strip() for part in line.split("\t", 1))
        elif "->" in line:
            old, new = (part.strip() for part in line.split("->", 1))
        else:
            errors.append(f"line {number}: expected `old<TAB>new` or `old -> new`: {raw!r}")
            continue
        if not old or not new:
            errors.append(f"line {number}: empty side: {raw!r}")
            continue
        name, _, anchor = old.partition("@")
        name, anchor = name.strip(), anchor.strip()
        if anchor and not name:
            errors.append(f"line {number}: the anchor has no identifier before `@`: {raw!r}")
            continue
        if not anchor and ("/" in old or old.endswith(RUST_SUFFIXES + TS_SUFFIXES)):
            pairs.append({"kind": "path", "old": old, "new": new, "line": number})
            continue
        pairs.append(
            {
                "kind": "identifier",
                "old": name,
                "new": new,
                "line": number,
                "scope": parse_scope(anchor) if anchor else None,
            }
        )
    return pairs, errors


UNIT_MARKERS = ("Cargo.toml", "package.json")


def unit_of(root, relative):
    """The building block a file belongs to: the nearest crate or Node package above it."""
    path = (root / relative).resolve()
    for parent in path.parents:
        if not parent.is_dir():
            continue
        for marker in UNIT_MARKERS:
            if (parent / marker).is_file():
                try:
                    return parent.relative_to(root).as_posix() or "."
                except ValueError:
                    return parent.as_posix()
        if parent == root:
            break
    return "."


def declared_units(root, found, count):
    """Units where `count`'s name is declared, so two symbols sharing it are told apart."""
    units = {}
    for hit in found:
        suffix = hit["path"].suffix.lower()
        if suffix not in RUST_SUFFIXES + TS_SUFFIXES:
            continue
        language = "rust" if suffix in RUST_SUFFIXES else "ts"
        source_line = hit["path"].read_text(encoding="utf-8", errors="replace").splitlines()[hit["line"] - 1]
        if declaration_rank(language, source_line, count["old"]) != 0:
            continue
        units.setdefault(unit_of(root, hit["relative"]), hit["relative"])
    return units


def published_names():
    inventory = glossary.inventario_publicado()
    names = {}
    for _, value, reason in names_gate.generated_entries(inventory):
        names.setdefault(value.lower(), reason)
    for kind, value, reason in names_gate.MANUAL_ENTRIES:
        if kind == "name":
            names.setdefault(value.lower(), reason)
    return names


def portuguese_terms():
    entries, _ = names_gate.read_map()
    allowed = {value for kind, value, _ in names_gate.MANUAL_ENTRIES if kind == "term"}
    return names_gate.build_dictionary(entries, allowed)


def check_identifier_pair(pair, published, dictionary):
    old, new = pair["old"], pair["new"]
    if not IDENTIFIER.match(new):
        raise Refused(f"rename: {new!r} is not a valid identifier (line {pair['line']})")
    if old == new:
        raise Refused(f"rename: {old!r} is mapped to itself (line {pair['line']})")
    reason = published.get(old.lower())
    if reason:
        raise Refused(
            f"rename: refuses {old!r}: published name ({reason}); contract names keep the current value (P5)"
        )
    terms = [term for term in names_gate.split_identifier(new) if term in dictionary]
    if terms:
        raise Refused(
            f"rename: refuses {new!r}: it still carries a Portuguese term "
            f"({', '.join(sorted({dictionary[term] for term in terms}))}); the identifier gate would fail"
        )


class Snapshot:
    """Bytes of the files one rename is about to touch, so a refusal leaves no half edit."""

    def __init__(self, files):
        self.files = {}
        for file in files:
            path = Path(file)
            if path.is_file():
                self.files[path] = path.read_bytes()

    def restore(self):
        for path, data in self.files.items():
            path.write_bytes(data)


def changes_files(changes):
    return sorted({change["file"] for change in changes})


def leftover_findings(found, allow_leftover, scope=None):
    """Splits the remaining occurrences into accepted (allowed path) and blocking ones.

    Under an anchor only the occurrences inside it block: a name that also exists in another crate
    is a different symbol, and a reference the language tool missed in Rust breaks the build anyway.
    """
    inside = [hit for hit in found if in_scope(hit, scope)]
    accepted = [hit for hit in inside if any(hit["relative"].startswith(prefix) for prefix in allow_leftover)]
    leftover = [hit for hit in inside if hit not in accepted]
    return leftover, accepted


def report_scope(root, pair, tally):
    """Prints what left the anchor: a reference elsewhere, renamed by the same language tool."""
    scope = pair.get("scope")
    if scope is None or not tally:
        return
    outside = {}
    for file, count in tally.items():
        try:
            relative = Path(file).resolve().relative_to(root).as_posix()
        except ValueError:
            relative = file
        if not in_scope_path(relative, scope):
            outside[relative] = count
    if outside:
        shown = ", ".join(f"{relative}:{count}" for relative, count in sorted(outside.items()))
        print(f"rename: {len(outside)} file(s) outside the anchor also changed, as references: {shown}")


def rename_identifier(root, pair, dry_run, tools, allow_leftover):
    old, new = pair["old"], pair["new"]
    scope = pair.get("scope")
    found = occurrences(root, old)
    if not found:
        raise Refused(f"rename: {old!r} does not appear as an identifier outside strings and comments")
    if scope is not None and not [hit for hit in found if in_scope(hit, scope)]:
        raise Refused(
            f"rename: the anchor {scope['path']} of {old!r} holds no occurrence of it; "
            "the map points at the wrong file"
        )
    if scope is None:
        units = declared_units(root, found, pair)
        if len(units) > 1:
            shown = ", ".join(f"{unit} ({path})" for unit, path in sorted(units.items()))
            raise Refused(
                f"rename: {old!r} is declared in {len(units)} modules: {shown}; anchor the rename "
                f"in the map with `{old} @ <path> -> {new}` so one symbol is renamed"
            )
    tally = {}
    attempted = False
    snapshot = None
    for language in ("rust", "ts"):
        candidates = ordered_candidates(root, found, language, old, scope)
        if not candidates:
            continue
        attempted = True
        tool = tools.rust if language == "rust" else tools.get_typescript()
        if tool is None:
            reason = tools.typescript_error if language == "ts" else tools.rust_error
            raise ToolMissing(f"rename: the {language} language tool is not usable ({reason}); {old!r} cannot be renamed safely")
        target = None
        for candidate in candidates:
            if tool.can_rename(candidate):
                target = candidate
                break
        if target is None:
            raise Refused(
                f"rename: the {language} language tool cannot rename {old!r} safely "
                f"({len(candidates)} occurrence(s) tried, none renameable); the module goes back to the Planner"
            )
        changes = tool.rename(target, new)
        if not changes:
            raise Refused(f"rename: the {language} language tool returned no edit for {old!r}")
        if language == "rust":
            changes = range_changes_to_offsets(changes, root)
            unit = "utf8"
        else:
            unit = "utf16"
        snapshot = Snapshot(changes_files(changes)) if not dry_run else None
        written = apply_changes(changes, unit, dry_run)
        if language == "rust":
            tools.rust.close_documents()
        for file, count in written.items():
            tally[file] = tally.get(file, 0) + count
        if dry_run:
            return tally
        found = occurrences(root, old)
        if not found:
            break
    if not attempted:
        places = ", ".join(f"{hit['relative']}:{hit['line']}" for hit in found[:5])
        raise Refused(
            f"rename: no language tool owns {old!r}: it only appears outside Rust and TypeScript ({places}); "
            "it is data, a serialized key or a frozen example, not an identifier to rename"
        )
    leftover, accepted = leftover_findings(found, allow_leftover, scope)
    for hit in accepted:
        print(f"rename: accepted leftover of {old!r} in {hit['relative']}:{hit['line']}")
    if leftover:
        if snapshot is not None:
            snapshot.restore()
        shown = ", ".join(f"{hit['relative']}:{hit['line']}" for hit in leftover[:10])
        raise Leftover(
            f"rename: {len(leftover)} occurrence(s) of {old!r} left outside strings and comments ({shown}); "
            "the edit was undone"
        )
    report_scope(root, pair, tally)
    return tally


def mask_comments(source, language):
    """Source with comment characters replaced by spaces, so offsets and lines do not move."""
    masked = list(source)
    for span in scanner.normalize_spans(source, scanner.SCANNERS[language](source)):
        for offset in range(span.start, min(span.end, len(masked))):
            if masked[offset] != "\n":
                masked[offset] = " "
    return "".join(masked)


def module_references(root, stem):
    found = []
    pattern = re.compile(rf"\bmod\s+{re.escape(stem)}\b|::{re.escape(stem)}\b|\b{re.escape(stem)}::")
    for relative, path in scanned_files(root):
        if path.suffix.lower() != ".rs":
            continue
        source = path.read_text(encoding="utf-8", errors="replace")
        code = mask_comments(source, "rust")
        for match in pattern.finditer(code):
            found.append({"relative": relative, "line": source[: match.start()].count("\n") + 1})
    return found


def rename_path(root, pair, dry_run, tools):
    old_path = (root / pair["old"]).resolve()
    new_path = (root / pair["new"]).resolve()
    if not old_path.is_file():
        raise Refused(f"rename: {pair['old']} is not a file")
    if new_path.exists():
        raise Refused(f"rename: {pair['new']} already exists")
    if old_path.parent != new_path.parent:
        raise Refused("rename: a file rename must stay in the same directory (module rename)")
    if old_path.suffix != new_path.suffix:
        raise Refused("rename: a file rename must keep the extension")
    stem = old_path.stem
    changes = []
    unit = "utf8"
    if old_path.suffix in RUST_SUFFIXES:
        if tools.rust is None:
            raise ToolMissing(f"rename: rust-analyzer is not usable ({tools.rust_error}); a module cannot be renamed safely")
        raw = tools.rust.will_rename_files(old_path, new_path)
        if not raw:
            raise Refused(
                f"rename: rust-analyzer returned no reference edit for {pair['old']} -> {pair['new']}; "
                "the module path was not rewritten, so nothing was moved"
            )
        changes = range_changes_to_offsets(raw, root)
        tools.rust.close_documents()
    elif old_path.suffix in TS_SUFFIXES:
        server = tools.get_typescript()
        if server is None:
            raise ToolMissing(
                f"rename: the TypeScript language service is not usable ({tools.typescript_error}); "
                "a module cannot be renamed safely"
            )
        changes = server.file_rename(old_path, new_path)
        unit = "utf16"
    snapshot = Snapshot(changes_files(changes)) if changes and not dry_run else None
    tally = apply_changes(changes, unit, dry_run) if changes else {}
    if dry_run:
        return {"edits": tally, "moved": f"{pair['old']} -> {pair['new']}"}
    tracking = subprocess.run(
        ["git", "-C", str(root), "ls-files", "--error-unmatch", pair["old"]], capture_output=True, text=True
    )
    if tracking.returncode == 0:
        move = subprocess.run(["git", "-C", str(root), "mv", pair["old"], pair["new"]], capture_output=True, text=True)
        if move.returncode != 0:
            raise Refused(f"rename: git mv failed: {move.stderr.strip()}")
    else:
        old_path.rename(new_path)
    if old_path.suffix in RUST_SUFFIXES:
        left = module_references(root, stem)
        if left:
            if tracking.returncode == 0:
                subprocess.run(["git", "-C", str(root), "mv", pair["new"], pair["old"]], capture_output=True, text=True)
            elif new_path.exists():
                new_path.rename(old_path)
            if snapshot is not None:
                snapshot.restore()
            shown = ", ".join(f"{hit['relative']}:{hit['line']}" for hit in left[:10])
            raise Leftover(f"rename: {len(left)} module reference(s) to {stem!r} left ({shown}); the rename was undone")
    return {"edits": tally, "moved": f"{pair['old']} -> {pair['new']}"}


class Tools:
    """Language tools started on demand: rust-analyzer costs a workspace load, tsserver a program."""

    def __init__(self, root, verbose):
        self.root = root
        self.verbose = verbose
        self.rust = None
        self.rust_error = None
        self.typescript = None
        self.typescript_error = None
        try:
            self.rust = RustAnalyzer(root, verbose)
        except ToolMissing as error:
            self.rust_error = str(error).removeprefix("rename: ")

    def get_typescript(self):
        if self.typescript is None and self.typescript_error is None:
            try:
                self.typescript = TypeScriptServer(self.root)
            except ToolMissing as error:
                self.typescript_error = str(error).removeprefix("rename: ")
        return self.typescript

    def stop(self):
        if self.rust is not None:
            self.rust.stop()
        if self.typescript is not None:
            self.typescript.stop()


def main(argv):
    parser = argparse.ArgumentParser(
        prog="rename.sh",
        description="Renames identifiers and modules with the language tool, from a map.",
    )
    parser.add_argument("map_argument", nargs="?", help="map file with one rename per line, applied in order")
    parser.add_argument("--map", dest="map_option", help="same as the positional argument")
    parser.add_argument("--dry-run", dest="dry_run", action="store_true", help="prints the plan without writing")
    parser.add_argument("--root", default=str(ROOT), help="repository root")
    parser.add_argument(
        "--allow-leftover",
        dest="allow_leftover",
        action="append",
        default=[],
        help="path prefix where a leftover of the old name is accepted and printed",
    )
    parser.add_argument("--verbose", action="store_true", help="prints the language tool status")
    arguments = parser.parse_args(argv)

    root = Path(arguments.root).resolve()
    if not root.is_dir():
        print(f"rename: root not found: {root.as_posix()}", file=sys.stderr)
        return EXIT_USAGE
    map_name = arguments.map_option or arguments.map_argument
    if not map_name:
        print("rename: a map file is required: bash scripts/rename.sh --map <file>", file=sys.stderr)
        return EXIT_USAGE
    map_path = Path(map_name)
    if not map_path.is_file():
        print(f"rename: map not found: {map_name}", file=sys.stderr)
        return EXIT_USAGE
    pairs, errors = read_map(map_path)
    if errors:
        for error in errors:
            print(f"rename: {map_path.name}: {error}", file=sys.stderr)
        return EXIT_USAGE
    if not pairs:
        print("rename: the map is empty", file=sys.stderr)
        return EXIT_USAGE

    published = published_names()
    dictionary = portuguese_terms()
    try:
        for pair in pairs:
            if pair["kind"] == "identifier":
                check_identifier_pair(pair, published, dictionary)
    except Refused as error:
        print(str(error), file=sys.stderr)
        return EXIT_REFUSED

    tools = Tools(root, arguments.verbose)
    total_edits = 0
    try:
        for pair in pairs:
            if pair["kind"] == "identifier":
                tally = rename_identifier(root, pair, arguments.dry_run, tools, arguments.allow_leftover)
                count = sum(tally.values())
                total_edits += count
                print(f"rename: {pair['old']} -> {pair['new']} ({count} edit(s) in {len(tally)} file(s))")
                if arguments.verbose:
                    for file, items in sorted(tally.items()):
                        print(f"  {file}: {items}")
            else:
                result = rename_path(root, pair, arguments.dry_run, tools)
                count = sum(result["edits"].values())
                total_edits += count
                print(f"rename: {result['moved']} ({count} reference edit(s))")
    except Leftover as error:
        print(str(error), file=sys.stderr)
        return EXIT_LEFTOVER
    except Refused as error:
        print(str(error), file=sys.stderr)
        return EXIT_REFUSED
    except ToolMissing as error:
        print(str(error), file=sys.stderr)
        return EXIT_TOOL
    finally:
        tools.stop()

    if arguments.dry_run:
        print(f"rename: dry run, nothing written ({len(pairs)} rename(s), {total_edits} edit(s))")
        return EXIT_OK
    print(f"rename: done ({len(pairs)} rename(s), {total_edits} edit(s))")
    print(
        "rename: next: bash scripts/check-names.sh --atualizar-pendentes and "
        "bash scripts/check-comments.sh --atualizar-pendentes"
    )
    return EXIT_OK


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
