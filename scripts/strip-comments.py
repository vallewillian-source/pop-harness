#!/usr/bin/env python3
import argparse
import bisect
import difflib
import fnmatch
import sys
from dataclasses import dataclass
from pathlib import Path

ALLOW_LIST_DEFAULT = Path("scripts/allow-comments.txt")

SKIP_DIRS = {
    ".git",
    ".cache",
    ".venv",
    "__pycache__",
    "build",
    "dist",
    "node_modules",
    "spike",
    "target",
    "venv",
}

SKIP_PATHS = {
    "console/src/api/schema.d.ts",
    "Cargo.lock",
    "pnpm-lock.yaml",
}

EXTENSIONS = {
    ".bash": "shell",
    ".cjs": "javascript",
    ".css": "css",
    ".cts": "javascript",
    ".html": "html",
    ".js": "javascript",
    ".jsx": "javascript",
    ".mjs": "javascript",
    ".mts": "javascript",
    ".py": "python",
    ".rs": "rust",
    ".sh": "shell",
    ".sql": "sql",
    ".star": "python",
    ".template": "yaml",
    ".toml": "toml",
    ".ts": "javascript",
    ".tsx": "javascript",
    ".yaml": "yaml",
    ".yml": "yaml",
    ".zsh": "shell",
}

DOTFILE_LANGUAGES = {
    ".dockerignore": "dockerfile",
    ".editorconfig": "hash",
    ".gitattributes": "hash",
    ".gitignore": "hash",
}

RUST_IDENT = set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_")
JS_REGEX_CHARS = set("(,=:[!&|?{};+-*%~^<>")
JS_REGEX_WORDS = {
    "await",
    "case",
    "delete",
    "do",
    "else",
    "in",
    "instanceof",
    "new",
    "of",
    "return",
    "throw",
    "typeof",
    "void",
    "yield",
}
PY_PREFIX_CHARS = set("rbufRBUF")
SHELL_WORD_BREAK = set(" \t\n;|&(){}<>`")
HEREDOC_NAME = set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_")


@dataclass
class Span:
    start: int
    end: int
    block: bool
    marker: str


@dataclass
class AllowRule:
    pattern: str
    kind: str
    match: str
    reason: str


@dataclass
class Finding:
    path: str
    line: int
    column: int
    marker: str
    text: str


def line_starts(source):
    starts = [0]
    position = source.find("\n")
    while position >= 0:
        starts.append(position + 1)
        position = source.find("\n", position + 1)
    return starts


def locate(starts, offset):
    index = bisect.bisect_right(starts, offset) - 1
    return index + 1, offset - starts[index] + 1


def scan_escaped(source, start, quote):
    size = len(source)
    index = start
    while index < size:
        char = source[index]
        if char == "\\":
            index += 2
            continue
        if char == quote:
            return index + 1
        index += 1
    return size


def scan_rust_raw(source, start):
    size = len(source)
    index = start
    if index < size and source[index] == "b":
        index += 1
    if index >= size or source[index] != "r":
        return None
    if start > 0 and source[start - 1] in RUST_IDENT:
        return None
    index += 1
    hashes = 0
    while index < size and source[index] == "#":
        hashes += 1
        index += 1
    if index >= size or source[index] != '"':
        return None
    closing = '"' + "#" * hashes
    end = source.find(closing, index + 1)
    return size if end < 0 else end + len(closing)


def scan_rust_quote(source, start):
    size = len(source)
    index = start + 1
    if index >= size:
        return start + 1
    if source[index] == "\\":
        while index < size:
            if source[index] == "\\":
                index += 2
                continue
            if source[index] == "'":
                return index + 1
            if source[index] == "\n":
                break
            index += 1
        return start + 1
    if index + 1 < size and source[index + 1] == "'" and source[index] != "'" and source[index] != "\n":
        return index + 2
    while index < size and source[index] in RUST_IDENT:
        index += 1
    return index


def scan_rust(source):
    spans = []
    size = len(source)
    index = 0
    while index < size:
        char = source[index]
        if char == "/" and source.startswith("//", index):
            end = source.find("\n", index)
            end = size if end < 0 else end
            spans.append(Span(index, end, False, "//"))
            index = end
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
            spans.append(Span(index, cursor, True, "/*"))
            index = cursor
            continue
        raw_end = scan_rust_raw(source, index)
        if raw_end is not None:
            index = raw_end
            continue
        if char == '"':
            index = scan_escaped(source, index + 1, '"')
            continue
        if char == "'":
            index = scan_rust_quote(source, index)
            continue
        index += 1
    return spans


