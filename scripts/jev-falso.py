#!/usr/bin/env python3
"""Local fake Jev — only to measure the gate's *mechanics* when the real Jev is unreachable.

**This is not the Jev.** It is a keyword classifier that implements the contract of
`POST /v1/systemone` (question `choice` → `answers[id]` with `choice`/`confidence`/`probabilities`),
so that `poph jev eval` and the smoke tests run without network access and without a real key.

The number it produces measures the **labeled set and the plumbing** (thresholds, cache, fallback,
false-positive and false-negative counts), not the quality of the real model. The official measurement
of a gate is done with the real Jev — `[jev].url_base = "https://api.typesafe.ai"`, no gateway (the client
speaks https since HR-10b) — see `docs/JEV.md` §7.

**It does not read the instruction or the criteria**: only the `state` and the name of each question. A
change to a gate's wording (such as the one in HR-29a) changes nothing here, and that is why its effect can
only be measured with the real Jev.

Usage:
    python3 scripts/jev-falso.py --porta 18099
    # in another terminal:
    TYPESAFE_API_KEY=ficticia POPH_CONFIG=/tmp/jev/poph.toml \
        cargo run -p poph -- jev eval g1 tests/jev/g1.jsonl

What it answers:
- `g1_transferir`: explicit request for a person, configured situation, complaint/dissatisfaction,
  urgency, repeated request or health risk → `transferir_agora`; the rest → `continuar_agente`.
- `g2_esperar`: cut-off sentence (dangling ending, ellipsis, colon) or a short request to prepare
  ("peraí", "deixa eu ver", "vou mandar o comprovante") → `esperar_mais`; the rest →
  `processar_agora`.
- `risco_imediato`: `ingestao_contato_produto`, `animal_peconhento`, `outro_risco_saude` or `nenhum`.
- `g4_departamento`: one of the `politica.departamentos` of the state, by keyword.
"""

from __future__ import annotations

import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PERSON = [
    "atendente",
    "uma pessoa",
    "um humano",
    "humano",
    "responsável",
    "responsavel",
    "gerente",
    "dono",
    "falar com alguém",
    "falar com alguem",
    "me transfere",
    "me passa para",
    "transferir",
]
DISSATISFACTION = [
    "reclamar",
    "reclamação",
    "reclamacao",
    "absurdo",
    "insatisfeito",
    "insatisfeita",
    "não resolveu",
    "nao resolveu",
    "ninguém resolve",
    "ninguem resolve",
    "ninguém me responde",
    "ninguem me responde",
    "processar",
    "de novo",
    "terceira vez",
    "muito insatisfeito",
]
URGENCY = ["urgente", "hoje", "desde ontem", "prazo acaba", "ninguém confirmou"]
SITUATIONS = {
    "devolução e troca": ["devolver", "devolução", "devolucao", "troca", "veio quebrado"],
    "reclamação": ["reclamar", "reclamação", "reclamacao"],
    "segunda via de nota fiscal": ["segunda via", "nota fiscal"],
    "cancelamento de pedido": ["cancelar"],
    "visita técnica não realizada": ["técnico não apareceu", "tecnico nao apareceu"],
    "pedido de desconto": ["desconto", "mais barato", "pagar menos"],
}
INGESTION_RISK = [
    "engoliu",
    "engolir",
    "comeu a isca",
    "comeu o veneno",
    "tomou o veneno",
    "passou mal",
    "contato com o produto",
    "inalou",
    "intoxic",
]
ANIMAL_RISK = ["escorpião", "escorpiao", "aranha", "cobra", "abelha", "marimbondo", "picou"]
OTHER_RISK = ["falta de ar", "convulsão", "convulsao", "desmaio", "dor no peito", "sangramento"]


def text_of_state(state: dict) -> str:
    messages = (state.get("conversa") or {}).get("mensagens") or []
    last_messages = [
        (message.get("texto") or "").lower()
        for message in messages
        if message.get("papel") == "cliente"
    ]
    return last_messages[-1] if last_messages else ""


def g1(state: dict) -> str:
    text = text_of_state(state)
    if any(word in text for word in PERSON):
        return "transferir_agora"
    if any(word in text for word in INGESTION_RISK + ANIMAL_RISK + OTHER_RISK):
        return "transferir_agora"
    if any(word in text for word in DISSATISFACTION):
        return "transferir_agora"
    if any(word in text for word in URGENCY):
        return "transferir_agora"
    policy = (state.get("politica") or {}).get("situacoes_de_handoff") or []
    for situation in policy:
        key = situation.lower().strip()
        for name, words in SITUATIONS.items():
            if key == name and any(word in text for word in words):
                return "transferir_agora"
    return "continuar_agente"


def immediate_risk(state: dict) -> str:
    text = text_of_state(state)
    if any(word in text for word in INGESTION_RISK):
        return "ingestao_contato_produto"
    if any(word in text for word in ANIMAL_RISK):
        return "animal_peconhento"
    if any(word in text for word in OTHER_RISK):
        return "outro_risco_saude"
    return "nenhum"


