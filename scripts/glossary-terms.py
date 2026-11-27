#!/usr/bin/env python3
"""Measures the identifier terms of the tree, extracts the published names and validates the glossary.

This script is the reproducible source of `docs/GLOSSARY.md`: the translation table of the document
is generated here from two versioned inputs — the terms measured in the code and the Portuguese to
English map of `scripts/glossary-map.txt`.

Commands:

- `--medir [--top N]`: counts the identifier terms of the code and configuration files, ordered by
  frequency. It is the measurement the table rests on.
- `--publicados`: extracts the published names (contract) by kind: binary, environment variable,
  metric, table, column, index, trigger, route, `operationId`, JSON field, CLI command and flag,
  `poph.toml` key, plugin protocol term and backup format key.
- `--gerar`: rewrites the table region of `docs/GLOSSARY.md`.
- `--validar`: checks the table on disk against the measurement, the published inventory and the 41
  flow language names. It exits non zero on the first problem.

The string and comment reader is the one of `scripts/strip-comments.py` (imported, not copied): it
knows where a Rust raw string, a TypeScript template, a shell heredoc and a Python docstring starts
and ends. Only the space between one and the other is used to read an identifier — a string and a
comment never become a term.

Classification: a term is Portuguese when it is in `scripts/glossary-map.txt`. The map covers the
Portuguese terms measured with high frequency; `--medir --candidatos` lists the measured terms that
are not in the map yet, for the next translation round. Nothing here invents a term: a map line
whose term does not appear in the tree fails the validation.

Measurement scope: only what the repository publishes. The scan uses `git ls-files --cached
--others --exclude-standard`, so a file that `.git/info/exclude` hides on this machine is out of the
count — without that, the table generated here differs from the one a clean clone validates. The
published name extraction follows the same scope: an environment variable cited only in an internal
document is not in the inventory. Without `git` available, the scan falls back to the whole working
tree.

In `design_assets/` only the utility scripts count: `design_assets/<asset>/scripts/**` is measured,
and captures, images and visual artefacts stay out. The rule is the one of `in_code_scope`, used by
the identifier gate and the renamer as well, so the three never disagree on scope.
"""

import argparse
import importlib.util
import json
import re
import sqlite3
import subprocess
import sys
import tomllib
import unicodedata
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MAPPING = ROOT / "scripts" / "glossary-map.txt"
GLOSSARY = ROOT / "docs" / "GLOSSARY.md"
READER = ROOT / "scripts" / "strip-comments.py"

TABLE_START = "<!-- glossario:tabela:inicio -->"
TABLE_END = "<!-- glossario:tabela:fim -->"
HEADER = "| termo_pt | termo_en | decisão | onde | ocorrências |"
SEPARATOR = "| --- | --- | --- | --- | --- |"

MIGRATIONS = ROOT / "crates" / "poph-store" / "migrations" / "harness"
CONTRACTS_BASE = ROOT / "tests" / "contratos" / "base"
DEFAULT_TOML = ROOT / "tests" / "contratos" / "base" / "config" / "default.toml"

DESIGN_ASSETS = "design_assets/"
DESIGN_ASSETS_SCRIPTS = re.compile(r"^design_assets/[^/]+/scripts/")

IDENT_START = set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_$")
JSX_OPENS_AFTER = set("(,=:[!&|?{};") | {"=>"}
KEY_AFTER = {"{", ",", ";"}
FUNCTION_GENERIC = re.compile(r"<\s*[A-Za-z_$][\w$]*\s*(?:,|extends\b)")


def _next_visible(source, index):
    while index < len(source) and source[index].isspace():
        index += 1
    return source[index] if index < len(source) else ""

PUBLISHED_TYPES = (
    "linguagem_fluxos",
    "binario",
    "variavel_ambiente",
    "metrica",
    "tabela",
    "coluna",
    "indice",
    "gatilho",
    "rota",
    "operation_id",
    "campo_json",
    "comando_cli",
    "flag_cli",
    "chave_toml",
    "protocolo_plugin",
    "backup",
)


