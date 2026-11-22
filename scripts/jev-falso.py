#!/usr/bin/env python3
"""Jev falso local — só para medir a *mecânica* do gate quando o Jev real não é alcançável.

**Não é o Jev.** É um classificador de palavras-chave que implementa o contrato de
`POST /v1/systemone` (pergunta `choice` → `answers[id]` com `choice`/`confidence`/`probabilities`),
para rodar `poph jev eval` e os testes de fumaça sem rede externa e sem chave real.

O número que ele produz mede o **conjunto rotulado e o encanamento** (limiares, cache, fallback,
contagem de falso positivo e falso negativo), não a qualidade do modelo de verdade. A medição
oficial do gate é com o Jev real — `[jev].url_base = "https://api.typesafe.ai"`, sem gateway (o
cliente fala https desde a HR-10b) — ver `docs/JEV.md` §7.

**Ele não lê a instrução nem as criteria**: só o `state` e o nome de cada pergunta. Mudança de
redação de um gate (como a da HR-29a) não muda nada aqui, e é por isso que o efeito dela só se mede
com o Jev real.

Uso:
    python3 scripts/jev-falso.py --porta 18099
    # noutro terminal:
    TYPESAFE_API_KEY=ficticia POPH_CONFIG=/tmp/jev/poph.toml \
        cargo run -p poph -- jev eval g1 tests/jev/g1.jsonl

O que ele responde:
- `g1_transferir`: pedido explícito de pessoa, situação configurada, reclamação/insatisfação,
  urgência, pedido repetido ou risco à saúde → `transferir_agora`; resto → `continuar_agente`.
- `g2_esperar`: frase cortada (final pendurado, reticências, dois pontos) ou pedido de preparo
  curto ("peraí", "deixa eu ver", "vou mandar o comprovante") → `esperar_mais`; resto →
  `processar_agora`.
- `risco_imediato`: `ingestao_contato_produto`, `animal_peconhento`, `outro_risco_saude` ou `nenhum`.
- `g4_departamento`: um dos departamentos de `politica.departamentos` do estado, por palavra-chave.
"""

from __future__ import annotations

