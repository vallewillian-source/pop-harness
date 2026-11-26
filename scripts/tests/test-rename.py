#!/usr/bin/env python3
"""Checks of `scripts/rename.sh` that need no language tool.

The live half (rust-analyzer and the TypeScript language service actually renaming a symbol) is
the rehearsed run reported with the task: it is too expensive for the CI, which needs seconds, so
this file stubs the language tool and proves the parts this repository owns: map parsing (with and
without the anchor), the refusals of the contract, the refusal of a name declared in more than one
module, the anchored rename that must leave a same-named symbol in another crate alone, the order
of the candidates, the code scope shared with the two gates, the edit applier in both protocol
units, the leftover decision and that `--dry-run` writes nothing.

Run: `python3 scripts/tests/test-rename.py`.
"""

import importlib.util
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

TOOL = Path(__file__).resolve().parent.parent / "rename.py"

failures = []


def check(name, condition, detail=""):
    if condition:
        print(f"  ok: {name}")
        return
    failures.append(name if not detail else f"{name}: {detail}")
    print(f"  FALHOU: {name} {detail}")


def load_tool():
    spec = importlib.util.spec_from_file_location("rename_tool", TOOL)
    module = importlib.util.module_from_spec(spec)
    sys.modules["rename_tool"] = module
    spec.loader.exec_module(module)
    return module


def run(args, cwd):
    return subprocess.run([sys.executable, str(TOOL), *args], cwd=str(cwd), capture_output=True, text=True)


def write(root, name, text):
    path = root / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


class StubTool:
    """Stands in for the language tool: renames `old` in the files it is told to touch."""

    def __init__(self, root, old, new, files):
        self.root = root
        self.old = old
        self.new = new
        self.files = set(files)
        self.calls = []

    def can_rename(self, hit):
        return hit["relative"] in self.files

    def rename(self, hit, new_name):
        self.calls.append(hit["relative"])
        changes = []
        for relative in self.files:
            path = self.root / relative
            source = path.read_text(encoding="utf-8")
            position = source.find(self.old)
            if position < 0:
                continue
            prefix = source[:position]
            line = prefix.count("\n")
            character = len(prefix.split("\n")[-1].encode("utf-8"))
            changes.append(
                {
                    "file": str(path),
                    "range": {
                        "start": {"line": line, "character": character},
                        "end": {"line": line, "character": character + len(self.old)},
                    },
                    "newText": new_name,
                }
            )
        return changes

    def close_documents(self):
        pass


class StubTools:
    def __init__(self, rust):
        self.rust = rust
        self.rust_error = "stub has no rust-analyzer"
        self.typescript_error = "stub has no TypeScript"

    def get_typescript(self):
        return None


def test_map_parsing(tool, root):
    print("mapa:")
    path = write(
        root,
        "map.txt",
        "# comentário\n"
        "VarreduraBanco -> DatabaseScan   # fim de linha\n"
        "\n"
        "crates/a/src/armazem.rs -> crates/a/src/warehouse.rs\n"
    )
    pairs, errors = tool.read_map(path)
    check("lê duas linhas e ignora comentário", not errors and len(pairs) == 2, str(errors))
    check("identificador reconhecido", pairs[0]["kind"] == "identifier", str(pairs[0]))
    check("caminho reconhecido", pairs[1]["kind"] == "path", str(pairs[1]))
    check("tradução lida", pairs[0]["new"] == "DatabaseScan", pairs[0]["new"])
    broken = write(root, "broken.txt", "VarreduraBanco DatabaseScan\n")
    pairs, errors = tool.read_map(broken)
    check("linha sem seta reprova", not pairs and len(errors) == 1, str(errors))


def test_refusals(tool, root):
    print("recusas:")
    published = tool.published_names()
    dictionary = tool.portuguese_terms()
    cases = [
        ("nome publicado (tabela)", {"old": "fichas", "new": "cases", "line": 1}),
        ("palavra da linguagem de fluxos", {"old": "mensagem_ao_cliente", "new": "message_to_client", "line": 2}),
        ("identificador inválido", {"old": "VarreduraBanco", "new": "2Bad", "line": 3}),
        ("termo português no destino", {"old": "VarreduraBanco", "new": "banco", "line": 4}),
        ("igual ao original", {"old": "VarreduraBanco", "new": "VarreduraBanco", "line": 5}),
    ]
    for label, pair in cases:
        try:
            tool.check_identifier_pair(pair, published, dictionary)
            check(f"recusa {label}", False, "aceitou")
        except tool.Refused as error:
            check(f"recusa {label}", bool(str(error)), str(error))
    try:
        tool.check_identifier_pair({"old": "VarreduraBanco", "new": "DatabaseScan", "line": 6}, published, dictionary)
        check("aceita nome em inglês", True)
    except tool.Refused as error:
        check("aceita nome em inglês", False, str(error))


