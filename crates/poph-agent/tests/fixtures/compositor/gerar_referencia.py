"""Gera a saída de referência do compositor do POP Atende para os fixtures de paridade da HR-09.

Uso (a partir da raiz do monorepo):
    cd apps/backend && .venv/bin/python ../pop-harness/crates/poph-core/tests/fixtures/compositor/gerar_referencia.py

Para cada `<nome>.json` na pasta, grava `<nome>.esperado.txt` exatamente com o texto que
`core.agent.instructions_composer.compose_instructions` devolve hoje. Nenhum arquivo do backend
é alterado; o compositor é apenas importado e chamado.
"""

from __future__ import annotations

import json
import pathlib
import sys

RAIZ_BACKEND = pathlib.Path(__file__).resolve().parents[6] / "backend"
sys.path.insert(0, str(RAIZ_BACKEND))

from core.agent.instructions_composer import compose_instructions  # noqa: E402
from core.agent.instructions_models import StructuredInstructions  # noqa: E402

PASTA = pathlib.Path(__file__).resolve().parent


def main() -> int:
    gerados = 0
    for entrada in sorted(PASTA.glob("*.json")):
        dados = json.loads(entrada.read_text(encoding="utf-8"))
        texto = compose_instructions(StructuredInstructions.model_validate(dados))
        saida = entrada.with_name(entrada.stem + ".esperado.txt")
        saida.write_text(texto, encoding="utf-8")
        print(f"{entrada.name}: {len(texto)} caracteres, {len(texto.encode('utf-8'))} bytes")
        gerados += 1
    print(f"{gerados} referência(s) gerada(s) em {PASTA}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