def scan_js_regex(source, start):
    size = len(source)
    index = start + 1
    in_class = False
    while index < size:
        char = source[index]
        if char == "\\":
            index += 2
            continue
        if char == "\n":
            return None
        if in_class:
            if char == "]":
                in_class = False
            index += 1
            continue
        if char == "[":
            in_class = True
            index += 1
            continue
        if char == "/":
            index += 1
            while index < size and source[index].isalpha():
                index += 1
            return index
        index += 1
    return None


def scan_js_template(source, start, spans):
    size = len(source)
    index = start + 1
    while index < size:
        char = source[index]
        if char == "\\":
            index += 2
            continue
        if char == "`":
            return index + 1
        if char == "$" and source.startswith("${", index):
            index = scan_js_code(source, index + 2, spans, True)
            continue
        index += 1
    return size


def scan_js_code(source, start, spans, closes_brace):
    size = len(source)
    index = start
    depth = 0
    previous = ""
    while index < size:
        char = source[index]
        if char == "}" and closes_brace and depth == 0:
            return index + 1
        if char == "{":
            depth += 1
            previous = "{"
            index += 1
            continue
        if char == "}":
            if depth > 0:
                depth -= 1
            previous = "}"
            index += 1
            continue
        if char.isspace():
            index += 1
            continue
        if char == "/" and source.startswith("//", index):
            end = source.find("\n", index)
            end = size if end < 0 else end
            spans.append(Span(index, end, False, "//"))
            index = end
            continue
        if char == "/" and source.startswith("/*", index):
            end = source.find("*/", index + 2)
            end = size if end < 0 else end + 2
            spans.append(Span(index, end, True, "/*"))
            index = end
            continue
        if char == "/":
            if previous == "" or previous in JS_REGEX_CHARS or previous in JS_REGEX_WORDS:
                end = scan_js_regex(source, index)
                if end is not None:
                    index = end
                    previous = "regex"
                    continue
            previous = "/"
            index += 1
            continue
        if char == "`":
            index = scan_js_template(source, index, spans)
            previous = "string"
            continue
        if char in "\"'":
            index = scan_escaped(source, index + 1, char)
            previous = "string"
            continue
        if char.isalpha() or char in "_$":
            word_start = index
            while index < size and (source[index].isalnum() or source[index] in "_$"):
                index += 1
            previous = source[word_start:index]
            continue
        previous = char
        index += 1
    return index


def scan_javascript(source):
    spans = []
    scan_js_code(source, 0, spans, False)
    return spans


def scan_css(source):
    spans = []
    size = len(source)
    index = 0
    while index < size:
        char = source[index]
        if char == "/" and source.startswith("/*", index):
            end = source.find("*/", index + 2)
            end = size if end < 0 else end + 2
            spans.append(Span(index, end, True, "/*"))
            index = end
            continue
        if char in "\"'":
            index = scan_escaped(source, index + 1, char)
            continue
        index += 1
    return spans


def scan_sql(source):
    spans = []
    size = len(source)
    index = 0
    while index < size:
        char = source[index]
        if char == "-" and source.startswith("--", index):
            end = source.find("\n", index)
            end = size if end < 0 else end
            spans.append(Span(index, end, False, "--"))
            index = end
            continue
        if char == "/" and source.startswith("/*", index):
            end = source.find("*/", index + 2)
            end = size if end < 0 else end + 2
            spans.append(Span(index, end, True, "/*"))
            index = end
            continue
        if char in "'\"`":
            cursor = index + 1
            while cursor < size:
                if source[cursor] == char:
                    if cursor + 1 < size and source[cursor + 1] == char:
                        cursor += 2
                        continue
                    cursor += 1
                    break
                cursor += 1
            index = cursor
            continue
        if char == "[":
            end = source.find("]", index)
            index = size if end < 0 else end + 1
            continue
        index += 1
    return spans


