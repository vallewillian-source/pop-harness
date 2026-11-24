#!/usr/bin/env python3
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

TOOL = Path(__file__).resolve().parent.parent / "strip-comments.py"

RUST_INPUT = """\
//! Documentação de módulo: a URL http://exemplo.test/a//b não é comentário.
use std::fmt;
/* bloco /* aninhado */ ainda dentro do bloco */
fn main() {
    let url = "http://exemplo.test/a//b";
    let bruto = r#"texto // não é comentário /* nem isto */"#;
    let bruto2 = r##"com "# interno e // dentro"##;
    let aspa = '/';
    let escapada = '\\'';
    let byte = b'\\n';
    let aspas_duplas = '"';
    let referencia: &'static str = "// continua string";
    println!("{url} {bruto} {bruto2} {aspa} {escapada} {byte} {aspas_duplas} {referencia}");
}
// comentário de verdade no fim
"""

RUST_EXPECTED = """\
use std::fmt;
fn main() {
    let url = "http://exemplo.test/a//b";
    let bruto = r#"texto // não é comentário /* nem isto */"#;
    let bruto2 = r##"com "# interno e // dentro"##;
    let aspa = '/';
    let escapada = '\\'';
    let byte = b'\\n';
    let aspas_duplas = '"';
    let referencia: &'static str = "// continua string";
    println!("{url} {bruto} {bruto2} {aspa} {escapada} {byte} {aspas_duplas} {referencia}");
}
"""

JS_INPUT = """\
const url = "http://a.test//b";
const modelo = `linha ${valor ? "a // b" : "c"} fim`;
const aninhado = `${`${a}`} // dentro`;
const regex = /https?:\\/\\/a\\.test\\/x/;
// sai esta linha
/* sai este bloco */
export const total = 1; // sai o comentário de fim de linha
const marcador = "# versao";
"""

JS_EXPECTED = """\
const url = "http://a.test//b";
const modelo = `linha ${valor ? "a // b" : "c"} fim`;
const aninhado = `${`${a}`} // dentro`;
const regex = /https?:\\/\\/a\\.test\\/x/;
export const total = 1;
const marcador = "# versao";
"""

CSS_INPUT = """\
:root {
    --cor: #121a24;
}
/* sai este comentário */
.caixa {
    content: "/* não sai */";
    background: url("http://a.test//b.png");
    margin: 0; /* sai este também */
}
"""

CSS_EXPECTED = """\
:root {
    --cor: #121a24;
}
.caixa {
    content: "/* não sai */";
    background: url("http://a.test//b.png");
    margin: 0;
}
"""

SQL_INPUT = """\
-- sai esta linha
CREATE TABLE exemplo (
    texto TEXT DEFAULT 'a -- b' NOT NULL,
    outro TEXT DEFAULT 'linha com -- dentro'
);
/* sai este bloco */
INSERT INTO exemplo (texto) VALUES ('x -- y');
"""

SQL_EXPECTED = """\
CREATE TABLE exemplo (
    texto TEXT DEFAULT 'a -- b' NOT NULL,
    outro TEXT DEFAULT 'linha com -- dentro'
);
INSERT INTO exemplo (texto) VALUES ('x -- y');
"""

SHELL_INPUT = """\
#!/usr/bin/env bash
# sai este comentário
set -uo pipefail

lista=(a b c)
total=${#lista[@]}
sem_prefixo=${1#*=}

uso() {
    sed -n '1,3p' "$0" | sed 's/^# \\{0,1\\}//'
}

echo "total ${total} ${sem_prefixo}"  # sai o comentário de fim de linha

cat <<'TEXTO'
# esta linha é dado do heredoc, não comentário
chave = "valor"  # também é dado
TEXTO

echo feito;
"""

SHELL_EXPECTED = """\
#!/usr/bin/env bash
set -uo pipefail

lista=(a b c)
total=${#lista[@]}
sem_prefixo=${1#*=}

uso() {
    sed -n '1,3p' "$0" | sed 's/^# \\{0,1\\}//'
}

echo "total ${total} ${sem_prefixo}"

cat <<'TEXTO'
# esta linha é dado do heredoc, não comentário
chave = "valor"  # também é dado
TEXTO

echo feito;
"""

PYTHON_INPUT = """\
#!/usr/bin/env python3
\"\"\"Resumo com # dentro da docstring e http://a.test//b.
\"\"\"
import os

# sai este comentário
valor = "# não sai"
consulta = f"{os.curdir!r} # não sai também"
texto = r"# nem este"

print(valor, consulta, texto)  # sai este de fim
"""

PYTHON_EXPECTED = """\
#!/usr/bin/env python3
\"\"\"Resumo com # dentro da docstring e http://a.test//b.
\"\"\"
import os

valor = "# não sai"
consulta = f"{os.curdir!r} # não sai também"
texto = r"# nem este"

print(valor, consulta, texto)
"""

TOML_INPUT = """\
# sai este comentário
[servidor]
endereco = "0.0.0.0"   # sai este de fim
nome = "valor # não sai"
literal = 'outro # também não'

[lista]
itens = [
    "a", # sai
    "b",
]
"""