def test_apply_changes(tool, root):
    print("aplicação de edição:")
    path = write(root, "utf8.txt", "um VarreduraBanco dois VarreduraBanco\n")
    source = path.read_text(encoding="utf-8")
    positions = []
    cursor = 0
    while True:
        position = source.find("VarreduraBanco", cursor)
        if position < 0:
            break
        positions.append(position)
        cursor = position + 1
    changes = [{"file": str(path), "start": position, "length": len("VarreduraBanco"), "newText": "DatabaseScan"} for position in positions]
    tool.apply_changes(changes, "utf8")
    check("utf-8 aplica todas as edições", path.read_text(encoding="utf-8").count("DatabaseScan") == 2)

    astral = write(root, "utf16.txt", "a😀VarreduraBanco fim\n")
    text = astral.read_text(encoding="utf-8")
    offset = len("a😀".encode("utf-16-le")) // 2
    tool.apply_changes(
        [{"file": str(astral), "start": offset, "length": len("VarreduraBanco"), "newText": "DatabaseScan"}], "utf16"
    )
    check("utf-16 respeita caractere fora do plano", astral.read_text(encoding="utf-8") == "a😀DatabaseScan fim\n", astral.read_text(encoding="utf-8"))

    untouched = write(root, "untouched.txt", "VarreduraBanco\n")
    tool.apply_changes([{"file": str(untouched), "start": 0, "length": 13, "newText": "DatabaseScan"}], "utf8", dry_run=True)
    check("dry-run não grava", untouched.read_text(encoding="utf-8") == "VarreduraBanco\n")


def test_leftover_decision(tool):
    print("sobra de nome antigo:")
    found = [{"relative": "a.rs", "line": 3}, {"relative": "docs/b.star", "line": 7}]
    leftover, accepted = tool.leftover_findings(found, [])
    check("sem exceção, tudo bloqueia", len(leftover) == 2 and not accepted)
    leftover, accepted = tool.leftover_findings(found, ["docs/"])
    check("exceção de caminho desonera só ele", len(leftover) == 1 and len(accepted) == 1, str(leftover))


def test_stub_run(tool, root):
    print("execução com ferramenta simulada:")
    work = root / "stub"
    write(work, "a.rs", "pub struct VarreduraBanco;\n")
    write(work, "b.rs", "use crate::a::VarreduraBanco;\n")
    pair = {"kind": "identifier", "old": "VarreduraBanco", "new": "DatabaseScan", "line": 1}

    both = StubTool(work, "VarreduraBanco", "DatabaseScan", ["a.rs", "b.rs"])
    tally = tool.rename_identifier(work, pair, True, StubTools(both), [])
    check("dry-run relata as duas edições", sum(tally.values()) == 2, str(tally))
    check(
        "dry-run não toca no arquivo",
        (work / "a.rs").read_text(encoding="utf-8") == "pub struct VarreduraBanco;\n",
        (work / "a.rs").read_text(encoding="utf-8"),
    )

    both = StubTool(work, "VarreduraBanco", "DatabaseScan", ["a.rs", "b.rs"])
    tally = tool.rename_identifier(work, pair, False, StubTools(both), [])
    check("aplicação renomeia os dois arquivos", sum(tally.values()) == 2, str(tally))
    check("a.rs renomeado", "DatabaseScan" in (work / "a.rs").read_text(encoding="utf-8"))
    check("b.rs renomeado", "DatabaseScan" in (work / "b.rs").read_text(encoding="utf-8"))

    write(work, "a.rs", "pub struct VarreduraBanco;\n")
    write(work, "b.rs", "use crate::a::VarreduraBanco;\n")
    partial = StubTool(work, "VarreduraBanco", "DatabaseScan", ["a.rs"])
    try:
        tool.rename_identifier(work, pair, False, StubTools(partial), [])
        check("recusa quando sobra ocorrência", False, "aceitou")
    except tool.Leftover as error:
        check("recusa quando sobra ocorrência", "left outside strings and comments" in str(error), str(error))

    write(work, "a.rs", "pub struct VarreduraBanco;\n")
    write(work, "b.rs", "use crate::a::VarreduraBanco;\n")
    partial = StubTool(work, "VarreduraBanco", "DatabaseScan", ["a.rs"])
    tally = tool.rename_identifier(work, pair, False, StubTools(partial), ["b.rs"])
    check("exceção de caminho aceita a sobra", sum(tally.values()) == 1, str(tally))

    write(work, "c.star", "VarreduraBanco = 1\n")
    write(work, "a.rs", "pub struct OutraCoisa;\n")
    write(work, "b.rs", "use crate::a::OutraCoisa;\n")
    only_data = {"kind": "identifier", "old": "VarreduraBanco", "new": "DatabaseScan", "line": 1}
    try:
        tool.rename_identifier(work, only_data, False, StubTools(None), [])
        check("recusa nome que nenhuma ferramenta vê", False, "aceitou")
    except tool.Refused as error:
        check("recusa nome que nenhuma ferramenta vê", "no language tool owns" in str(error), str(error))