def parse_heredoc(source, start):
    size = len(source)
    index = start + 2
    strip_tabs = False
    if index < size and source[index] == "-":
        strip_tabs = True
        index += 1
    while index < size and source[index] in " \t":
        index += 1
    quote = ""
    if index < size and source[index] in "'\"":
        quote = source[index]
        index += 1
    name_start = index
    while index < size and source[index] not in " \t\n":
        if quote and source[index] == quote:
            break
        index += 1
    delimiter = source[name_start:index]
    if not delimiter or delimiter[0] not in HEREDOC_NAME:
        return None
    if any(char not in HEREDOC_NAME for char in delimiter):
        return None
    if quote:
        index += 1
    return index, delimiter, strip_tabs


def skip_heredocs(source, start, heredocs):
    size = len(source)
    index = start
    for delimiter, strip_tabs in heredocs:
        while index < size:
            end = source.find("\n", index)
            line_end = size if end < 0 else end
            line = source[index:line_end]
            candidate = line.lstrip("\t") if strip_tabs else line
            index = size if end < 0 else end + 1
            if candidate == delimiter:
                break
    return index


def scan_shell(source):
    spans = []
    size = len(source)
    frames = []
    heredocs = []
    index = 0
    while index < size:
        char = source[index]
        frame = frames[-1] if frames else "root"
        if frame == "single":
            if char == "'":
                frames.pop()
            index += 1
            continue
        if frame == "ansi":
            if char == "\\":
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
            if source.startswith("$(", index):
                frames.append("command")
                index += 2
                continue
            if source.startswith("${", index):
                frames.append("parameter")
                index += 2
                continue
            if char == "`":
                frames.append("backtick")
                index += 1
                continue
            index += 1
            continue
        if frame == "parameter":
            if char == "\\":
                index += 2
                continue
            if char == "}":
                frames.pop()
                index += 1
                continue
            if char == "'":
                frames.append("single")
                index += 1
                continue
            if char == '"':
                frames.append("double")
                index += 1
                continue
            if source.startswith("$(", index):
                frames.append("command")
                index += 2
                continue
            if source.startswith("${", index):
                frames.append("parameter")
                index += 2
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
        if char == "\\":
            index += 2
            continue
        if char == "'":
            frames.append("single")
            index += 1
            continue
        if char == '"':
            frames.append("double")
            index += 1
            continue
        if source.startswith("$'", index):
            frames.append("ansi")
            index += 2
            continue
        if source.startswith("$(", index):
            frames.append("command")
            index += 2
            continue
        if source.startswith("${", index):
            frames.append("parameter")
            index += 2
            continue
        if char == "`":
            frames.append("backtick")
            index += 1
            continue
        if char == ")" and frame == "command":
            frames.pop()
            index += 1
            continue
        if char == "\n":
            index += 1
            if heredocs:
                index = skip_heredocs(source, index, heredocs)
                heredocs = []
            continue
        if source.startswith("<<", index) and not source.startswith("<<<", index):
            heredoc = parse_heredoc(source, index)
            if heredoc is not None:
                index, delimiter, strip_tabs = heredoc
                heredocs.append((delimiter, strip_tabs))
                continue
        if char == "#" and (index == 0 or source[index - 1] in SHELL_WORD_BREAK):
            end = source.find("\n", index)
            end = size if end < 0 else end
            spans.append(Span(index, end, False, "#"))
            index = end
            continue
        index += 1
    return spans


def python_prefix(source, quote_index):
    letters = []
    index = quote_index - 1
    while index >= 0 and source[index] in PY_PREFIX_CHARS and len(letters) < 2:
        letters.append(source[index])
        index -= 1
    if index >= 0 and (source[index].isalnum() or source[index] == "_"):
        return ""
    return "".join(reversed(letters)).lower()


def scan_fstring_expression(source, start):
    size = len(source)
    index = start
    depth = 1
    while index < size:
        char = source[index]
        if char in "'\"":
            index = scan_python_string(source, index, python_prefix(source, index))
            continue
        if char == "{":
            depth += 1
            index += 1
            continue
        if char == "}":
            depth -= 1
            index += 1
            if depth == 0:
                return index
            continue
        index += 1
    return size