TOML_EXPECTED = """\
[servidor]
endereco = "0.0.0.0"
nome = "valor # não sai"
literal = 'outro # também não'

[lista]
itens = [
    "a",
    "b",
]
"""

YAML_INPUT = """\
# sai este comentário
nome: valor#com_cerquilha_sem_espaco
lista:
  - Yes, I'll open the pull request
  - No, I'm only suggesting
texto: "com # dentro da aspa"
outro: 'com # dentro da aspa simples'
chave: valor  # sai este de fim
"""

YAML_EXPECTED = """\
nome: valor#com_cerquilha_sem_espaco
lista:
  - Yes, I'll open the pull request
  - No, I'm only suggesting
texto: "com # dentro da aspa"
outro: 'com # dentro da aspa simples'
chave: valor
"""

STAR_INPUT = """\
# sai este comentário
RESUMO = \"\"\"
texto com # dentro da string tripla
\"\"\"
def etapa(campo):
    # sai este também
    return campo == "# não sai"
"""

STAR_EXPECTED = """\
RESUMO = \"\"\"
texto com # dentro da string tripla
\"\"\"
def etapa(campo):
    return campo == "# não sai"
"""

HTML_INPUT = """\
<!doctype html>
<!-- sai este comentário -->
<html lang="pt-BR">
    <body><div id="root"></div></body>
</html>
"""

HTML_EXPECTED = """\
<!doctype html>
<html lang="pt-BR">
    <body><div id="root"></div></body>
</html>
"""

DOCKERFILE_INPUT = """\
# sai este comentário
FROM ubuntu:24.04
RUN apt-get update \\
    && apt-get install -y --no-install-recommends ca-certificates
"""

DOCKERFILE_EXPECTED = """\
FROM ubuntu:24.04
RUN apt-get update \\
    && apt-get install -y --no-install-recommends ca-certificates
"""

FIXTURES = [
    ("exemplo.rs", RUST_INPUT, RUST_EXPECTED),
    ("exemplo.ts", JS_INPUT, JS_EXPECTED),
    ("exemplo.tsx", JS_INPUT, JS_EXPECTED),
    ("exemplo.css", CSS_INPUT, CSS_EXPECTED),
    ("exemplo.sql", SQL_INPUT, SQL_EXPECTED),
    ("exemplo.sh", SHELL_INPUT, SHELL_EXPECTED),
    ("exemplo.py", PYTHON_INPUT, PYTHON_EXPECTED),
    ("exemplo.toml", TOML_INPUT, TOML_EXPECTED),
    ("exemplo.yaml", YAML_INPUT, YAML_EXPECTED),
    ("exemplo.yml", YAML_INPUT, YAML_EXPECTED),
    ("exemplo.star", STAR_INPUT, STAR_EXPECTED),
    ("exemplo.html", HTML_INPUT, HTML_EXPECTED),
    ("Dockerfile", DOCKERFILE_INPUT, DOCKERFILE_EXPECTED),
]

ALLOW_LIST = """\
*.sh\tshebang\t\tprimeira linha do script
*.py\tshebang\t\tprimeira linha do script
*\tdirective\tnoqa\tdiretiva do linter
"""

failures = []


def check(name, condition, detail=""):
    if condition:
        print(f"  ok: {name}")
        return
    failures.append(name if not detail else f"{name}: {detail}")
    print(f"  FALHOU: {name} {detail}")


def run(args, cwd):
    return subprocess.run(
        [sys.executable, str(TOOL), *args],
        cwd=str(cwd),
        capture_output=True,
        text=True,
    )


def write(base, name, text):
    path = base / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


def test_removal(base, allow_list):
    print("remoção por linguagem")
    for name, source, expected in FIXTURES:
        path = write(base, name, source)
        result = run(["--allow-list", str(allow_list), name], base)
        check(f"{name} sai com 0", result.returncode == 0, result.stderr.strip())
        actual = path.read_text(encoding="utf-8")
        check(f"{name} preserva só o que não é comentário", actual == expected, f"obtido={actual!r}")


def test_idempotent(base, allow_list):
    print("idempotência")
    path = write(base, "repetido.rs", RUST_INPUT)
    run(["--allow-list", str(allow_list), "repetido.rs"], base)
    first = path.read_text(encoding="utf-8")
    result = run(["--allow-list", str(allow_list), "repetido.rs"], base)
    check("segunda passada não muda nada", path.read_text(encoding="utf-8") == first)
    check("segunda passada informa 0 arquivo alterado", "0 arquivo(s) alterado(s)" in result.stdout, result.stdout)