class UnitStubTool:
    """Stands in for a language tool that renames one symbol: it moves every hit of its unit.

    `StubTool` renames a fixed list of files; this one reads the position it is asked about and
    renames the occurrences of that directory only, which is what rust-analyzer does for a symbol
    declared in one crate. It is what lets the test tell "renamed the anchored symbol" apart from
    "renamed something else that happened to have the same name".
    """

    def __init__(self, root, old):
        self.root = root
        self.old = old
        self.calls = []

    def can_rename(self, hit):
        return True

    def rename(self, hit, new_name):
        self.calls.append(hit["relative"])
        changes = []
        directory = (self.root / hit["relative"]).parent
        for path in sorted(directory.rglob("*.rs")):
            source = path.read_text(encoding="utf-8")
            cursor = 0
            while True:
                position = source.find(self.old, cursor)
                if position < 0:
                    break
                cursor = position + len(self.old)
                prefix = source[:position]
                line = prefix.count("\n")
                character = len(prefix.split("\n")[-1].encode("utf-8"))
                changes.append(
                    {
                        "file": str(path),
                        "range": {
                            "start": {"line": line, "character": character},
                            "end": {"line": line, "character": character + len(self.old)},
                        },
                        "newText": new_name,
                    }
                )
        return changes

    def close_documents(self):
        pass


def two_crates(root):
    """Two crates declaring the same name: the map has to say which symbol it means."""
    work = root / "two"
    write(work, "Cargo.toml", "[workspace]\n")
    write(work, "alfa/Cargo.toml", "[package]\nname = \"alfa\"\n")
    write(work, "alfa/src/lib.rs", "pub struct VarreduraBanco;\n\npub fn usar() -> VarreduraBanco {\n    VarreduraBanco\n}\n")
    write(work, "zebra/Cargo.toml", "[package]\nname = \"zebra\"\n")
    write(work, "zebra/src/lib.rs", "pub struct VarreduraBanco;\n")
    write(work, "zebra/src/uso.rs", "use crate::VarreduraBanco;\n\npub fn chamar(valor: VarreduraBanco) -> VarreduraBanco {\n    valor\n}\n")
    return work


def test_anchor_parsing(tool, root):
    print("âncora do mapa:")
    path = write(
        root,
        "map-anchor.txt",
        "VarreduraBanco -> DatabaseScan\n"
        "VarreduraBanco @ crates/zebra/src/lib.rs -> DatabaseScan\n"
        "VarreduraBanco @ crates/zebra:12 -> DatabaseScan\n"
        "crates/a/src/armazem.rs -> crates/a/src/warehouse.rs\n",
    )
    pairs, errors = tool.read_map(path)
    check("quatro linhas lidas", not errors and len(pairs) == 4, str(errors))
    check("sem âncora o escopo é nulo", pairs[0]["scope"] is None, str(pairs[0]))
    check("âncora de arquivo", pairs[1]["scope"] == {"path": "crates/zebra/src/lib.rs", "line": None}, str(pairs[1]))
    check("âncora de diretório com linha", pairs[2]["scope"] == {"path": "crates/zebra", "line": 12}, str(pairs[2]))
    check("linha de caminho continua caminho", pairs[3]["kind"] == "path", str(pairs[3]))
    check("nome sem âncora fica limpo", pairs[1]["old"] == "VarreduraBanco", pairs[1]["old"])
    broken = write(root, "map-anchor-broken.txt", "@ crates/zebra/src/lib.rs -> DatabaseScan\n")
    _, errors = tool.read_map(broken)
    check("âncora sem nome reprova", len(errors) == 1, str(errors))