def scan_python_string(source, quote_index, prefix):
    size = len(source)
    quote = source[quote_index]
    triple = source.startswith(quote * 3, quote_index)
    terminator = quote * (3 if triple else 1)
    raw = "r" in prefix
    formatted = "f" in prefix
    index = quote_index + len(terminator)
    while index < size:
        char = source[index]
        if not raw and char == "\\":
            index += 2
            continue
        if source.startswith(terminator, index):
            return index + len(terminator)
        if not triple and char == "\n":
            return index
        if formatted and char == "{":
            if source.startswith("{{", index):
                index += 2
                continue
            index = scan_fstring_expression(source, index + 1)
            continue
        if formatted and source.startswith("}}", index):
            index += 2
            continue
        index += 1
    return size


def scan_python(source):
    spans = []
    size = len(source)
    index = 0
    while index < size:
        char = source[index]
        if char == "#":
            end = source.find("\n", index)
            end = size if end < 0 else end
            spans.append(Span(index, end, False, "#"))
            index = end
            continue
        if char in "'\"":
            index = scan_python_string(source, index, python_prefix(source, index))
            continue
        index += 1
    return spans


def scan_toml(source):
    spans = []
    size = len(source)
    index = 0
    while index < size:
        char = source[index]
        if char == "#":
            end = source.find("\n", index)
            end = size if end < 0 else end
            spans.append(Span(index, end, False, "#"))
            index = end
            continue
        if char == '"' and source.startswith('"""', index):
            index += 3
            while index < size:
                if source[index] == "\\":
                    index += 2
                    continue
                if source.startswith('"""', index):
                    index += 3
                    break
                index += 1
            continue
        if char == "'" and source.startswith("'''", index):
            end = source.find("'''", index + 3)
            index = size if end < 0 else end + 3
            continue
        if char == "'":
            end = source.find("'", index + 1)
            index = size if end < 0 else end + 1
            continue
        if char == '"':
            index = scan_escaped(source, index + 1, '"')
            continue
        index += 1
    return spans


def yaml_quote_start(source, index):
    cursor = index - 1
    while cursor >= 0 and source[cursor] in " \t":
        cursor -= 1
    if cursor < 0:
        return True
    char = source[cursor]
    if char in ":[{,-?":
        return True
    if char == "\n":
        return True
    return False


def scan_yaml(source):
    spans = []
    size = len(source)
    index = 0
    while index < size:
        char = source[index]
        if char == "#" and (index == 0 or source[index - 1] in " \t\n"):
            end = source.find("\n", index)
            end = size if end < 0 else end
            spans.append(Span(index, end, False, "#"))
            index = end
            continue
        if char == "'" and yaml_quote_start(source, index):
            cursor = index + 1
            while cursor < size:
                if source[cursor] == "'":
                    if cursor + 1 < size and source[cursor + 1] == "'":
                        cursor += 2
                        continue
                    cursor += 1
                    break
                cursor += 1
            index = cursor
            continue
        if char == '"' and yaml_quote_start(source, index):
            index = scan_escaped(source, index + 1, '"')
            continue
        index += 1
    return spans


def scan_line_marks(source, markers):
    spans = []
    size = len(source)
    index = 0
    while index < size:
        end = source.find("\n", index)
        line_end = size if end < 0 else end
        line = source[index:line_end]
        stripped = len(line) - len(line.lstrip(" \t"))
        if stripped < len(line) and line[stripped] in markers:
            spans.append(Span(index + stripped, line_end, False, line[stripped]))
        index = size if end < 0 else end + 1
    return spans


def scan_dockerfile(source):
    return scan_line_marks(source, "#")


def scan_hash(source):
    return scan_line_marks(source, "#;")


def scan_html(source):
    spans = []
    size = len(source)
    index = 0
    while index < size:
        start = source.find("<!--", index)
        if start < 0:
            break
        end = source.find("-->", start + 4)
        end = size if end < 0 else end + 3
        spans.append(Span(start, end, True, "<!--"))
        index = end
    return spans


SCANNERS = {
    "css": scan_css,
    "dockerfile": scan_dockerfile,
    "hash": scan_hash,
    "html": scan_html,
    "javascript": scan_javascript,
    "python": scan_python,
    "rust": scan_rust,
    "shell": scan_shell,
    "sql": scan_sql,
    "toml": scan_toml,
    "yaml": scan_yaml,
}


def is_skipped(path):
    posix = path.as_posix()
    for skipped in SKIP_PATHS:
        if posix == skipped or posix.endswith("/" + skipped):
            return True
    return False


