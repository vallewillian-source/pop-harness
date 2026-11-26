#!/usr/bin/env python3
"""Mede os termos de identificador da árvore, extrai os nomes publicados e valida o glossário.

Este script é a fonte reproduzível de `docs/GLOSSARY.md`: a tabela de tradução do documento é
gerada por ele a partir de duas entradas versionadas — os termos medidos no código e o mapa
português → inglês de `scripts/glossary-map.txt`.

Comandos:

- `--medir [--top N]`: conta os termos de identificador dos arquivos de código e configuração,
  ordenados por frequência. É a medição que sustenta a tabela.
- `--publicados`: extrai os nomes publicados (contrato) por tipo: binário, variável de ambiente,
  métrica, tabela, coluna, índice, gatilho, rota, `operationId`, campo JSON, comando e flag da CLI,
  chave do `poph.toml`, termo do protocolo de plugin e chave do formato de backup.
- `--gerar`: reescreve a região da tabela em `docs/GLOSSARY.md`.
- `--validar`: confere a tabela em disco contra a medição, o inventário publicado e os 41 nomes da
  linguagem de fluxos. Sai diferente de zero no primeiro problema.

O leitor de string e de comentário é o de `scripts/strip-comments.py` (importado, não copiado): é
ele que sabe onde começa e onde termina string bruta do Rust, template do TypeScript, heredoc do
shell e docstring do Python. Aqui só se aproveita o espaço entre uma coisa e outra para ler
identificador — string e comentário nunca viram termo.

Classificação: um termo é português quando está em `scripts/glossary-map.txt`. O mapa cobre os
termos portugueses medidos com frequência alta; `--medir --candidatos` lista os termos medidos que
ainda não estão no mapa, para a próxima rodada de tradução. Nada aqui inventa termo: linha do mapa
cujo termo não aparece na árvore reprova a validação.

Escopo da medição: só o que o repositório publica. A varredura usa `git ls-files --cached --others
--exclude-standard`, então arquivo que o `.git/info/exclude` esconde desta máquina não entra na
contagem — sem isso a tabela gerada aqui diverge da que um clone limpo valida. A extração dos
nomes publicados segue o mesmo escopo: variável de ambiente citada só em documento interno não
entra no inventário. Sem `git` disponível, a varredura cai para a árvore de trabalho inteira.

Em `design_assets/` só os scripts utilitários contam: `design_assets/<ativo>/scripts/**` entra na
medição, e capturas, imagens e artefatos visuais ficam fora. A regra é a de `in_code_scope`, usada
também pela trava de nomes e pelo renomeador, para os três nunca divergirem de escopo.
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

RAIZ = Path(__file__).resolve().parent.parent
MAPA = RAIZ / "scripts" / "glossary-map.txt"
GLOSSARIO = RAIZ / "docs" / "GLOSSARY.md"
LEITOR = RAIZ / "scripts" / "strip-comments.py"

INICIO_TABELA = "<!-- glossario:tabela:inicio -->"
FIM_TABELA = "<!-- glossario:tabela:fim -->"
CABECALHO = "| termo_pt | termo_en | decisão | onde | ocorrências |"
SEPARADOR = "| --- | --- | --- | --- | --- |"

MIGRACOES = RAIZ / "crates" / "poph-store" / "migrations" / "harness"
BASE_CONTRATOS = RAIZ / "tests" / "contratos" / "base"
TOML_PADRAO = RAIZ / "tests" / "contratos" / "base" / "config" / "default.toml"

DESIGN_ASSETS = "design_assets/"
DESIGN_ASSETS_SCRIPTS = re.compile(r"^design_assets/[^/]+/scripts/")

IDENT_START = set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_$")
JSX_ABRE_APOS = set("(,=:[!&|?{};") | {"=>"}
CHAVE_APOS_DE = {"{", ",", ";"}
GENERICO_DE_FUNCAO = re.compile(r"<\s*[A-Za-z_$][\w$]*\s*(?:,|extends\b)")


def _proximo_visivel(fonte, indice):
    while indice < len(fonte) and fonte[indice].isspace():
        indice += 1
    return fonte[indice] if indice < len(fonte) else ""

TIPOS_PUBLICADOS = (
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


def carregar_leitor():
    spec = importlib.util.spec_from_file_location("strip_comments", LEITOR)
    modulo = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(modulo)
    return modulo


leitor = carregar_leitor()


def sem_acento(texto):
    decomposto = unicodedata.normalize("NFD", texto)
    return "".join(char for char in decomposto if not unicodedata.combining(char))


def dividir_termo(palavra):
    """Quebra `snake_case`, `kebab-case` e `camelCase` em termos minúsculos."""
    partes = []
    for bloco in re.split(r"(\d+)", palavra):
        if not bloco or bloco.isdigit():
            continue
        atual = ""
        for char in bloco:
            if not (char.isalnum()):
                if atual:
                    partes.append(atual)
                atual = ""
                continue
            if char.isupper() and atual and atual[-1].islower():
                partes.append(atual)
                atual = ""
            elif char.isupper() and len(atual) > 1 and atual[-1].isupper():
                partes.append(atual[:-1])
                atual = atual[-1]
            atual += char
        if atual:
            partes.append(atual)
    termos = []
    for parte in partes:
        termo = sem_acento(parte).lower()
        if len(termo) >= 2 and termo.isalpha():
            termos.append(termo)
    return termos


class LeitorTermos:
    """Lê identificador de código, pulando string e comentário pela sintaxe da linguagem."""

    def __init__(self, modulo):
        self.m = modulo

    def _identificador(self, fonte, indice):
        inicio = indice
        while indice < len(fonte) and (fonte[indice].isalnum() or fonte[indice] in "_$"):
            indice += 1
        return fonte[inicio:indice], indice

    def rust(self, fonte):
        tokens = []
        tamanho = len(fonte)
        indice = 0
        while indice < tamanho:
            char = fonte[indice]
            if char == "/" and fonte.startswith("//", indice):
                fim = fonte.find("\n", indice)
                indice = tamanho if fim < 0 else fim
                continue
            if char == "/" and fonte.startswith("/*", indice):
                profundidade = 0
                cursor = indice
                while cursor < tamanho:
                    if fonte.startswith("/*", cursor):
                        profundidade += 1
                        cursor += 2
                        continue
                    if fonte.startswith("*/", cursor):
                        profundidade -= 1
                        cursor += 2
                        if profundidade == 0:
                            break
                        continue
                    cursor += 1
                indice = cursor
                continue
            fim_bruta = self.m.scan_rust_raw(fonte, indice)
            if fim_bruta is not None:
                indice = fim_bruta
                continue
            if char == '"':
                indice = self.m.scan_escaped(fonte, indice + 1, '"')
                continue
            if char == "'":
                indice = self.m.scan_rust_quote(fonte, indice)
                continue
            if char in IDENT_START:
                token, indice = self._identificador(fonte, indice)
                tokens.append(token)
                continue
            indice += 1
        return tokens

    def _nomes(self, trecho):
        nomes = []
        indice = 0
        while indice < len(trecho):
            if trecho[indice] in IDENT_START:
                nome, indice = self._identificador(trecho, indice)
                nomes.append(nome)
            else:
                indice += 1
        return nomes

    def _abre_jsx(self, fonte, indice, anterior):
        proximo = fonte[indice + 1] if indice + 1 < len(fonte) else ""
        if not (proximo in IDENT_START or proximo == ">"):
            return False
        if GENERICO_DE_FUNCAO.match(fonte, indice):
            return False
        return anterior == "" or anterior in JSX_ABRE_APOS or anterior in self.m.JS_REGEX_WORDS

    def _modelo(self, fonte, indice):
        tamanho = len(fonte)
        cursor = indice + 1
        tokens = []
        while cursor < tamanho:
            char = fonte[cursor]
            if char == "\\":
                cursor += 2
                continue
            if char == "`":
                return cursor + 1, tokens
            if char == "$" and fonte.startswith("${", cursor):
                fim = self.m.scan_js_code(fonte, cursor + 2, [], True)
                tokens.extend(self.javascript(fonte[cursor + 2 : max(cursor + 2, fim - 1)]))
                cursor = fim
                continue
            cursor += 1
        return tamanho, tokens

    def javascript(self, fonte, jsx=False):
        tokens = []
        tamanho = len(fonte)
        indice = 0
        anterior = ""
        pilha = [["codigo", 0, False]]
        while indice < tamanho:
            quadro = pilha[-1]
            char = fonte[indice]
            if quadro[0] == "filhos":
                if char == "<":
                    if fonte.startswith("</", indice):
                        fim = fonte.find(">", indice)
                        fim = tamanho if fim < 0 else fim + 1
                        tokens.extend(self._nomes(fonte[indice:fim]))
                        pilha.pop()
                        indice = fim
                        anterior = "jsx"
                        continue
                    pilha.append(["tag", 0, False])
                    indice += 1
                    anterior = "<"
                    continue
                if char == "{":
                    pilha.append(["codigo", 0, True])
                    indice += 1
                    anterior = "{"
                    continue
                indice += 1
                continue
            if quadro[0] == "tag":
                if char in IDENT_START:
                    token, indice = self._identificador(fonte, indice)
                    tokens.append(token)
                    anterior = token
                    continue
                if char in "\"'":
                    indice = self.m.scan_escaped(fonte, indice + 1, char)
                    continue
                if char == "{":
                    pilha.append(["codigo", 0, True])
                    indice += 1
                    anterior = "{"
                    continue
                if fonte.startswith("/>", indice):
                    pilha.pop()
                    indice += 2
                    anterior = "jsx"
                    continue
                if char == ">":
                    quadro[0] = "filhos"
                    indice += 1
                    anterior = ">"
                    continue
                indice += 1
                continue
            if char.isspace():
                indice += 1
                continue
            if char == "/" and fonte.startswith("//", indice):
                fim = fonte.find("\n", indice)
                indice = tamanho if fim < 0 else fim
                continue
            if char == "/" and fonte.startswith("/*", indice):
                fim = fonte.find("*/", indice + 2)
                indice = tamanho if fim < 0 else fim + 2
                continue
            if char == "/":
                if anterior == "" or anterior in self.m.JS_REGEX_CHARS or anterior in self.m.JS_REGEX_WORDS:
                    fim = self.m.scan_js_regex(fonte, indice)
                    if fim is not None:
                        indice = fim
                        anterior = "regex"
                        continue
                anterior = "/"
                indice += 1
                continue
            if char == "`":
                indice, modelo = self._modelo(fonte, indice)
                tokens.extend(modelo)
                anterior = "string"
                continue
            if char in "\"'":
                fim = self.m.scan_escaped(fonte, indice + 1, char)
                if anterior in CHAVE_APOS_DE and _proximo_visivel(fonte, fim) == ":":
                    tokens.extend(self._nomes(fonte[indice + 1 : max(indice + 1, fim - 1)]))
                indice = fim
                anterior = "string"
                continue
            if char == "{":
                quadro[1] += 1
                indice += 1
                anterior = "{"
                continue
            if char == "}":
                if quadro[1] == 0 and quadro[2]:
                    pilha.pop()
                elif quadro[1] > 0:
                    quadro[1] -= 1
                indice += 1
                anterior = "}"
                continue
            if jsx and char == "<" and self._abre_jsx(fonte, indice, anterior):
                if fonte.startswith("<>", indice):
                    pilha.append(["filhos", 0, False])
                    indice += 2
                    anterior = "<"
                    continue
                pilha.append(["tag", 0, False])
                indice += 1
                anterior = "<"
                continue
            if char in IDENT_START:
                token, indice = self._identificador(fonte, indice)
                tokens.append(token)
                anterior = token
                continue
            if char == ">":
                anterior = "=>" if indice > 0 and fonte[indice - 1] == "=" else ">"
                indice += 1
                continue
            anterior = char
            indice += 1
        return tokens

    def css(self, fonte):
        tokens = []
        tamanho = len(fonte)
        indice = 0
        while indice < tamanho:
            char = fonte[indice]
            if fonte.startswith("/*", indice):
                fim = fonte.find("*/", indice + 2)
                indice = tamanho if fim < 0 else fim + 2
                continue
            if char == "@":
                indice += 1
                while indice < tamanho and (fonte[indice].isalnum() or fonte[indice] in "-_"):
                    indice += 1
                continue
            if char in "\"'":
                indice = self.m.scan_escaped(fonte, indice + 1, char)
                continue
            if char in IDENT_START:
                token, indice = self._identificador(fonte, indice)
                tokens.append(token)
                continue
            indice += 1
        return tokens

    def python(self, fonte):
        tokens = []
        tamanho = len(fonte)
        indice = 0
        while indice < tamanho:
            char = fonte[indice]
            if char == "#":
                fim = fonte.find("\n", indice)
                indice = tamanho if fim < 0 else fim
                continue
            if char in "'\"":
                indice = self.m.scan_python_string(fonte, indice, self.m.python_prefix(fonte, indice))
                continue
            if char in IDENT_START:
                token, indice = self._identificador(fonte, indice)
                tokens.append(token)
                continue
            indice += 1
        return tokens

    def shell(self, fonte):
        tokens = []
        tamanho = len(fonte)
        quadros = []
        heredocs = []
        indice = 0
        while indice < tamanho:
            char = fonte[indice]
            quadro = quadros[-1] if quadros else "raiz"
            if quadro in ("simples", "ansi"):
                if char == "\\" and quadro == "ansi":
                    indice += 2
                    continue
                if char == "'":
                    quadros.pop()
                indice += 1
                continue
            if quadro == "dupla":
                if char == "\\":
                    indice += 2
                    continue
                if char == '"':
                    quadros.pop()
                    indice += 1
                    continue
                if char == "$" and fonte.startswith("${", indice):
                    quadros.append("chave")
                    indice += 2
                    continue
                if char == "$":
                    indice += 1
                    token, indice = self._identificador(fonte, indice)
                    if token:
                        tokens.append(token)
                    continue
                indice += 1
                continue
            if quadro == "crase":
                if char == "\\":
                    indice += 2
                    continue
                if char == "`":
                    quadros.pop()
                    indice += 1
                    continue
                if char in IDENT_START:
                    token, indice = self._identificador(fonte, indice)
                    tokens.append(token)
                    continue
                indice += 1
                continue
            if quadro == "chave":
                if char == "}":
                    quadros.pop()
                    indice += 1
                    continue
                if char in IDENT_START:
                    token, indice = self._identificador(fonte, indice)
                    tokens.append(token)
                    continue
                indice += 1
                continue
            if char == "#":
                fim = fonte.find("\n", indice)
                indice = tamanho if fim < 0 else fim
                continue
            if char == "<" and fonte.startswith("<<", indice):
                analisado = self.m.parse_heredoc(fonte, indice)
                if analisado is not None:
                    fim, delimitador, tiras_tab = analisado
                    heredocs.append((delimitador, tiras_tab))
                    indice = fim
                    continue
            if char == "\n" and heredocs:
                indice = self.m.skip_heredocs(fonte, indice + 1, heredocs)
                heredocs = []
                continue
            if char == "'":
                quadros.append("simples")
                indice += 1
                continue
            if char == '"':
                quadros.append("dupla")
                indice += 1
                continue
            if char == "`":
                quadros.append("crase")
                indice += 1
                continue
            if char == "$" and indice + 1 < tamanho and fonte[indice + 1] == "'":
                quadros.append("ansi")
                indice += 2
                continue
            if char in IDENT_START:
                token, indice = self._identificador(fonte, indice)
                tokens.append(token)
                continue
            indice += 1
        return tokens

    def generico(self, fonte, marcadores_bloco=(), marcadores_linha=()):
        tokens = []
        tamanho = len(fonte)
        indice = 0
        while indice < tamanho:
            char = fonte[indice]
            casou_bloco = next(
                (marcador for marcador in marcadores_bloco if fonte.startswith(marcador, indice)), None
            )
            if casou_bloco is not None:
                fim = fonte.find("*/", indice + len(casou_bloco))
                indice = tamanho if fim < 0 else fim + 2
                continue
            casou_linha = next(
                (marcador for marcador in marcadores_linha if fonte.startswith(marcador, indice)), None
            )
            if casou_linha is not None:
                if char == "#" and indice > 0 and not (fonte[indice - 1].isspace() or fonte[indice - 1] in ";"):
                    pass
                else:
                    fim = fonte.find("\n", indice)
                    indice = tamanho if fim < 0 else fim
                    continue
            if char in "\"'`":
                if char == "`" and "`" in marcadores_linha:
                    indice += 1
                    continue
                indice = self.m.scan_escaped(fonte, indice + 1, char)
                continue
            if char in IDENT_START:
                token, indice = self._identificador(fonte, indice)
                tokens.append(token)
                continue
            indice += 1
        return tokens

    def tokens(self, caminho, linguagem):
        fonte = caminho.read_text(encoding="utf-8", errors="replace")
        if linguagem == "rust":
            return self.rust(fonte)
        if linguagem == "javascript":
            return self.javascript(fonte, jsx=caminho.suffix in (".jsx", ".tsx"))
        if linguagem == "python":
            return self.python(fonte)
        if linguagem == "shell":
            return self.shell(fonte)
        if linguagem == "css":
            return self.css(fonte)
        if linguagem == "sql":
            return self.generico(fonte, ("/*",), ("--",))
        if linguagem in ("toml", "yaml", "hash", "dockerfile"):
            return self.generico(fonte, (), ("#",))
        return []


leitor_termos = LeitorTermos(leitor)


def area_de(caminho):
    partes = caminho.parts
    if partes[0] == "crates" and len(partes) > 1:
        return partes[1]
    if partes[0] == "console":
        return "console"
    if partes[0] == "scripts":
        return "scripts"
    return "raiz"


@dataclass
class Medicao:
    contagem: Counter = field(default_factory=Counter)
    areas: dict = field(default_factory=lambda: defaultdict(Counter))
    arquivos: int = 0
    por_idioma: Counter = field(default_factory=Counter)

    def areas_do(self, termo, limite=2):
        ranking = self.areas.get(termo)
        if not ranking:
            return "-"
        ordenado = sorted(ranking.items(), key=lambda item: (-item[1], item[0]))
        return ", ".join(nome for nome, _ in ordenado[:limite])


def caminhos_do_repositorio():
    try:
        resultado = subprocess.run(
            ["git", "-C", str(RAIZ), "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
            capture_output=True,
            check=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return None
    nomes = resultado.stdout.decode("utf-8", "surrogateescape").split("\0")
    return {nome for nome in nomes if nome}


def in_code_scope(path):
    """Whether a repository path is source this project writes, and so measured and gated.

    `design_assets/` holds rendered captures, images and binaries next to the scripts that produce
    them; only `design_assets/<asset>/scripts/` is source. Excluding the whole directory would
    leave that source ungated, so the rule is the narrow one and every gate shares it.
    """
    if not path.startswith(DESIGN_ASSETS):
        return True
    return bool(DESIGN_ASSETS_SCRIPTS.match(path))


def medir():
    medicao = Medicao()
    visiveis = caminhos_do_repositorio()
    for caminho in leitor.collect_files([str(RAIZ)]):
        relativo = caminho.relative_to(RAIZ).as_posix()
        if visiveis is not None and relativo not in visiveis:
            continue
        if not in_code_scope(relativo):
            continue
        if leitor.is_skipped(caminho):
            continue
        linguagem = leitor.language_of(caminho)
        if linguagem in (None, "html"):
            continue
        medicao.arquivos += 1
        medicao.por_idioma[linguagem] += 1
        area = area_de(caminho.relative_to(RAIZ))
        for token in leitor_termos.tokens(caminho, linguagem):
            for termo in dividir_termo(token):
                medicao.contagem[termo] += 1
                medicao.areas[termo][area] += 1
    return medicao


def ler_mapa():
    mapa = {}
    duplicados = []
    if not MAPA.exists():
        return mapa, ["scripts/glossary-map.txt não existe"]
    for numero, linha in enumerate(MAPA.read_text(encoding="utf-8").splitlines(), start=1):
        if not linha.strip():
            continue
        campos = linha.split("\t")
        if len(campos) != 2:
            duplicados.append(f"linha {numero}: esperava dois campos separados por TAB: {linha!r}")
            continue
        termo, ingles = campos[0].strip(), campos[1].strip()
        if not termo or not ingles:
            duplicados.append(f"linha {numero}: termo ou tradução vazio")
            continue
        if termo in mapa:
            duplicados.append(f"linha {numero}: termo repetido: {termo}")
            continue
        mapa[termo] = ingles
    return mapa, duplicados


def nomes_da_linguagem_de_fluxos():
    fonte = (RAIZ / "crates" / "poph-fluxos" / "src" / "linguagem.rs").read_text(encoding="utf-8")
    nomes = []
    for constante in (
        "DECLARACOES",
        "VALORES",
        "EFEITOS",
        "COMBINADORES",
        "CONSTANTES",
        "FUNCOES_PURAS",
    ):
        achado = re.search(rf"pub const {constante}: &\[&str\] = &\[(.*?)\];", fonte, re.S)
        if achado is None:
            raise SystemExit(f"linguagem.rs: constante {constante} não encontrada")
        nomes.extend(re.findall(r'"([^"]+)"', achado.group(1)))
    return nomes


def esquema_do_banco():
    conexao = sqlite3.connect(":memory:")
    for migracao in sorted(MIGRACOES.glob("*.sql")):
        conexao.executescript(migracao.read_text(encoding="utf-8"))
    objetos = conexao.execute(
        "select type, name from sqlite_master where name not like 'sqlite_%' order by type, name"
    ).fetchall()
    tabelas = [nome for tipo, nome in objetos if tipo == "table"]
    indices = [nome for tipo, nome in objetos if tipo == "index"]
    gatilhos = [nome for tipo, nome in objetos if tipo == "trigger"]
    colunas = []
    for tabela in tabelas:
        linhas = conexao.execute(f'pragma table_info("{tabela}")').fetchall()
        colunas.extend(linha[1] for linha in linhas)
    conexao.close()
    return tabelas, colunas, indices, gatilhos


def openapi():
    documento = json.loads((BASE_CONTRATOS / "openapi" / "openapi.json").read_text(encoding="utf-8"))
    rotas = sorted(documento.get("paths", {}))
    operacoes = set()
    for item in documento.get("paths", {}).values():
        for especificacao in item.values():
            if isinstance(especificacao, dict) and especificacao.get("operationId"):
                operacoes.add(especificacao["operationId"])
    campos = set()
    for esquema in documento.get("components", {}).get("schemas", {}).values():
        campos.update((esquema.get("properties") or {}).keys())
    return rotas, sorted(operacoes), sorted(campos)


def cli():
    comandos = set()
    flags = set()
    for arquivo in sorted((BASE_CONTRATOS / "cli").glob("*.txt")):
        if arquivo.stem.startswith("poph__version"):
            continue
        comandos.add(" ".join(arquivo.stem.split("__")).replace("_poph", "poph"))
        for flag in re.findall(r"(?<![\w-])(--?[A-Za-z][A-Za-z0-9-]*)", arquivo.read_text(encoding="utf-8")):
            flags.add(flag)
    return sorted(comandos), sorted(flags)


def chaves_toml():
    with open(TOML_PADRAO, "rb") as handle:
        dados = tomllib.load(handle)
    chaves = []

    def visitar(prefixo, valor):
        for chave, conteudo in valor.items():
            caminho = f"{prefixo}.{chave}" if prefixo else chave
            chaves.append(caminho)
            if isinstance(conteudo, dict):
                visitar(caminho, conteudo)

    visitar("", dados)
    return sorted(chaves)


def variaveis_de_ambiente():
    padrao = re.compile(r"\b(?:POPH_[A-Z0-9_]+|[A-Z][A-Z0-9_]*_API_KEY)\b")
    visiveis = caminhos_do_repositorio()
    nomes = set()
    for arquivo in sorted((RAIZ / "docs").glob("*.md")) + [RAIZ / "README.md"]:
        if not arquivo.exists():
            continue
        if visiveis is not None and arquivo.relative_to(RAIZ).as_posix() not in visiveis:
            continue
        nomes.update(padrao.findall(arquivo.read_text(encoding="utf-8")))
    return sorted(nomes)


def metricas():
    nomes = []
    for linha in (BASE_CONTRATOS / "metrics" / "types.txt").read_text(encoding="utf-8").splitlines():
        achado = re.match(r"# TYPE (\S+) ", linha)
        if achado:
            nomes.append(achado.group(1))
    return nomes


def binarios():
    nomes = set()
    for manifesto in sorted(RAIZ.glob("crates/*/Cargo.toml")) + [RAIZ / "Cargo.toml"]:
        texto = manifesto.read_text(encoding="utf-8")
        nomes.update(re.findall(r"\[\[bin\]\]\s*\nname = \"([^\"]+)\"", texto))
    for principal in sorted(RAIZ.glob("crates/*/src/main.rs")):
        nomes.add(principal.parent.parent.name)
    return sorted(nomes)


def chaves_de_json(texto):
    chaves = set()

    def visitar(valor):
        if isinstance(valor, dict):
            for chave, conteudo in valor.items():
                chaves.add(chave)
                visitar(conteudo)
        elif isinstance(valor, list):
            for item in valor:
                visitar(item)

    for bloco in re.findall(r"```json\n(.*?)```", texto, re.S):
        try:
            visitar(json.loads(bloco))
        except json.JSONDecodeError:
            continue
    return chaves


def protocolo_de_plugin():
    texto = (RAIZ / "docs" / "PLUGIN_PROTOCOLO.md").read_text(encoding="utf-8")
    return sorted(chaves_de_json(texto))


def formato_de_backup():
    texto = (RAIZ / "docs" / "BACKUP.md").read_text(encoding="utf-8")
    campos = set()
    for linha in texto.splitlines():
        if not linha.startswith("| `"):
            continue
        primeira = linha.split("|")[1]
        achado = re.search(r"`([^`]+)`", primeira)
        if achado:
            campos.add(achado.group(1))
    return sorted(campos)


def inventario_publicado():
    tabelas, colunas, indices, gatilhos = esquema_do_banco()
    rotas, operacoes, campos_json = openapi()
    comandos, flags = cli()
    return {
        "linguagem_fluxos": nomes_da_linguagem_de_fluxos(),
        "binario": binarios(),
        "variavel_ambiente": variaveis_de_ambiente(),
        "metrica": metricas(),
        "tabela": tabelas,
        "coluna": colunas,
        "indice": indices,
        "gatilho": gatilhos,
        "rota": rotas,
        "operation_id": operacoes,
        "campo_json": campos_json,
        "comando_cli": comandos,
        "flag_cli": flags,
        "chave_toml": chaves_toml(),
        "protocolo_plugin": protocolo_de_plugin(),
        "backup": formato_de_backup(),
    }


def termos_congelados(inventario, mapa):
    """Termo português → escopos onde ele é nome publicado.

    Os 41 nomes da linguagem de fluxos são congelados inteiros (a palavra `mensagem_ao_cliente`
    não se divide: é ela que está gravada no banco). Os demais nomes publicados entram pelos
    termos que os compõem, e só quando o termo é português: nome publicado em inglês não precisa
    de linha no glossário.
    """
    congelados = defaultdict(set)
    for nome in inventario["linguagem_fluxos"]:
        congelados[nome].add("linguagem de fluxos")
    for tipo in TIPOS_PUBLICADOS:
        if tipo == "linguagem_fluxos":
            continue
        for nome in inventario[tipo]:
            for termo in dividir_termo(nome):
                if termo not in mapa:
                    continue
                congelados[termo].add(f"{tipo} {nome}")
    return {termo: sorted(escopos) for termo, escopos in congelados.items()}


def celula_onde(escopos, limite=3):
    mostrados = escopos[:limite]
    texto = "; ".join(mostrados)
    if len(escopos) > limite:
        texto += f"; e mais {len(escopos) - limite}"
    return texto.replace("|", "/")


def linhas_da_tabela(medicao, mapa, congelados):
    linhas = []
    for termo, ingles in mapa.items():
        contagem = medicao.contagem.get(termo, 0)
        if contagem == 0:
            continue
        linhas.append(
            (termo, ingles, "traduz", medicao.areas_do(termo), str(contagem), -contagem)
        )
    linhas.sort(key=lambda linha: (linha[5], linha[0]))
    traduz = [[linha[0], linha[1], linha[2], linha[3], linha[4]] for linha in linhas]
    congelado = []
    for termo in sorted(congelados):
        congelado.append([termo, "—", "congelado", celula_onde(congelados[termo]), "-"])
    return traduz + congelado


def tabela_markdown(linhas):
    partes = [INICIO_TABELA, CABECALHO, SEPARADOR]
    for linha in linhas:
        partes.append("| " + " | ".join(linha) + " |")
    partes.append(FIM_TABELA)
    return "\n".join(partes)


def documento_gerado(medicao, mapa, congelados):
    return tabela_markdown(linhas_da_tabela(medicao, mapa, congelados))


def substituir_regiao(documento, tabela):
    inicio = documento.find(INICIO_TABELA)
    fim = documento.find(FIM_TABELA)
    if inicio < 0 or fim < 0:
        raise SystemExit(
            "docs/GLOSSARY.md: faltam os marcadores "
            f"{INICIO_TABELA} e {FIM_TABELA}"
        )
    return documento[:inicio] + tabela + documento[fim + len(FIM_TABELA) :]


def ler_tabela(documento):
    inicio = documento.find(INICIO_TABELA)
    fim = documento.find(FIM_TABELA)
    if inicio < 0 or fim < 0:
        return None, [f"faltam os marcadores {INICIO_TABELA} e {FIM_TABELA}"]
    bloco = documento[inicio + len(INICIO_TABELA) : fim].strip("\n").splitlines()
    problemas = []
    if len(bloco) < 2:
        return None, ["a região da tabela está vazia"]
    if bloco[0].strip() != CABECALHO:
        problemas.append(f"cabeçalho diferente do esperado: {bloco[0].strip()!r}")
    if bloco[1].strip() != SEPARADOR:
        problemas.append("linha separadora diferente da esperada")
    linhas = []
    for numero, linha in enumerate(bloco[2:], start=3):
        if not linha.startswith("|"):
            problemas.append(f"linha {numero}: não é linha de tabela: {linha!r}")
            continue
        celulas = [celula.strip() for celula in linha.strip().strip("|").split("|")]
        if len(celulas) != 5:
            problemas.append(f"linha {numero}: esperava 5 colunas, achei {len(celulas)}: {linha!r}")
            continue
        linhas.append((numero, celulas))
    return linhas, problemas


def validar(medicao, mapa, erros_do_mapa, inventario, congelados):
    falhas = list(erros_do_mapa)
    documento = GLOSSARIO.read_text(encoding="utf-8")
    linhas, problemas = ler_tabela(documento)
    falhas.extend(problemas)
    if linhas is None:
        return falhas, {"traduz": 0, "congelado": 0}

    vistos = set()
    por_decisao = Counter()
    ocorrencias = {}
    for numero, celulas in linhas:
        termo, ingles, decisao, onde, contagem = celulas
        if decisao not in ("traduz", "congelado"):
            falhas.append(f"linha {numero}: decisão inválida: {decisao!r}")
            continue
        if (termo, decisao) in vistos:
            falhas.append(f"linha {numero}: termo repetido para {decisao}: {termo}")
            continue
        vistos.add((termo, decisao))
        por_decisao[decisao] += 1
        if not onde or onde == "-":
            falhas.append(f"linha {numero}: coluna onde vazia em {termo}")
        if decisao == "traduz":
            if not ingles or ingles == "—":
                falhas.append(f"linha {numero}: tradução vazia em {termo}")
            if not contagem.isdigit():
                falhas.append(f"linha {numero}: ocorrências de {termo} deveria ser número: {contagem!r}")
                continue
            ocorrencias[termo] = int(contagem)

    for termo in mapa:
        medido = medicao.contagem.get(termo, 0)
        if medido == 0:
            continue
        if (termo, "traduz") not in vistos:
            falhas.append(f"{termo}: está no mapa e não tem linha 'traduz' na tabela")
            continue
        if ocorrencias.get(termo) != medido:
            falhas.append(
                f"{termo}: tabela diz {ocorrencias.get(termo)}, medição diz {medido} (tabela velha)"
            )

    for termo in congelados:
        if (termo, "congelado") not in vistos:
            falhas.append(f"{termo}: nome publicado sem linha 'congelado' na tabela")

    for termo in sorted(set(mapa) & set(congelados)):
        if (termo, "traduz") not in vistos or (termo, "congelado") not in vistos:
            falhas.append(f"{termo}: tem os dois papéis e precisa das duas linhas")

    nomes_da_linguagem = nomes_da_linguagem_de_fluxos()
    if len(nomes_da_linguagem) != 41:
        falhas.append(f"linguagem.rs: esperava 41 nomes, achei {len(nomes_da_linguagem)}")
    for nome in nomes_da_linguagem:
        if (nome, "congelado") not in vistos:
            falhas.append(f"{nome}: palavra da linguagem de fluxos sem linha 'congelado'")

    esperado = documento_gerado(medicao, mapa, congelados)
    if esperado not in documento:
        falhas.append("a tabela em disco difere da gerada: rode --gerar")

    return falhas, {"traduz": por_decisao["traduz"], "congelado": por_decisao["congelado"]}


def imprimir_medicao(medicao, mapa, top, candidatos, minimo):
    portugueses = [
        (termo, medicao.contagem[termo]) for termo in mapa if medicao.contagem.get(termo, 0) > 0
    ]
    portugueses.sort(key=lambda item: (-item[1], item[0]))
    print(f"arquivos lidos: {medicao.arquivos}")
    print("por linguagem: " + ", ".join(f"{nome} {total}" for nome, total in sorted(medicao.por_idioma.items())))
    print(f"termos distintos: {len(medicao.contagem)}")
    print(f"termos portugueses no mapa, medidos: {len(portugueses)}")
    print()
    print("| ocorrências | termo | onde |")
    print("| ---: | --- | --- |")
    for termo, contagem in portugueses[:top]:
        print(f"| {contagem} | {termo} | {medicao.areas_do(termo)} |")
    if candidatos:
        fora_do_mapa = [
            (termo, contagem)
            for termo, contagem in medicao.contagem.items()
            if termo not in mapa and contagem >= minimo
        ]
        fora_do_mapa.sort(key=lambda item: (-item[1], item[0]))
        print()
        print(f"candidatos fora do mapa (ocorrências >= {minimo}): {len(fora_do_mapa)}")
        print(", ".join(f"{termo}:{contagem}" for termo, contagem in fora_do_mapa))


def imprimir_publicados(inventario, mapa, congelados):
    for tipo in TIPOS_PUBLICADOS:
        nomes = inventario[tipo]
        termos = sorted({termo for termo in congelados if any(f"{tipo} {nome}" in congelados[termo] for nome in nomes)})
        print(f"== {tipo}: {len(nomes)} nomes, {len(termos)} termos portugueses congelados")
        exemplos = nomes[:6]
        print("   " + ", ".join(exemplos) + (" …" if len(nomes) > len(exemplos) else ""))
        if termos:
            print("   termos: " + ", ".join(termos))
    print()
    print(f"termos congelados no total: {len(congelados)}")
    print(f"termos traduzidos: {len(mapa)}")


def main(argv):
    parser = argparse.ArgumentParser(
        prog="glossary-terms.py",
        description="Mede os termos de identificador, extrai os nomes publicados e valida docs/GLOSSARY.md.",
    )
    acao = parser.add_mutually_exclusive_group(required=True)
    acao.add_argument("--medir", action="store_true", help="conta os termos e lista os mais frequentes")
    acao.add_argument("--publicados", action="store_true", help="lista os nomes publicados por tipo")
    acao.add_argument("--gerar", action="store_true", help="reescreve a tabela de docs/GLOSSARY.md")
    acao.add_argument("--validar", action="store_true", help="confere a tabela contra a medição")
    parser.add_argument("--top", type=int, default=40, help="quantas linhas mostrar com --medir (padrão: 40)")
    parser.add_argument("--minimo", type=int, default=20, help="corte dos candidatos com --medir (padrão: 20)")
    parser.add_argument("--candidatos", action="store_true", help="com --medir, lista os termos fora do mapa")
    argumentos = parser.parse_args(argv)

    mapa, erros_do_mapa = ler_mapa()
    if argumentos.medir:
        medicao = medir()
        imprimir_medicao(medicao, mapa, argumentos.top, argumentos.candidatos, argumentos.minimo)
        return 0

    inventario = inventario_publicado()
    congelados = termos_congelados(inventario, mapa)

    if argumentos.publicados:
        imprimir_publicados(inventario, mapa, congelados)
        return 0

    medicao = medir()

    if argumentos.gerar:
        documento = GLOSSARIO.read_text(encoding="utf-8")
        novo = substituir_regiao(documento, documento_gerado(medicao, mapa, congelados))
        GLOSSARIO.write_text(novo, encoding="utf-8")
        linhas = linhas_da_tabela(medicao, mapa, congelados)
        print(f"glossário: {len(linhas)} linha(s) gravada(s) em docs/GLOSSARY.md")
        return 0

    falhas, contagem = validar(medicao, mapa, erros_do_mapa, inventario, congelados)
    if falhas:
        for falha in falhas[:80]:
            print(f"glossário: {falha}", file=sys.stderr)
        if len(falhas) > 80:
            print(f"glossário: e mais {len(falhas) - 80} problema(s)", file=sys.stderr)
        print(f"glossário: {len(falhas)} problema(s)", file=sys.stderr)
        return 1
    nomes_fluxos = len(nomes_da_linguagem_de_fluxos())
    print(
        "glossário: OK "
        f"({contagem['traduz']} termos traduzidos, {contagem['congelado']} congelados, "
        f"{nomes_fluxos} palavras da linguagem de fluxos, {medicao.arquivos} arquivos medidos)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