def test_ambiguous_name(tool, root):
    print("nome declarado em dois módulos:")
    work = two_crates(root)
    pair = {"kind": "identifier", "old": "VarreduraBanco", "new": "DatabaseScan", "line": 1, "scope": None}
    tool_ = UnitStubTool(work, "VarreduraBanco")
    try:
        tool.rename_identifier(work, pair, False, StubTools(tool_), [])
        check("sem âncora, recusa nome declarado em dois módulos", False, "renomeou")
    except tool.Refused as error:
        check("sem âncora, recusa nome declarado em dois módulos", "declared in 2 modules" in str(error), str(error))
        check("a recusa ensina a âncora", "@ <path>" in str(error), str(error))
    check("a recusa não tocou em nenhum arquivo", not tool_.calls, str(tool_.calls))
    check(
        "alfa continua em português",
        "VarreduraBanco" in (work / "alfa/src/lib.rs").read_text(encoding="utf-8"),
    )


def test_anchored_rename(tool, root):
    print("renomeação ancorada:")
    work = two_crates(root)
    pair = {
        "kind": "identifier",
        "old": "VarreduraBanco",
        "new": "DatabaseScan",
        "line": 1,
        "scope": {"path": "zebra", "line": None},
    }
    tool_ = UnitStubTool(work, "VarreduraBanco")
    tally = tool.rename_identifier(work, pair, False, StubTools(tool_), [])
    check("só a unidade ancorada é renomeada", all(call.startswith("zebra/") for call in tool_.calls), str(tool_.calls))
    zebra = (work / "zebra/src/lib.rs").read_text(encoding="utf-8") + (work / "zebra/src/uso.rs").read_text(encoding="utf-8")
    check("zebra em inglês", "DatabaseScan" in zebra and "VarreduraBanco" not in zebra, zebra)
    untouched = (work / "alfa/src/lib.rs").read_text(encoding="utf-8")
    check("alfa intocado", "VarreduraBanco" in untouched and "DatabaseScan" not in untouched, untouched)
    check("edição contada só no escopo", sum(tally.values()) == 4, str(tally))

    wrong = dict(pair)
    wrong["scope"] = {"path": "nao-existe", "line": None}
    try:
        tool.rename_identifier(work, wrong, False, StubTools(UnitStubTool(work, "DatabaseScan")), [])
        check("âncora sem ocorrência reprova", False, "aceitou")
    except tool.Refused as error:
        check("âncora sem ocorrência reprova", "holds no occurrence" in str(error), str(error))


def test_declaration_ranking(tool, root):
    print("ordem dos candidatos:")
    work = root / "ranking"
    write(work, "a.rs", "pub fn usar() -> VarreduraBanco {\n    VarreduraBanco\n}\n")
    write(work, "b.rs", "pub struct VarreduraBanco;\n")
    found = tool.occurrences(work, "VarreduraBanco")
    ordered = tool.ordered_candidates(work, found, "rust", "VarreduraBanco")
    check("a declaração vem primeiro, não o arquivo de nome igual", ordered[0]["relative"] == "b.rs", str(ordered[0]))
    scoped = tool.ordered_candidates(work, found, "rust", "VarreduraBanco", {"path": "a.rs", "line": None})
    check(
        "a âncora corta os candidatos",
        {hit["relative"] for hit in scoped} == {"a.rs"} and len(scoped) == 2,
        str(scoped),
    )