def language_of(path):
    name = path.name
    if name in DOTFILE_LANGUAGES:
        return DOTFILE_LANGUAGES[name]
    if name == "Dockerfile" or name.startswith("Dockerfile.") or name.endswith(".Dockerfile"):
        return "dockerfile"
    return EXTENSIONS.get(path.suffix.lower())


def collect_files(paths):
    files = []
    for raw in paths:
        path = Path(raw)
        if not path.exists():
            raise FileNotFoundError(raw)
        if path.is_file():
            files.append(path)
            continue
        for candidate in sorted(path.rglob("*")):
            if not candidate.is_file():
                continue
            if any(part in SKIP_DIRS for part in candidate.parts):
                continue
            if is_skipped(candidate):
                continue
            if language_of(candidate) is None:
                continue
            files.append(candidate)
    unique = []
    seen = set()
    for path in files:
        key = path.as_posix()
        if key in seen:
            continue
        seen.add(key)
        unique.append(path)
    return unique


def load_allow_list(path):
    rules = []
    if not path.exists():
        return rules
    for raw in path.read_text(encoding="utf-8").splitlines():
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        parts = raw.split("\t")
        if len(parts) < 4:
            raise ValueError(f"linha inválida na allowlist: {raw!r}")
        rules.append(AllowRule(parts[0].strip(), parts[1].strip(), parts[2].strip(), parts[3].strip()))
    return rules


def comment_text(source, span):
    return source[span.start:span.end]


def inner_text(source, span):
    raw = source[span.start:span.end]
    if span.block:
        marker = span.marker
        closing = {"/*": "*/", "<!--": "-->"}.get(marker)
        body = raw[len(marker):]
        if closing and body.endswith(closing):
            body = body[: -len(closing)]
        return body
    return raw[len(span.marker):]


def rule_matches(rule, relative, starts, span, source, is_first_line_shebang):
    if not fnmatch.fnmatch(relative, rule.pattern):
        return False
    line, _ = locate(starts, span.start)
    if rule.kind == "shebang":
        return is_first_line_shebang
    if rule.kind == "directive":
        body = inner_text(source, span).strip()
        return body.startswith(rule.match)
    if rule.kind == "data":
        match = rule.match
        if match.startswith("line:"):
            return line == int(match[len("line:"):])
        if match.startswith("lines:"):
            low, _, high = match[len("lines:"):].partition("-")
            end_line, _ = locate(starts, max(span.end - 1, span.start))
            return int(low) <= line and end_line <= int(high)
        if match.startswith("starts:"):
            return inner_text(source, span).strip().startswith(match[len("starts:"):])
        if match.startswith("contains:"):
            return match[len("contains:"):] in inner_text(source, span)
        return False
    return False


def allowed(rule_list, relative, starts, span, source, is_first_line_shebang):
    for rule in rule_list:
        if rule_matches(rule, relative, starts, span, source, is_first_line_shebang):
            return True
    return False


def strip_source(source, spans, keep_ids):
    limit = len(source)
    removed = bytearray(limit)
    for span in spans:
        if id(span) in keep_ids:
            continue
        for offset in range(span.start, min(span.end, limit)):
            removed[offset] = 1
    output = []
    index = 0
    while index < limit:
        end = source.find("\n", index)
        line_end = limit if end < 0 else end
        has_removal = False
        kept_chars = []
        for offset in range(index, line_end):
            if removed[offset]:
                has_removal = True
            else:
                kept_chars.append(source[offset])
        kept = "".join(kept_chars)
        if not has_removal:
            output.append(source[index:line_end])
        elif kept.strip():
            body = kept.rstrip("\r")
            output.append(body.rstrip(" \t") + kept[len(body):])
        if end < 0:
            index = limit
            continue
        if not has_removal or kept.strip():
            output.append("\n")
        index = end + 1
    return "".join(output)


def describe(relative, starts, span, source):
    line, column = locate(starts, span.start)
    text = " ".join(comment_text(source, span).split())
    if len(text) > 100:
        text = text[:97] + "..."
    return Finding(relative, line, column, span.marker, text)


def normalize_spans(source, spans):
    for span in spans:
        if span.block:
            continue
        while span.end > span.start and source[span.end - 1] == "\r":
            span.end -= 1
    return spans