import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# Pedido explícito de pessoa: as palavras que o POP Atende trata como transferência.
PESSOA = [
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
# Insatisfação, reclamação e pedido repetido.
INSATISFACAO = [
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
URGENCIA = ["urgente", "hoje", "desde ontem", "prazo acaba", "ninguém confirmou"]
# Situações que a empresa costuma configurar em `situacoes_de_handoff`.
SITUACOES = {
    "devolução e troca": ["devolver", "devolução", "devolucao", "troca", "veio quebrado"],
    "reclamação": ["reclamar", "reclamação", "reclamacao"],
    "segunda via de nota fiscal": ["segunda via", "nota fiscal"],
    "cancelamento de pedido": ["cancelar"],
    "visita técnica não realizada": ["técnico não apareceu", "tecnico nao apareceu"],
    "pedido de desconto": ["desconto", "mais barato", "pagar menos"],
}
# Risco à saúde descrito pelo cliente.
RISCO_INGESTAO = [
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
RISCO_ANIMAL = ["escorpião", "escorpiao", "aranha", "cobra", "abelha", "marimbondo", "picou"]
RISCO_OUTRO = ["falta de ar", "convulsão", "convulsao", "desmaio", "dor no peito", "sangramento"]


def texto_do_estado(estado: dict) -> str:
    mensagens = (estado.get("conversa") or {}).get("mensagens") or []
    ultimas = [
        (mensagem.get("texto") or "").lower()
        for mensagem in mensagens
        if mensagem.get("papel") == "cliente"
    ]
    return ultimas[-1] if ultimas else ""


def g1(estado: dict) -> str:
    texto = texto_do_estado(estado)
    if any(palavra in texto for palavra in PESSOA):
        return "transferir_agora"
    if any(palavra in texto for palavra in RISCO_INGESTAO + RISCO_ANIMAL + RISCO_OUTRO):
        return "transferir_agora"
    if any(palavra in texto for palavra in INSATISFACAO):
        return "transferir_agora"
    if any(palavra in texto for palavra in URGENCIA):
        return "transferir_agora"
    politica = (estado.get("politica") or {}).get("situacoes_de_handoff") or []
    for situacao in politica:
        chave = situacao.lower().strip()
        for nome, palavras in SITUACOES.items():
            if chave == nome and any(palavra in texto for palavra in palavras):
                return "transferir_agora"
    return "continuar_agente"


def risco(estado: dict) -> str:
    texto = texto_do_estado(estado)
    if any(palavra in texto for palavra in RISCO_INGESTAO):
        return "ingestao_contato_produto"
    if any(palavra in texto for palavra in RISCO_ANIMAL):
        return "animal_peconhento"
    if any(palavra in texto for palavra in RISCO_OUTRO):
        return "outro_risco_saude"
    return "nenhum"


# G2 (HR-30): o cliente está no meio da ideia? Palavras de preparo e finais que pedem continuação.
PREPARO = [
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
# Última palavra que deixa a frase pendurada.
FINAIS_INCOMPLETOS = {
    "o", "a", "os", "as", "de", "do", "da", "dos", "das", "que", "e", "é", "eh", "em", "no",
    "na", "nos", "nas", "com", "para", "pra", "por", "se", "meu", "minha", "seu", "sua", "ao",
    "um", "uma", "sobre", "porque", "como", "onde", "quando", "sabe", "sabe?",
}


def g2(estado: dict) -> str:
    texto = texto_do_estado(estado).strip()
    if not texto:
        return "esperar_mais"
    baixo = texto.lower()
    if baixo.endswith(("...", "…", ":")):
        return "esperar_mais"
    palavras = baixo.replace("?", " ?").split()
    if palavras and palavras[-1].strip(".,!?") in FINAIS_INCOMPLETOS:
        return "esperar_mais"
    if len(palavras) <= 8 and any(frase in baixo for frase in PREPARO):
        return "esperar_mais"
    return "processar_agora"


# G4 (HR-32): assunto do cliente por departamento. A ordem importa — o primeiro que casar vence,
# e o que não casa com nenhum cai no primeiro candidato do estado.
ASSUNTOS_G4 = [
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


def g4(estado: dict) -> str | None:
    """Departamento por palavra-chave, entre os candidatos que o estado trouxe."""
    candidatos = (estado.get("politica") or {}).get("departamentos") or []
    if len(candidatos) < 2:
        return None
    por_nome = {candidato.lower(): candidato for candidato in candidatos}
    texto = texto_do_estado(estado)
    for departamento, palavras in ASSUNTOS_G4:
        if departamento.lower() not in por_nome:
            continue
        if any(palavra in texto for palavra in palavras):
            return por_nome[departamento.lower()]
    return candidatos[0]


RESPOSTAS = {
    "g1_transferir": g1,
    "g2_esperar": g2,
    "risco_imediato": risco,
    "g4_departamento": g4,
}


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_POST(self) -> None:  # noqa: N802 (nome da lib)
        tamanho = int(self.headers.get("Content-Length") or 0)
        corpo = json.loads(self.rfile.read(tamanho) or b"{}")
        estado = corpo.get("state") or {}
        respostas = {}
        for pergunta_id in (corpo.get("questions") or {}):
            escolha = RESPOSTAS.get(pergunta_id, lambda _estado: None)(estado)
            if escolha is None:
                continue
            respostas[pergunta_id] = {
                "type": "choice",
                "choice": escolha,
                "probabilities": {escolha: 0.86},
                "confidence": 0.86,
            }
        saida = json.dumps(
            {"answers": respostas, "usage": {"input_tokens": 40, "output_tokens": 4}}
        ).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(saida)))
        self.end_headers()
        self.wfile.write(saida)

    def log_message(self, formato: str, *args) -> None:
        pass


def main() -> None:
    parser = argparse.ArgumentParser(description="Jev falso local (contrato systemone)")
    parser.add_argument("--porta", type=int, default=18099)
    args = parser.parse_args()
    servidor = ThreadingHTTPServer(("127.0.0.1", args.porta), Handler)
    print(f"Jev falso em http://127.0.0.1:{args.porta} (Ctrl-C para parar)")
    servidor.serve_forever()


if __name__ == "__main__":
    main()