def test_code_scope(tool, root):
    print("escopo das travas:")
    work = root / "scope"
    write(work, "design_assets/captura/scripts/seed.py", "valor = 1\n")
    write(work, "design_assets/captura/scripts/capture.sh", "echo 1\n")
    write(work, "design_assets/captura/01-tela.html", "<html></html>\n")
    write(work, "design_assets/captura/tela.png", "\x89PNG\n")
    write(work, "design_assets/captura/capture.mjs", "const a = 1;\n")
    write(work, "crates/a/src/lib.rs", "pub struct A;\n")
    seen = sorted(relative for relative, _ in tool.scanned_files(work))
    check("o script de design_assets é lido", "design_assets/captura/scripts/seed.py" in seen, str(seen))
    check("o shell de design_assets é lido", "design_assets/captura/scripts/capture.sh" in seen, str(seen))
    check("a captura html não é lida", "design_assets/captura/01-tela.html" not in seen, str(seen))
    check("a imagem não é lida", "design_assets/captura/tela.png" not in seen, str(seen))
    check("o script fora de scripts/ não é lido", "design_assets/captura/capture.mjs" not in seen, str(seen))
    check("o código do produto continua lido", "crates/a/src/lib.rs" in seen, str(seen))
    check("a regra é a do medidor", tool.glossary.in_code_scope("design_assets/x/scripts/a.py"))
    check("a regra exclui a imagem", not tool.glossary.in_code_scope("design_assets/x/a.png"))
    write(work, "design_assets/captura/scripts/capture.mjs", "const VarreduraBanco = 1;\n")
    written = tool.occurrences(work, "VarreduraBanco")
    check("o script de captura é visto", any("capture.mjs" in hit["relative"] for hit in written), str(written))
    units = tool.declared_units(work, written, {"old": "VarreduraBanco"})
    check("declaração fora de Rust e TypeScript não é unidade", not units, str(units))


def test_path_refusals(tool, root):
    print("recusa de renomeação de arquivo:")
    work = root / "paths"
    write(work, "a.rs", "pub struct A;\n")
    write(work, "dir/b.rs", "pub struct B;\n")
    cases = [
        ("arquivo inexistente", "nao_existe.rs", "zz.rs"),
        ("outro diretório", "a.rs", "dir/c.rs"),
        ("extensão diferente", "a.rs", "a.txt"),
    ]
    for label, old, new in cases:
        try:
            tool.rename_path(work, {"old": old, "new": new, "line": 1}, True, StubTools(None))
            check(f"recusa {label}", False, "aceitou")
        except tool.Refused as error:
            check(f"recusa {label}", bool(str(error)), str(error))
    try:
        tool.rename_path(work, {"old": "a.rs", "new": "zz.rs", "line": 1}, True, StubTools(None))
        check("sem ferramenta, a renomeação de arquivo para com erro de ferramenta", False, "aceitou")
    except tool.ToolMissing:
        check("sem ferramenta, a renomeação de arquivo para com erro de ferramenta", True)


def test_cli(tool, root):
    print("linha de comando:")
    result = run(["--map", str(root / "nao-existe.txt")], root)
    check("mapa ausente sai 2", result.returncode == 2, result.stderr.strip())
    empty = write(root, "empty.txt", "# só comentário\n")
    result = run(["--map", str(empty)], root)
    check("mapa vazio sai 2", result.returncode == 2, result.stderr.strip())
    published = write(root, "published.txt", "fichas -> cases\n")
    result = run(["--map", str(published)], root)
    check("nome publicado sai 3 antes de tocar em qualquer coisa", result.returncode == 3, result.stderr.strip())
    check("mensagem explica o motivo", "published name" in result.stderr, result.stderr.strip())
    result = run(["--map", str(root / "nao-existe.txt"), "--help"], root)
    check("--help sai 0", result.returncode == 0)


def main():
    if not TOOL.exists():
        print(f"ferramenta não encontrada: {TOOL}")
        return 1
    tool = load_tool()
    root = Path(tempfile.mkdtemp(prefix="rename-test-"))
    try:
        test_map_parsing(tool, root)
        test_refusals(tool, root)
        test_apply_changes(tool, root)
        test_leftover_decision(tool)
        test_stub_run(tool, root)
        test_anchor_parsing(tool, root)
        test_ambiguous_name(tool, root)
        test_anchored_rename(tool, root)
        test_declaration_ranking(tool, root)
        test_code_scope(tool, root)
        test_path_refusals(tool, root)
        test_cli(tool, root)
    finally:
        shutil.rmtree(root, ignore_errors=True)
    if failures:
        print(f"\nteste do renomeador: FALHOU ({len(failures)} caso(s))")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("\nteste do renomeador: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