def load_reader():
    spec = importlib.util.spec_from_file_location("strip_comments", READER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


reader = load_reader()


def without_accent(text):
    decomposed = unicodedata.normalize("NFD", text)
    return "".join(char for char in decomposed if not unicodedata.combining(char))


def split_term(word):
    """Splits `snake_case`, `kebab-case` and `camelCase` into lowercase terms."""
    parts = []
    for block in re.split(r"(\d+)", word):
        if not block or block.isdigit():
            continue
        current = ""
        for char in block:
            if not (char.isalnum()):
                if current:
                    parts.append(current)
                current = ""
                continue
            if char.isupper() and current and current[-1].islower():
                parts.append(current)
                current = ""
            elif char.isupper() and len(current) > 1 and current[-1].isupper():
                parts.append(current[:-1])
                current = current[-1]
            current += char
        if current:
            parts.append(current)
    terms = []
    for part in parts:
        term = without_accent(part).lower()
        if len(term) >= 2 and term.isalpha():
            terms.append(term)
    return terms


class TermReader:
    """Reads code identifiers, skipping strings and comments by the syntax of the language."""

    def __init__(self, module):
        self.m = module

    def _identifier(self, source, index):
        position = index
        while index < len(source) and (source[index].isalnum() or source[index] in "_$"):
            index += 1
        return source[position:index], index

    def rust(self, source):
        tokens = []
        size = len(source)
        index = 0
        while index < size:
            char = source[index]
            if char == "/" and source.startswith("//", index):
                stop = source.find("\n", index)
                index = size if stop < 0 else stop
                continue
            if char == "/" and source.startswith("/*", index):
                depth = 0
                cursor = index
                while cursor < size:
                    if source.startswith("/*", cursor):
                        depth += 1
                        cursor += 2
                        continue
                    if source.startswith("*/", cursor):
                        depth -= 1
                        cursor += 2
                        if depth == 0:
                            break
                        continue
                    cursor += 1
                index = cursor
                continue
            raw_end = self.m.scan_rust_raw(source, index)
            if raw_end is not None:
                index = raw_end
                continue
            if char == '"':
                index = self.m.scan_escaped(source, index + 1, '"')
                continue
            if char == "'":
                index = self.m.scan_rust_quote(source, index)
                continue
            if char in IDENT_START:
                token, index = self._identifier(source, index)
                tokens.append(token)
                continue
            index += 1
        return tokens

    def _names(self, snippet):
        names = []
        index = 0
        while index < len(snippet):
            if snippet[index] in IDENT_START:
                name, index = self._identifier(snippet, index)
                names.append(name)
            else:
                index += 1
        return names

    def _abre_jsx(self, source, index, previous):
        following = source[index + 1] if index + 1 < len(source) else ""
        if not (following in IDENT_START or following == ">"):
            return False
        if FUNCTION_GENERIC.match(source, index):
            return False
        return previous == "" or previous in JSX_OPENS_AFTER or previous in self.m.JS_REGEX_WORDS

    def _template(self, source, index):
        size = len(source)
        cursor = index + 1
        tokens = []
        while cursor < size:
            char = source[cursor]
            if char == "\\":
                cursor += 2
                continue
            if char == "`":
                return cursor + 1, tokens
            if char == "$" and source.startswith("${", cursor):
                stop = self.m.scan_js_code(source, cursor + 2, [], True)
                tokens.extend(self.javascript(source[cursor + 2 : max(cursor + 2, stop - 1)]))
                cursor = stop
                continue
            cursor += 1
        return size, tokens

    def javascript(self, source, jsx=False):
        tokens = []
        size = len(source)
        index = 0
        previous = ""
        stack = [["code", 0, False]]
        while index < size:
            frame = stack[-1]
            char = source[index]
            if frame[0] == "children":
                if char == "<":
                    if source.startswith("</", index):
                        stop = source.find(">", index)
                        stop = size if stop < 0 else stop + 1
                        tokens.extend(self._names(source[index:stop]))
                        stack.pop()
                        index = stop
                        previous = "jsx"
                        continue
                    stack.append(["tag", 0, False])
                    index += 1
                    previous = "<"
                    continue
                if char == "{":
                    stack.append(["code", 0, True])
                    index += 1
                    previous = "{"
                    continue
                index += 1
                continue
            if frame[0] == "tag":
                if char in IDENT_START:
                    token, index = self._identifier(source, index)
                    tokens.append(token)
                    previous = token
                    continue
                if char in "\"'":
                    index = self.m.scan_escaped(source, index + 1, char)
                    continue
                if char == "{":
                    stack.append(["code", 0, True])
                    index += 1
                    previous = "{"
                    continue
                if source.startswith("/>", index):
                    stack.pop()
                    index += 2
                    previous = "jsx"
                    continue
                if char == ">":
                    frame[0] = "children"
                    index += 1
                    previous = ">"
                    continue
                index += 1
                continue
            if char.isspace():
                index += 1
                continue
            if char == "/" and source.startswith("//", index):
                stop = source.find("\n", index)
                index = size if stop < 0 else stop
                continue
            if char == "/" and source.startswith("/*", index):
                stop = source.find("*/", index + 2)
                index = size if stop < 0 else stop + 2
                continue
            if char == "/":
                if previous == "" or previous in self.m.JS_REGEX_CHARS or previous in self.m.JS_REGEX_WORDS:
                    stop = self.m.scan_js_regex(source, index)
                    if stop is not None:
                        index = stop
                        previous = "regex"
                        continue
                previous = "/"
                index += 1
                continue
            if char == "`":
                index, modelo = self._template(source, index)
                tokens.extend(modelo)
                previous = "string"
                continue
            if char in "\"'":
                stop = self.m.scan_escaped(source, index + 1, char)
                if previous in KEY_AFTER and _next_visible(source, stop) == ":":
                    tokens.extend(self._names(source[index + 1 : max(index + 1, stop - 1)]))
                index = stop
                previous = "string"
                continue
            if char == "{":
                frame[1] += 1
                index += 1
                previous = "{"
                continue
            if char == "}":
                if frame[1] == 0 and frame[2]:
                    stack.pop()
                elif frame[1] > 0:
                    frame[1] -= 1
                index += 1
                previous = "}"
                continue
            if jsx and char == "<" and self._abre_jsx(source, index, previous):
                if source.startswith("<>", index):
                    stack.append(["children", 0, False])
                    index += 2
                    previous = "<"
                    continue
                stack.append(["tag", 0, False])
                index += 1
                previous = "<"
                continue
            if char in IDENT_START:
                token, index = self._identifier(source, index)
                tokens.append(token)
                previous = token
                continue
            if char == ">":
                previous = "=>" if index > 0 and source[index - 1] == "=" else ">"
                index += 1
                continue
            previous = char
            index += 1
        return tokens

    def css(self, source):
        tokens = []
        size = len(source)
        index = 0
        while index < size:
            char = source[index]
            if source.startswith("/*", index):
                stop = source.find("*/", index + 2)
                index = size if stop < 0 else stop + 2
                continue
            if char == "@":
                index += 1
                while index < size and (source[index].isalnum() or source[index] in "-_"):
                    index += 1
                continue
            if char in "\"'":
                index = self.m.scan_escaped(source, index + 1, char)
                continue
            if char in IDENT_START:
                token, index = self._identifier(source, index)
                tokens.append(token)
                continue
            index += 1
        return tokens

    def python(self, source):
        tokens = []
        size = len(source)
        index = 0
        while index < size:
            char = source[index]
            if char == "#":
                stop = source.find("\n", index)
                index = size if stop < 0 else stop
                continue
            if char in "'\"":
                index = self.m.scan_python_string(source, index, self.m.python_prefix(source, index))
                continue
            if char in IDENT_START:
                token, index = self._identifier(source, index)
                tokens.append(token)
                continue
            index += 1
        return tokens

    def shell(self, source):
        tokens = []
        size = len(source)
        frames = []
        heredocs = []
        index = 0
        while index < size:
            char = source[index]
            frame = frames[-1] if frames else "root"
            if frame in ("single", "ansi"):
                if char == "\\" and frame == "ansi":
                    index += 2
                    continue
                if char == "'":
                    frames.pop()
                index += 1
                continue
            if frame == "double":
                if char == "\\":
                    index += 2
                    continue
                if char == '"':
                    frames.pop()
                    index += 1
                    continue
                if char == "$" and source.startswith("${", index):
                    frames.append("brace")
                    index += 2
                    continue
                if char == "$":
                    index += 1
                    token, index = self._identifier(source, index)
                    if token:
                        tokens.append(token)
                    continue
                index += 1
                continue
            if frame == "backtick":
                if char == "\\":
                    index += 2
                    continue
                if char == "`":
                    frames.pop()
                    index += 1
                    continue
                if char in IDENT_START:
                    token, index = self._identifier(source, index)
                    tokens.append(token)
                    continue
                index += 1
                continue
            if frame == "brace":
                if char == "}":
                    frames.pop()
                    index += 1
                    continue
                if char in IDENT_START:
                    token, index = self._identifier(source, index)
                    tokens.append(token)
                    continue
                index += 1
                continue
            if char == "#":
                stop = source.find("\n", index)
                index = size if stop < 0 else stop
                continue
            if char == "<" and source.startswith("<<", index):
                parsed = self.m.parse_heredoc(source, index)
                if parsed is not None:
                    stop, delimiter, strip_tabs = parsed
                    heredocs.append((delimiter, strip_tabs))
                    index = stop
                    continue
            if char == "\n" and heredocs:
                index = self.m.skip_heredocs(source, index + 1, heredocs)
                heredocs = []
                continue
            if char == "'":
                frames.append("single")
                index += 1
                continue
            if char == '"':
                frames.append("double")
                index += 1
                continue
            if char == "`":
                frames.append("backtick")
                index += 1
                continue
            if char == "$" and index + 1 < size and source[index + 1] == "'":
                frames.append("ansi")
                index += 2
                continue
            if char in IDENT_START:
                token, index = self._identifier(source, index)
                tokens.append(token)
                continue
            index += 1
        return tokens

    def generic(self, source, block_markers=(), line_markers=()):
        tokens = []
        size = len(source)
        index = 0
        while index < size:
            char = source[index]
            matched_block = next(
                (marker for marker in block_markers if source.startswith(marker, index)), None
            )
            if matched_block is not None:
                stop = source.find("*/", index + len(matched_block))
                index = size if stop < 0 else stop + 2
                continue
            matched_line = next(
                (marker for marker in line_markers if source.startswith(marker, index)), None
            )
            if matched_line is not None:
                if char == "#" and index > 0 and not (source[index - 1].isspace() or source[index - 1] in ";"):
                    pass
                else:
                    stop = source.find("\n", index)
                    index = size if stop < 0 else stop
                    continue
            if char in "\"'`":
                if char == "`" and "`" in line_markers:
                    index += 1
                    continue
                index = self.m.scan_escaped(source, index + 1, char)
                continue
            if char in IDENT_START:
                token, index = self._identifier(source, index)
                tokens.append(token)
                continue
            index += 1
        return tokens

    def tokens(self, path, language):
        source = path.read_text(encoding="utf-8", errors="replace")
        if language == "rust":
            return self.rust(source)
        if language == "javascript":
            return self.javascript(source, jsx=path.suffix in (".jsx", ".tsx"))
        if language == "python":
            return self.python(source)
        if language == "shell":
            return self.shell(source)
        if language == "css":
            return self.css(source)
        if language == "sql":
            return self.generic(source, ("/*",), ("--",))
        if language in ("toml", "yaml", "hash", "dockerfile"):
            return self.generic(source, (), ("#",))
        return []


term_reader = TermReader(reader)


def area_of(path):
    parts = path.parts
    if parts[0] == "crates" and len(parts) > 1:
        return parts[1]
    if parts[0] == "console":
        return "console"
    if parts[0] == "scripts":
        return "scripts"
    return "raiz"


@dataclass
class Measurement:
    count: Counter = field(default_factory=Counter)
    areas: dict = field(default_factory=lambda: defaultdict(Counter))
    files: int = 0
    by_language: Counter = field(default_factory=Counter)

    def areas_of(self, term, limit=2):
        ranking = self.areas.get(term)
        if not ranking:
            return "-"
        ordered = sorted(ranking.items(), key=lambda item: (-item[1], item[0]))
        return ", ".join(name for name, _ in ordered[:limit])


def repository_paths():
    try:
        result = subprocess.run(
            ["git", "-C", str(ROOT), "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
            capture_output=True,
            check=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return None
    names = result.stdout.decode("utf-8", "surrogateescape").split("\0")
    return {name for name in names if name}


def in_code_scope(path):
    """Whether a repository path is source this project writes, and so measured and gated.

    `design_assets/` holds rendered captures, images and binaries next to the scripts that produce
    them; only `design_assets/<asset>/scripts/` is source. Excluding the whole directory would
    leave that source ungated, so the rule is the narrow one and every gate shares it.
    """
    if not path.startswith(DESIGN_ASSETS):
        return True
    return bool(DESIGN_ASSETS_SCRIPTS.match(path))


def measure():
    measurement = Measurement()
    visible = repository_paths()
    for path in reader.collect_files([str(ROOT)]):
        relative = path.relative_to(ROOT).as_posix()
        if visible is not None and relative not in visible:
            continue
        if not in_code_scope(relative):
            continue
        if reader.is_skipped(path):
            continue
        language = reader.language_of(path)
        if language in (None, "html"):
            continue
        measurement.files += 1
        measurement.by_language[language] += 1
        area = area_of(path.relative_to(ROOT))
        for token in term_reader.tokens(path, language):
            for term in split_term(token):
                measurement.count[term] += 1
                measurement.areas[term][area] += 1
    return measurement


def read_mapping():
    mapping = {}
    duplicated = []
    if not MAPPING.exists():
        return mapping, ["scripts/glossary-map.txt does not exist"]
    for number, line in enumerate(MAPPING.read_text(encoding="utf-8").splitlines(), start=1):
        if not line.strip():
            continue
        fields = line.split("\t")
        if len(fields) != 2:
            duplicated.append(f"line {number}: expected two fields separated by TAB: {line!r}")
            continue
        term, english = fields[0].strip(), fields[1].strip()
        if not term or not english:
            duplicated.append(f"line {number}: empty term or translation")
            continue
        if term in mapping:
            duplicated.append(f"line {number}: repeated term: {term}")
            continue
        mapping[term] = english
    return mapping, duplicated


LEGACY_FLOW_CONSTANTS = {
    "DECLARATIONS": "DECLARACOES",
    "VALUES": "VALORES",
    "EFFECTS": "EFEITOS",
    "COMBINATORS": "COMBINADORES",
    "CONSTANTS": "CONSTANTES",
    "PURE_FUNCTIONS": "FUNCOES_PURAS",
}


def flow_language_names():
    source = (ROOT / "crates" / "poph-fluxos" / "src" / "language.rs")
    if not source.is_file():
        source = ROOT / "crates" / "poph-fluxos" / "src" / "linguagem.rs"
    source = source.read_text(encoding="utf-8")
    names = []
    for constant in (
        "DECLARATIONS",
        "VALUES",
        "EFFECTS",
        "COMBINATORS",
        "CONSTANTS",
        "PURE_FUNCTIONS",
    ):
        found = re.search(rf"pub const {constant}: &\[&str\] = &\[(.*?)\];", source, re.S)
        if found is None:
            legacy = LEGACY_FLOW_CONSTANTS.get(constant)
            if legacy is not None:
                found = re.search(rf"pub const {legacy}: &\[&str\] = &\[(.*?)\];", source, re.S)
        if found is None:
            raise SystemExit(f"language.rs: constant {constant} not found")
        names.extend(re.findall(r'"([^"]+)"', found.group(1)))
    return names


def database_schema():
    connection = sqlite3.connect(":memory:")
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        connection.executescript(migration.read_text(encoding="utf-8"))
    objects = connection.execute(
        "select type, name from sqlite_master where name not like 'sqlite_%' order by type, name"
    ).fetchall()
    tables = [name for kind, name in objects if kind == "table"]
    indexes = [name for kind, name in objects if kind == "index"]
    triggers = [name for kind, name in objects if kind == "trigger"]
    columns = []
    for table in tables:
        lines = connection.execute(f'pragma table_info("{table}")').fetchall()
        columns.extend(line[1] for line in lines)
    connection.close()
    return tables, columns, indexes, triggers


def openapi():
    document = json.loads((CONTRACTS_BASE / "openapi" / "openapi.json").read_text(encoding="utf-8"))
    routes = sorted(document.get("paths", {}))
    operations = set()
    for item in document.get("paths", {}).values():
        for specification in item.values():
            if isinstance(specification, dict) and specification.get("operationId"):
                operations.add(specification["operationId"])
    fields = set()
    for schema in document.get("components", {}).get("schemas", {}).values():
        fields.update((schema.get("properties") or {}).keys())
    return routes, sorted(operations), sorted(fields)


def cli():
    commands = set()
    flags = set()
    for file in sorted((CONTRACTS_BASE / "cli").glob("*.txt")):
        if file.stem.startswith("poph__version"):
            continue
        commands.add(" ".join(file.stem.split("__")).replace("_poph", "poph"))
        for flag in re.findall(r"(?<![\w-])(--?[A-Za-z][A-Za-z0-9-]*)", file.read_text(encoding="utf-8")):
            flags.add(flag)
    return sorted(commands), sorted(flags)


def toml_keys():
    with open(DEFAULT_TOML, "rb") as handle:
        data = tomllib.load(handle)
    keys = []

    def visit(prefix, value):
        for key_name, content in value.items():
            path = f"{prefix}.{key_name}" if prefix else key_name
            keys.append(path)
            if isinstance(content, dict):
                visit(path, content)

    visit("", data)
    return sorted(keys)


def environment_variables():
    pattern = re.compile(r"\b(?:POPH_[A-Z0-9_]+|[A-Z][A-Z0-9_]*_API_KEY)\b")
    visible = repository_paths()
    names = set()
    for file in sorted((ROOT / "docs").glob("*.md")) + [ROOT / "README.md"]:
        if not file.exists():
            continue
        if visible is not None and file.relative_to(ROOT).as_posix() not in visible:
            continue
        names.update(pattern.findall(file.read_text(encoding="utf-8")))
    return sorted(names)


def metrics():
    names = []
    for line in (CONTRACTS_BASE / "metrics" / "types.txt").read_text(encoding="utf-8").splitlines():
        found = re.match(r"# TYPE (\S+) ", line)
        if found:
            names.append(found.group(1))
    return names


def binaries():
    names = set()
    for manifest in sorted(ROOT.glob("crates/*/Cargo.toml")) + [ROOT / "Cargo.toml"]:
        text = manifest.read_text(encoding="utf-8")
        names.update(re.findall(r"\[\[bin\]\]\s*\nname = \"([^\"]+)\"", text))
    for main_file in sorted(ROOT.glob("crates/*/src/main.rs")):
        names.add(main_file.parent.parent.name)
    return sorted(names)


def json_keys(text):
    keys = set()

    def visit(value):
        if isinstance(value, dict):
            for key_name, content in value.items():
                keys.add(key_name)
                visit(content)
        elif isinstance(value, list):
            for item in value:
                visit(item)

    for block in re.findall(r"```json\n(.*?)```", text, re.S):
        try:
            visit(json.loads(block))
        except json.JSONDecodeError:
            continue
    return keys


def plugin_protocol():
    text = (ROOT / "docs" / "PLUGIN_PROTOCOLO.md").read_text(encoding="utf-8")
    return sorted(json_keys(text))


def backup_format():
    text = (ROOT / "docs" / "BACKUP.md").read_text(encoding="utf-8")
    fields = set()
    for line in text.splitlines():
        if not line.startswith("| `"):
            continue
        first = line.split("|")[1]
        found = re.search(r"`([^`]+)`", first)
        if found:
            fields.add(found.group(1))
    return sorted(fields)


def published_inventory():
    tables, columns, indexes, triggers = database_schema()
    routes, operations, json_fields = openapi()
    commands, flags = cli()
    return {
        "linguagem_fluxos": flow_language_names(),
        "binario": binaries(),
        "variavel_ambiente": environment_variables(),
        "metrica": metrics(),
        "tabela": tables,
        "coluna": columns,
        "indice": indexes,
        "gatilho": triggers,
        "rota": routes,
        "operation_id": operations,
        "campo_json": json_fields,
        "comando_cli": commands,
        "flag_cli": flags,
        "chave_toml": toml_keys(),
        "protocolo_plugin": plugin_protocol(),
        "backup": backup_format(),
    }


def frozen_terms(inventory, mapping):
    """Portuguese term to the scopes where it is a published name.

    The 41 flow language names are frozen whole (the word `mensagem_ao_cliente` is not split: it is
    the one stored in the database). The other published names come in through the terms that
    compose them, and only when the term is Portuguese: a published name in English needs no row in
    the glossary.
    """
    frozen = defaultdict(set)
    for name in inventory["linguagem_fluxos"]:
        frozen[name].add("linguagem de fluxos")
    for kind in PUBLISHED_TYPES:
        if kind == "linguagem_fluxos":
            continue
        for name in inventory[kind]:
            for term in split_term(name):
                if term not in mapping:
                    continue
                frozen[term].add(f"{kind} {name}")
    return {term: sorted(scopes) for term, scopes in frozen.items()}


def cell_where(scopes, limit=3):
    shown = scopes[:limit]
    text = "; ".join(shown)
    if len(scopes) > limit:
        text += f"; e mais {len(scopes) - limit}"
    return text.replace("|", "/")


def table_rows(measurement, mapping, frozen):
    lines = []
    for term, english in mapping.items():
        count = measurement.count.get(term, 0)
        if count == 0:
            continue
        lines.append(
            (term, english, "traduz", measurement.areas_of(term), str(count), -count)
        )
    lines.sort(key=lambda line: (line[5], line[0]))
    translated = [[line[0], line[1], line[2], line[3], line[4]] for line in lines]
    frozen_rows = []
    for term in sorted(frozen):
        frozen_rows.append([term, "—", "congelado", cell_where(frozen[term]), "-"])
    return translated + frozen_rows


def markdown_table(lines):
    parts = [TABLE_START, HEADER, SEPARATOR]
    for line in lines:
        parts.append("| " + " | ".join(line) + " |")
    parts.append(TABLE_END)
    return "\n".join(parts)


def generated_document(measurement, mapping, frozen):
    return markdown_table(table_rows(measurement, mapping, frozen))


def replace_region(document, table):
    region_start = document.find(TABLE_START)
    region_end = document.find(TABLE_END)
    if region_start < 0 or region_end < 0:
        raise SystemExit(
            "docs/GLOSSARY.md: the markers are missing: "
            f"{TABLE_START} and {TABLE_END}"
        )
    return document[:region_start] + table + document[region_end + len(TABLE_END) :]


def read_table(document):
    region_start = document.find(TABLE_START)
    region_end = document.find(TABLE_END)
    if region_start < 0 or region_end < 0:
        return None, [f"the markers are missing: {TABLE_START} and {TABLE_END}"]
    block = document[region_start + len(TABLE_START) : region_end].strip("\n").splitlines()
    problems = []
    if len(block) < 2:
        return None, ["the table region is empty"]
    if block[0].strip() != HEADER:
        problems.append(f"header differs from the expected one: {block[0].strip()!r}")
    if block[1].strip() != SEPARATOR:
        problems.append("separator line differs from the expected one")
    lines = []
    for number, line in enumerate(block[2:], start=3):
        if not line.startswith("|"):
            problems.append(f"line {number}: not a table line: {line!r}")
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) != 5:
            problems.append(f"line {number}: expected 5 columns, found {len(cells)}: {line!r}")
            continue
        lines.append((number, cells))
    return lines, problems


def validate(measurement, mapping, mapping_errors, inventory, frozen):
    failures = list(mapping_errors)
    document = GLOSSARY.read_text(encoding="utf-8")
    lines, problems = read_table(document)
    failures.extend(problems)
    if lines is None:
        return failures, {"traduz": 0, "congelado": 0}

    seen = set()
    by_decision = Counter()
    occurrences = {}
    for number, cells in lines:
        term, english, decision, where, count = cells
        if decision not in ("traduz", "congelado"):
            failures.append(f"line {number}: invalid decision: {decision!r}")
            continue
        if (term, decision) in seen:
            failures.append(f"line {number}: repeated term for {decision}: {term}")
            continue
        seen.add((term, decision))
        by_decision[decision] += 1
        if not where or where == "-":
            failures.append(f"line {number}: empty where column in {term}")
        if decision == "traduz":
            if not english or english == "—":
                failures.append(f"line {number}: empty translation in {term}")
            if not count.isdigit():
                failures.append(f"line {number}: occurrences of {term} should be a number: {count!r}")
                continue
            occurrences[term] = int(count)

    for term in mapping:
        measured = measurement.count.get(term, 0)
        if measured == 0:
            continue
        if (term, "traduz") not in seen:
            failures.append(f"{term}: it is in the map and has no 'traduz' row in the table")
            continue
        if occurrences.get(term) != measured:
            failures.append(
                f"{term}: the table says {occurrences.get(term)}, the measurement says {measured} (stale table)"
            )

    for term in frozen:
        if (term, "congelado") not in seen:
            failures.append(f"{term}: published name without a 'congelado' row in the table")

    for term in sorted(set(mapping) & set(frozen)):
        if (term, "traduz") not in seen or (term, "congelado") not in seen:
            failures.append(f"{term}: it has both roles and needs both rows")

    language_names = flow_language_names()
    if len(language_names) != 41:
        failures.append(f"linguagem.rs: expected 41 names, found {len(language_names)}")
    for name in language_names:
        if (name, "congelado") not in seen:
            failures.append(f"{name}: flow language word without a 'congelado' row")

    expected = generated_document(measurement, mapping, frozen)
    if expected not in document:
        failures.append("the table on disk differs from the generated one: run --gerar")

    return failures, {"traduz": by_decision["traduz"], "congelado": by_decision["congelado"]}


def print_measurement(measurement, mapping, top, candidates, minimum):
    portuguese = [
        (term, measurement.count[term]) for term in mapping if measurement.count.get(term, 0) > 0
    ]
    portuguese.sort(key=lambda item: (-item[1], item[0]))
    print(f"files read: {measurement.files}")
    print("by language: " + ", ".join(f"{name} {total}" for name, total in sorted(measurement.by_language.items())))
    print(f"distinct terms: {len(measurement.count)}")
    print(f"Portuguese terms in the map, measured: {len(portuguese)}")
    print()
    print("| occurrences | term | where |")
    print("| ---: | --- | --- |")
    for term, count in portuguese[:top]:
        print(f"| {count} | {term} | {measurement.areas_of(term)} |")
    if candidates:
        outside_mapping = [
            (term, count)
            for term, count in measurement.count.items()
            if term not in mapping and count >= minimum
        ]
        outside_mapping.sort(key=lambda item: (-item[1], item[0]))
        print()
        print(f"candidates outside the map (occurrences >= {minimum}): {len(outside_mapping)}")
        print(", ".join(f"{term}:{count}" for term, count in outside_mapping))


def print_published(inventory, mapping, frozen):
    for kind in PUBLISHED_TYPES:
        names = inventory[kind]
        terms = sorted({term for term in frozen if any(f"{kind} {name}" in frozen[term] for name in names)})
        print(f"== {kind}: {len(names)} name(s), {len(terms)} frozen Portuguese term(s)")
        examples = names[:6]
        print("   " + ", ".join(examples) + (" …" if len(names) > len(examples) else ""))
        if terms:
            print("   terms: " + ", ".join(terms))
    print()
    print(f"frozen terms in total: {len(frozen)}")
    print(f"translated terms: {len(mapping)}")


def main(argv):
    parser = argparse.ArgumentParser(
        prog="glossary-terms.py",
        description="Measures the identifier terms, extracts the published names and validates docs/GLOSSARY.md.",
    )
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument(
        "--medir", dest="measure", action="store_true", help="counts the terms and lists the most frequent ones"
    )
    group.add_argument(
        "--publicados", dest="published", action="store_true", help="lists the published names by kind"
    )
    group.add_argument(
        "--gerar", dest="generate", action="store_true", help="rewrites the table of docs/GLOSSARY.md"
    )
    group.add_argument(
        "--validar", dest="validate", action="store_true", help="checks the table against the measurement"
    )
    parser.add_argument("--top", type=int, default=40, help="how many rows to show with --medir (default: 40)")
    parser.add_argument(
        "--minimo",
        dest="minimum",
        type=int,
        default=20,
        help="cut of the candidates with --medir (default: 20)",
    )
    parser.add_argument(
        "--candidatos",
        dest="candidates",
        action="store_true",
        help="with --medir, lists the terms outside the map",
    )
    arguments = parser.parse_args(argv)

    mapping, mapping_errors = read_mapping()
    if arguments.measure:
        measurement = measure()
        print_measurement(measurement, mapping, arguments.top, arguments.candidates, arguments.minimum)
        return 0

    inventory = published_inventory()
    frozen = frozen_terms(inventory, mapping)

    if arguments.published:
        print_published(inventory, mapping, frozen)
        return 0

    measurement = measure()

    if arguments.generate:
        document = GLOSSARY.read_text(encoding="utf-8")
        new = replace_region(document, generated_document(measurement, mapping, frozen))
        GLOSSARY.write_text(new, encoding="utf-8")
        lines = table_rows(measurement, mapping, frozen)
        print(f"glossary: {len(lines)} row(s) written to docs/GLOSSARY.md")
        return 0

    failures, count = validate(measurement, mapping, mapping_errors, inventory, frozen)
    if failures:
        for failure in failures[:80]:
            print(f"glossary: {failure}", file=sys.stderr)
        if len(failures) > 80:
            print(f"glossary: and {len(failures) - 80} more problem(s)", file=sys.stderr)
        print(f"glossary: {len(failures)} problem(s)", file=sys.stderr)
        return 1
    flow_names = len(flow_language_names())
    print(
        "glossary: OK "
        f"({count['traduz']} translated terms, {count['congelado']} frozen, "
        f"{flow_names} flow language words, {measurement.files} files measured)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
