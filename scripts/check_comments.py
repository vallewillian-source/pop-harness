import argparse
import importlib.util
import os
import re
import subprocess
import sys
from pathlib import Path

PENDING_DEFAULT = "scripts/pendentes-comentario.txt"
ALLOW_LIST_DEFAULT = "scripts/allow-comments.txt"
SCANNER_DEFAULT = "scripts/strip-comments.py"
UNKNOWN_MODULE = "-"
DESIGN_ASSETS_SCRIPTS = re.compile(r"^design_assets/[^/]+/scripts/")
PENDING_HEADER = (
    "# Arquivos com comentário fora da allowlist, um por linha: módulo, quantidade de comentários e caminho,",
    "# separados por TAB. Gerado por `bash scripts/check-comments.sh --atualizar-pendentes`; não editar à mão.",
)
MODULE_RULES = (
    ("Cargo.toml", "M00"),
    ("deny.toml", "M00"),
    ("rust-toolchain.toml", "M00"),
    (".github/", "M00"),
    ("crates/poph-types/", "M01"),
    ("crates/poph-core/", "M02"),
    ("crates/poph-plugins/", "M02"),
    ("crates/poph-plugin-eco/", "M02"),
    ("crates/poph-files/", "M03"),
    ("crates/poph-evolution/", "M03"),
    ("crates/poph-rag/", "M04"),
    ("crates/poph-jev/", "M05"),
    ("crates/poph-flows/src/", "M06a"),
    ("crates/poph-flows/", "M06b"),
    ("crates/poph-matrix/src/", "M07a"),
    ("crates/poph-matrix/", "M07b"),
    ("crates/poph-store/src/repos/", "M08b"),
    ("crates/poph-store/migrations/", "M08c"),
    ("crates/poph-store/src/", "M08a"),
    ("crates/poph-store/tests/", "M08d"),
    ("crates/poph-store/", "M08a"),
    ("crates/poph-retention/", "M09"),
    ("crates/poph-agent/src/", "M10a"),
    ("crates/poph-agent/", "M10b"),
    ("crates/poph-server/src/", "M11a-c"),
    ("crates/poph-server/tests/", "M11d"),
    ("crates/poph-server/", "M11a-c"),
    ("crates/poph/src/", "M12a"),
    ("crates/poph/tests/", "M12b"),
    ("crates/poph/", "M12a"),
    ("console/src/api/", "M13a"),
    ("console/src/components/", "M13b"),
    ("console/src/screens/", "M13c"),
    ("console/", "M13d"),
    ("scripts/", "M14"),
    ("tests/fluxos/", "M06b"),
    ("tests/matrix/", "M07b"),
)


def module_of(relative):
    if "/" not in relative:
        return "M00"
    if DESIGN_ASSETS_SCRIPTS.match(relative):
        return "M14"
    for pattern, module in MODULE_RULES:
        if pattern.endswith("/"):
            if relative.startswith(pattern):
                return module
        elif relative == pattern:
            return module
    return UNKNOWN_MODULE


def load_scanner(path):
    spec = importlib.util.spec_from_file_location("strip_comments", path)
    if spec is None or spec.loader is None:
        raise ValueError(f"could not load the comment scanner at {path.as_posix()}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def read_pending(path):
    listed = {}
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) != 3:
            raise ValueError(f"line {number}: expected module, count and path separated by TAB: {line!r}")
        module, count, relative = parts[0].strip(), parts[1].strip(), parts[2].strip()
        if not count.isdigit():
            raise ValueError(f"line {number}: invalid count: {count!r}")
        if not relative:
            raise ValueError(f"line {number}: empty path")
        if relative in listed:
            raise ValueError(f"line {number}: repeated path: {relative}")
        listed[relative] = int(count)
    return listed


def write_pending(path, dirty):
    lines = list(PENDING_HEADER)
    for relative in sorted(dirty, key=lambda item: (module_of(item), item)):
        lines.append(f"{module_of(relative)}\t{dirty[relative]}\t{relative}")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def repository_paths(root):
    try:
        result = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
            capture_output=True,
            check=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return None
    names = result.stdout.decode("utf-8", "surrogateescape").split("\0")
    return {name for name in names if name}


def scan(scanner, rules, paths, visible=None):
    dirty = {}
    findings = []
    matched = [0] * len(rules)
    for path in scanner.collect_files(paths):
        if scanner.is_skipped(path):
            continue
        if scanner.language_of(path) is None:
            continue
        relative = path.as_posix()
        if visible is not None and relative not in visible:
            continue
        raw, spans, keep_ids, starts = scanner.process_file(path, rules)
        for span in spans:
            if id(span) not in keep_ids:
                continue
            shebang = span.start == 0 and raw.startswith("#!")
            for index, rule in enumerate(rules):
                if scanner.rule_matches(rule, relative, starts, span, raw, shebang):
                    matched[index] += 1
                    break
        resting = [span for span in spans if id(span) not in keep_ids]
        if not resting:
            continue
        dirty[relative] = len(resting)
        for span in resting:
            findings.append(scanner.describe(relative, starts, span, raw))
    return dirty, findings, matched