def process_file(path, rule_list):
    raw = path.read_text(encoding="utf-8", newline="")
    language = language_of(path)
    spans = normalize_spans(raw, SCANNERS[language](raw))
    starts = line_starts(raw)
    relative = path.as_posix()
    shebang = raw.startswith("#!")
    keep_ids = set()
    for span in spans:
        if allowed(rule_list, relative, starts, span, raw, shebang and span.start == 0):
            keep_ids.add(id(span))
    return raw, spans, keep_ids, starts


def format_diff(relative, before, after):
    lines = list(
        difflib.unified_diff(
            before.splitlines(),
            after.splitlines(),
            fromfile=f"a/{relative}",
            tofile=f"b/{relative}",
            lineterm="",
        )
    )
    return "\n".join(lines)


def main(argv):
    parser = argparse.ArgumentParser(
        prog="strip-comments.py",
        description="Remove comentário respeitando a sintaxe de cada linguagem, com allowlist versionada.",
    )
    parser.add_argument("paths", nargs="*", default=["."], help="arquivos ou diretórios (padrão: .)")
    parser.add_argument("--check", action="store_true", help="lista o comentário fora da allowlist e sai diferente de zero")
    parser.add_argument("--dry-run", action="store_true", help="mostra a contagem (e o diff, com --diff) sem gravar")
    parser.add_argument("--diff", action="store_true", help="com --dry-run, imprime o diff unificado")
    parser.add_argument("--allow-list", default=str(ALLOW_LIST_DEFAULT), help="caminho da allowlist")
    parser.add_argument("--no-allow-list", action="store_true", help="ignora a allowlist")
    arguments = parser.parse_args(argv)

    if arguments.check and arguments.dry_run:
        print("strip-comments: --check e --dry-run são exclusivos", file=sys.stderr)
        return 2

    allow_path = Path(arguments.allow_list)
    try:
        rule_list = [] if arguments.no_allow_list else load_allow_list(allow_path)
    except ValueError as error:
        print(f"strip-comments: {error}", file=sys.stderr)
        return 2

    try:
        files = collect_files(arguments.paths or ["."])
    except FileNotFoundError as error:
        print(f"strip-comments: caminho não encontrado: {error}", file=sys.stderr)
        return 2

    findings = []
    changed = []
    allowed_count = 0
    read_count = 0
    for path in files:
        if is_skipped(path):
            print(f"strip-comments: ignorado (arquivo gerado): {path.as_posix()}", file=sys.stderr)
            continue
        if language_of(path) is None:
            print(f"strip-comments: ignorado (linguagem não reconhecida): {path.as_posix()}", file=sys.stderr)
            continue
        raw, spans, keep_ids, starts = process_file(path, rule_list)
        read_count += 1
        allowed_count += len(keep_ids)
        pending = [span for span in spans if id(span) not in keep_ids]
        if arguments.check:
            for span in pending:
                findings.append(describe(path.as_posix(), starts, span, raw))
            continue
        if not pending:
            continue
        stripped = strip_source(raw, spans, keep_ids)
        if stripped == raw:
            continue
        changed.append((path, raw, stripped))
        if arguments.dry_run:
            removed = raw.count("\n") - stripped.count("\n")
            print(f"{path.as_posix()}: {len(pending)} comentário(s), {removed} linha(s) removida(s)")
            if arguments.diff:
                print(format_diff(path.as_posix(), raw, stripped))
            continue
        path.write_text(stripped, encoding="utf-8", newline="")
        print(f"{path.as_posix()}: {len(pending)} comentário(s) removido(s)")

    if arguments.check:
        for finding in findings:
            print(f"{finding.path}:{finding.line}:{finding.column}: comentário fora da allowlist: {finding.text}")
        if findings:
            print(f"comentários: {len(findings)} fora da allowlist em {read_count} arquivo(s) lido(s)")
            return 1
        print(f"comentários: nenhum fora da allowlist ({read_count} arquivo(s) lido(s), {allowed_count} exceção(ões) aceita(s))")
        return 0

    if arguments.dry_run:
        print(f"seco: {len(changed)} arquivo(s) mudariam, {read_count} arquivo(s) lido(s), {allowed_count} exceção(ões) preservada(s)")
        return 0

    print(f"comentários: {len(changed)} arquivo(s) alterado(s), {read_count} arquivo(s) lido(s), {allowed_count} exceção(ões) preservada(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