PREPARATION = [
    "deixa eu ver",
    "me deixa ver",
    "deixa eu abrir",
    "deixa eu pensar",
    "peraí",
    "perai",
    "um momento",
    "só um instante",
    "so um instante",
    "aguarda",
    "espera que",
    "vou mandar",
    "vou te mandar",
    "vou tirar",
    "vou ver",
    "vou confirmar",
    "já te mando",
    "ja te mando",
]
INCOMPLETE_ENDINGS = {
    "o", "a", "os", "as", "de", "do", "da", "dos", "das", "que", "e", "é", "eh", "em", "no",
    "na", "nos", "nas", "com", "para", "pra", "por", "se", "meu", "minha", "seu", "sua", "ao",
    "um", "uma", "sobre", "porque", "como", "onde", "quando", "sabe", "sabe?",
}


def g2(state: dict) -> str:
    text = text_of_state(state).strip()
    if not text:
        return "esperar_mais"
    low = text.lower()
    if low.endswith(("...", "…", ":")):
        return "esperar_mais"
    words = low.replace("?", " ?").split()
    if words and words[-1].strip(".,!?") in INCOMPLETE_ENDINGS:
        return "esperar_mais"
    if len(words) <= 8 and any(phrase in low for phrase in PREPARATION):
        return "esperar_mais"
    return "processar_agora"


TOPICS_G4 = [
    ("Fiscal", ["nota fiscal", "cfop", "imposto", "tribut", "cte", "mdfe", "documento fiscal"]),
    ("Financeiro", ["boleto", "cobran", "fatura", "estorno", "pagamento", "pix", "vencimento",
                    "reembols", "cartão", "cartao", "caiu"]),
    ("Jurídico", ["jurídico", "juridico", "contrato", "judicial", "cláusula", "clausula"]),
    ("Recursos Humanos", ["funcionário", "funcionario", "treinamento", " rh ", "rh,"]),
    ("Marketing", ["divulgação", "divulgacao", "campanha", "marketing"]),
    ("Tecnologia", ["api", "sistema", "navegador", "integração", "integracao"]),
    ("Qualidade", ["conformidade", "especificação", "especificacao", "lote"]),
    ("Compras", ["fornecedor", "matéria-prima", "materia-prima", "fornecimento"]),
    ("Diretoria", ["diretoria", "direção", "direcao", "escalar"]),
    ("Logística", ["rastreio", "transportadora", "carga", "doca", "atrasad", "depósito", "deposito",
                   "centro de distribuição", "não chegou", "nao chegou", "pedido parado"]),
    ("Suporte", ["quebrado", "defeito", "troca", "trocar", "garantia", "devolu", "barulho",
                 "riscado", "parou de funcionar", "avariado", "faltando", "piscando", "errado",
                 "cancelar a compra", "pedido de compra", "segunda via"]),
    ("Comercial", ["preço", "preco", "orçamento", "orcamento", "desconto", "comprar", "cotação",
                   "cotacao", "prazo de entrega", "formas de pagamento", "pronta entrega",
                   "estoque", "negociar", "fechar"]),
]


def g4(state: dict) -> str | None:
    """Department by keyword, among the candidates the state brought."""
    candidates = (state.get("politica") or {}).get("departamentos") or []
    if len(candidates) < 2:
        return None
    by_name = {candidate.lower(): candidate for candidate in candidates}
    text = text_of_state(state)
    for department, words in TOPICS_G4:
        if department.lower() not in by_name:
            continue
        if any(word in text for word in words):
            return by_name[department.lower()]
    return candidates[0]


ANSWERS = {
    "g1_transferir": g1,
    "g2_esperar": g2,
    "risco_imediato": immediate_risk,
    "g4_departamento": g4,
}


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_POST(self) -> None:  # noqa: N802 (library name)
        size = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(size) or b"{}")
        state = body.get("state") or {}
        answers = {}
        for question_id in (body.get("questions") or {}):
            choice = ANSWERS.get(question_id, lambda _state: None)(state)
            if choice is None:
                continue
            answers[question_id] = {
                "type": "choice",
                "choice": choice,
                "probabilities": {choice: 0.86},
                "confidence": 0.86,
            }
        output = json.dumps(
            {"answers": answers, "usage": {"input_tokens": 40, "output_tokens": 4}}
        ).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(output)))
        self.end_headers()
        self.wfile.write(output)

    def log_message(self, log_format: str, *args) -> None:
        pass


def main() -> None:
    parser = argparse.ArgumentParser(description="Local fake Jev (systemone contract)")
    parser.add_argument("--porta", type=int, default=18099)
    args = parser.parse_args()
    server = ThreadingHTTPServer(("127.0.0.1", args.porta), Handler)
    print(f"Fake Jev at http://127.0.0.1:{args.porta} (Ctrl-C to stop)")
    server.serve_forever()


if __name__ == "__main__":
    main()