def orphan_rules(rules, matched):
    return [(rule, matched[index]) for index, rule in enumerate(rules) if "/" in rule.pattern and matched[index] == 0]


def main(argv):
    parser = argparse.ArgumentParser(
        prog="check-comments.sh",
        description="CI gate: fails on a comment outside the allowlist and outside the per-module pending list.",
    )
    parser.add_argument(
        "paths", nargs="*", help="paths to check; without them, checks the repository against the pending list"
    )
    parser.add_argument(
        "--atualizar-pendentes",
        dest="update_pending",
        action="store_true",
        help="regenerates the pending list from the tree",
    )
    parser.add_argument(
        "--pendentes",
        dest="pending",
        default=PENDING_DEFAULT,
        help=f"pending list (default: {PENDING_DEFAULT})",
    )
    parser.add_argument(
        "--allow-list",
        default=ALLOW_LIST_DEFAULT,
        help=f"allowlist of the exceptions (default: {ALLOW_LIST_DEFAULT})",
    )
    parser.add_argument("--root", default=".", help="repository root (default: .)")
    parser.add_argument(
        "--scanner", default=SCANNER_DEFAULT, help=f"comment scanner (default: {SCANNER_DEFAULT})"
    )
    arguments = parser.parse_args(argv)

    if arguments.update_pending and arguments.paths:
        print("check-comments: --atualizar-pendentes does not take paths", file=sys.stderr)
        return 2

    root = Path(arguments.root).resolve()
    if not root.is_dir():
        print(f"check-comments: root not found: {root.as_posix()}", file=sys.stderr)
        return 2
    scanner_path = Path(arguments.scanner)
    if not scanner_path.is_absolute():
        scanner_path = root / scanner_path
    try:
        scanner = load_scanner(scanner_path)
    except (OSError, ValueError) as error:
        print(f"check-comments: {error}", file=sys.stderr)
        return 2

    os.chdir(root)

    try:
        rules = scanner.load_allow_list(Path(arguments.allow_list))
    except ValueError as error:
        print(f"check-comments: allowlist: {error}", file=sys.stderr)
        return 2

    try:
        dirty, findings, matched = scan(scanner, rules, arguments.paths or ["."], repository_paths(root))
    except FileNotFoundError as error:
        print(f"check-comments: path not found: {error}", file=sys.stderr)
        return 2

    if not arguments.paths and not arguments.update_pending:
        orphans = orphan_rules(rules, matched)
        if orphans:
            for rule, _ in orphans:
                print(
                    f"check-comments: allowlist: the exception matches no comment any more: "
                    f"{rule.pattern} [{rule.kind} {rule.match}] {rule.reason}",
                    file=sys.stderr,
                )
            print("check-comments: remove the exception or fix the path", file=sys.stderr)
            return 2

    pending_path = Path(arguments.pending)

    if arguments.update_pending:
        write_pending(pending_path, dirty)
        total = sum(dirty.values())
        print(f"pending: {total} comment(s) in {len(dirty)} file(s) written to {pending_path.as_posix()}")
        return 0

    if arguments.paths:
        if not findings:
            return 0
        for finding in findings:
            print(f"{finding.path}:{finding.line}:{finding.column}: comment outside the allowlist: {finding.text}")
        print(f"comments: {len(findings)} outside the allowlist in {len(dirty)} file(s) checked")
        return 1

    if not pending_path.exists():
        print(f"check-comments: pending list not found: {pending_path.as_posix()}", file=sys.stderr)
        print("check-comments: run bash scripts/check-comments.sh --atualizar-pendentes", file=sys.stderr)
        return 2
    try:
        listed = read_pending(pending_path)
    except ValueError as error:
        print(f"check-comments: pending: {error}", file=sys.stderr)
        return 2

    appeared = sorted(set(dirty) - set(listed))
    vanished = sorted(set(listed) - set(dirty))
    drifted = [relative for relative in sorted(set(dirty) & set(listed)) if dirty[relative] != listed[relative]]
    if not appeared and not vanished and not drifted:
        print(f"comments: {len(dirty)} file(s) pending as listed, no new comment")
        return 0

    for relative in appeared:
        print(f"+ {relative}: {dirty[relative]} comment(s) outside the pending list")
    for relative in vanished:
        print(f"- {relative}: it is in the pending list and has no comment left in the tree")
    for relative in drifted:
        print(f"! {relative}: the list says {listed[relative]} comment(s) and the tree has {dirty[relative]}")
    print(
        f"comments: gate failed ({len(appeared)} new file(s), "
        f"{len(vanished)} stale entr(ies), {len(drifted)} count(s) differing)"
    )
    print("comments: run bash scripts/check-comments.sh --atualizar-pendentes after cleaning the module")
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