def test_check_mode(base, allow_list):
    print("modo --check")
    write(base, "sujo.rs", RUST_INPUT)
    write(base, "limpo.rs", RUST_EXPECTED)
    write(base, "shebang.sh", "#!/usr/bin/env bash\necho ok\n")
    write(base, "diretiva.py", "import os\nvalor = 1  # noqa: E501\n")
    write(base, "sem_diretiva.py", "valor = 1  # explicação qualquer\n")
    dirty = run(["--allow-list", str(allow_list), "--check", "sujo.rs"], base)
    check("arquivo com comentário sai diferente de zero", dirty.returncode == 1, str(dirty.returncode))
    check("lista o comentário com caminho, linha e coluna", "sujo.rs:1:1:" in dirty.stdout, dirty.stdout)
    clean = run(["--allow-list", str(allow_list), "--check", "limpo.rs"], base)
    check("arquivo limpo sai com 0", clean.returncode == 0, clean.stdout)
    shebang = run(["--allow-list", str(allow_list), "--check", "shebang.sh"], base)
    check("shebang é aceito pela allowlist", shebang.returncode == 0, shebang.stdout)
    directive = run(["--allow-list", str(allow_list), "--check", "diretiva.py"], base)
    check("diretiva noqa é aceita", directive.returncode == 0, directive.stdout)
    plain = run(["--allow-list", str(allow_list), "--check", "sem_diretiva.py"], base)
    check("comentário comum não passa por diretiva", plain.returncode == 1, plain.stdout)
    without = run(["--no-allow-list", "--check", "shebang.sh"], base)
    check("sem allowlist o shebang é reprovado", without.returncode == 1, without.stdout)


def test_dry_run(base, allow_list):
    print("modo --dry-run")
    path = write(base, "seco.sql", SQL_INPUT)
    result = run(["--allow-list", str(allow_list), "--dry-run", "--diff", "seco.sql"], base)
    check("dry-run sai com 0", result.returncode == 0, result.stderr.strip())
    check("dry-run não grava o arquivo", path.read_text(encoding="utf-8") == SQL_INPUT)
    check("dry-run conta o que sairia", "seco.sql: 2 comentário(s)" in result.stdout, result.stdout)
    check("dry-run mostra o diff", "--- a/seco.sql" in result.stdout, result.stdout)


def test_skipped_paths(base, allow_list):
    print("arquivos gerados")
    schema = write(base, "console/src/api/schema.d.ts", "// gerado\nexport type A = 1;\n")
    lock = write(base, "Cargo.lock", "# gerado\n[[package]]\nname = \"x\"\n")
    result = run(["--allow-list", str(allow_list), "."], base)
    check("execução no diretório sai com 0", result.returncode == 0, result.stderr.strip())
    check("schema.d.ts fica intacto", schema.read_text(encoding="utf-8") == "// gerado\nexport type A = 1;\n")
    check("Cargo.lock fica intacto", lock.read_text(encoding="utf-8") == "# gerado\n[[package]]\nname = \"x\"\n")
    direct = run(["--allow-list", str(allow_list), "console/src/api/schema.d.ts"], base)
    check("caminho gerado passado direto é ignorado", direct.returncode == 0 and schema.read_text(encoding="utf-8") == "// gerado\nexport type A = 1;\n", direct.stderr)


def test_allow_list_errors(base):
    print("allowlist inválida")
    path = write(base, "quebrada.txt", "scripts/*.sh shebang\n")
    result = run(["--allow-list", str(path), "--check", "exemplo.rs"], base)
    check("allowlist com linha inválida sai com 2", result.returncode == 2, result.stderr.strip())


def test_data_lines(base):
    print("linha de dado que parece comentário")
    source = "#!/usr/bin/env bash\n# uso: script.sh\nsed -n '2,3p' \"$0\"\n"
    write(base, "dados/uso.sh", source)
    allow = write(base, "dados/uso-allow.txt", "*.sh\tshebang\t\tprimeira linha\n*.sh\tdata\tline:2\tbloco de uso lido por sed\n")
    result = run(["--allow-list", str(allow), "--check", "uso.sh"], base / "dados")
    check("linha de dado aceita pela allowlist", result.returncode == 0, result.stdout)
    removal = run(["--allow-list", str(allow), "uso.sh"], base / "dados")
    check("linha de dado não é removida", (base / "dados" / "uso.sh").read_text(encoding="utf-8") == source, removal.stdout)


def test_language_of_directory_without_code(base, allow_list):
    print("diretório sem código")
    write(base, "sem-codigo/nota.md", "# título\n")
    write(base, "sem-codigo/dados.json", "{\n  \"a\": 1\n}\n")
    result = run(["--allow-list", str(allow_list), "--check", "."], base / "sem-codigo")
    check("markdown e json ficam fora da varredura", result.returncode == 0, result.stdout)


def main():
    if not TOOL.exists():
        print(f"ferramenta não encontrada: {TOOL}")
        return 1
    root = Path(tempfile.mkdtemp(prefix="strip-comments-"))
    try:
        allow_list = root / "allow-comments.txt"
        allow_list.write_text(ALLOW_LIST, encoding="utf-8")
        test_removal(root, allow_list)
        test_idempotent(root, allow_list)
        test_check_mode(root, allow_list)
        test_dry_run(root, allow_list)
        test_skipped_paths(root, allow_list)
        test_allow_list_errors(root)
        test_data_lines(root)
        test_language_of_directory_without_code(root, allow_list)
    finally:
        shutil.rmtree(root, ignore_errors=True)
    if failures:
        print(f"\nteste do removedor: FALHOU ({len(failures)} caso(s))")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("\nteste do removedor: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
