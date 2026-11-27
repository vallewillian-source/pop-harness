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

ROOT_BACKEND = pathlib.Path(__file__).resolve().parents[6] / "backend"
sys.path.insert(0, str(ROOT_BACKEND))

from core.agent.instructions_composer import compose_instructions  # noqa: E402
from core.agent.instructions_models import StructuredInstructions  # noqa: E402

FOLDER = pathlib.Path(__file__).resolve().parent


def main() -> int:
    generated = 0
    for entry in sorted(FOLDER.glob("*.json")):
        payload = json.loads(entry.read_text(encoding="utf-8"))
        text = compose_instructions(StructuredInstructions.model_validate(payload))
        output = entry.with_name(entry.stem + ".esperado.txt")
        output.write_text(text, encoding="utf-8")
        print(f"{entry.name}: {len(text)} caracteres, {len(text.encode('utf-8'))} bytes")
        generated += 1
    print(f"{generated} referência(s) gerada(s) em {FOLDER}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
