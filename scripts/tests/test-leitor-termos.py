#!/usr/bin/env python3
import importlib.util
import shutil
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
GLOSSARY = ROOT / "scripts" / "glossary-terms.py"

spec = importlib.util.spec_from_file_location("glossary_terms", GLOSSARY)
glossary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(glossary)
reader = glossary.term_reader

failures = []


def check(name, condition, detail=""):
    if condition:
        print(f"  ok: {name}")
        return
    failures.append(name if not detail else f"{name}: {detail}")
    print(f"  FALHOU: {name} {detail}")


def tokens(directory, name, source):
    path = directory / name
    path.write_text(source, encoding="utf-8")
    language = "css" if name.endswith(".css") else "javascript"
    return reader.tokens(path, language)


JSX_TEXT = """\
export function Rotulo({ valor }: { valor: string }) {
    return <span className="x">Organizações e Filtrar<b>{valor}</b></span>;
}
"""

JSX_NESTED_TEXT = """\
const a = <Painel titulo="Nova">texto de produto com apóstrofo: não é código<Item aoFechar={fechar} /></Painel>;
"""

JSX_FRAGMENT = """\
const f = <>Loja aberta<Cartao rotulo={rotulo} /></>;
"""

GENERIC_ARROW = """\
const cargaInicial = <T,>(): Carga<T> => ({ carregando: true, dados: null });
const outro = () => { const valor = 1; return valor; };
"""

COMPARISON = """\
const menor = a < b;
const lista: Array<string> = [];
"""

TEMPLATE = """\
const texto = `oi ${nomeDoUsuario} ${ valor + 1 }`;
"""

QUOTED_KEY = """\
const cabecalho = { "rotulo": 1, "corpo": 2 };
"""

OBJECT_KEY = """\
const objeto = { rotulo: 1, corpo: 2 };
"""

CODE = """\
function calcular() { const valorTotal = 3; return valorTotal; }
"""

CSS_AT_RULES = """\
@media (max-width: 720px) { .poph-x { color: red; } }
@supports (display: grid) { .poph-y { display: grid; } }
@keyframes girar { from { transform: none; } }
@font-face { font-family: Fonte; }
.poph-z { margin: 0; }
"""


def main():
    print("leitor de termos: texto de JSX, at-rule de CSS, chave e controles")
    directory = Path(tempfile.mkdtemp(prefix="leitor-termos-"))
    try:
        jsx = tokens(directory, "jsx.tsx", JSX_TEXT)
        check("texto de JSX não vira identificador", "Organizações" not in jsx and "Filtrar" not in jsx, str(jsx))
        check("identificador dentro de {expressão} de JSX continua lido", "valor" in jsx, str(jsx))

        nested = tokens(directory, "aninhado.tsx", JSX_NESTED_TEXT)
        check("texto de JSX depois de atributo com texto é pulado", "apóstrofo" not in nested and "código" not in nested, str(nested))
        check("atributo JSX com expressão é lido", "fechar" in nested, str(nested))

        fragment = tokens(directory, "fragmento.tsx", JSX_FRAGMENT)
        check("texto dentro de fragmento <> é pulado", "Loja" not in fragment and "aberta" not in fragment, str(fragment))
        check("prop depois de texto de fragmento é lida", "rotulo" in fragment, str(fragment))

        arrow = tokens(directory, "generico.tsx", GENERIC_ARROW)
        check("<T,> de função genérica não é tratado como JSX", "valor" in arrow and "carregando" in arrow, str(arrow))

        comparison = tokens(directory, "comparacao.ts", COMPARISON)
        check("comparação < não é JSX", "menor" in comparison and "lista" in comparison, str(comparison))

        template = tokens(directory, "modelo.ts", TEMPLATE)
        check("identificador dentro de ${} de template é lido", "nomeDoUsuario" in template and "valor" in template, str(template))

        quoted = tokens(directory, "chave.ts", QUOTED_KEY)
        check("chave entre aspas de objeto continua sendo lida (controle)", "rotulo" in quoted and "corpo" in quoted, str(quoted))

        keyed = tokens(directory, "objeto.ts", OBJECT_KEY)
        check("chave de objeto sem aspas continua sendo lida (controle)", "rotulo" in keyed and "corpo" in keyed, str(keyed))

        code = tokens(directory, "codigo.ts", CODE)
        check("identificador português em código continua sendo lido (controle)", "valorTotal" in code, str(code))

        css = tokens(directory, "estilo.css", CSS_AT_RULES)
        check("palavra de at-rule de CSS não vira termo", not {"media", "supports", "keyframes", "face"} & set(css), str(css))
        check("seletor de classe em CSS continua sendo lido", "poph" in css and "z" in css, str(css))
    finally:
        shutil.rmtree(directory, ignore_errors=True)
    if failures:
        print(f"\nteste do leitor de termos: FALHOU ({len(failures)} caso(s))")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("\nteste do leitor de termos: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
