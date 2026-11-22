#!/usr/bin/env python3
"""Lê a saída `--json` do `poph jev eval` e imprime a matriz de confusão do gate.

O `poph jev eval` conta acerto/abstenção/erro; para decidir se o gate pode ser ligado o que importa
é outra coisa: o erro que o cliente sente. Cada gate tem o seu:

- **G1** (`--gate g1`, padrão): falso negativo é a transferência que devia acontecer e não aconteceu
  (abstenção cai em "continuar", então conta como falso negativo); falso positivo é a transferência
  indevida.
- **G2** (`--gate g2`): falso negativo é **processar cedo** (a mensagem estava incompleta e a frase
  saiu partida); falso positivo é **esperar sem necessidade** (latência a mais). A abstenção cai em
  "processar agora" na janela fixa, então conta como falso negativo quando o rótulo era esperar.
- **G4** (`--gate g4`): o erro grave é **escolha confiante errada** (a conversa vai para o time
  errado); a abstenção cai no departamento padrão do agente, então é o erro barato — vale mesmo
  quando o padrão não é o ideal. O relatório separa as quatro coisas e agrupa por configuração de
  departamentos (2, 3, 5, 12).

Uso:
    poph jev eval g1 tests/jev/g1.jsonl --json > relatorio.json
    python3 scripts/jev-relatorio.py relatorio.json [--conjunto tests/jev/g1.jsonl] [--gate g2]

O conjunto é lido para mostrar o texto do cliente em cada caso errado e agrupar por `categoria`.
"""

from __future__ import annotations

import argparse
import json
import sys

GATES = {
    "g1": {
        "positivo": "transferir_agora",
        "padrao": "continuar_agente",
        "nome_positivo": "transferiu",
        "fp": "transferiu indevidamente",
        "fn": "devia transferir, não transferiu",
        "negativo": "não devia transferir",
    },
    "g2": {
        "positivo": "esperar_mais",
        "padrao": "processar_agora",
        "nome_positivo": "esperou",
        "fp": "esperou sem necessidade",
        "fn": "processou cedo (frase partida)",
        "negativo": "devia processar",
    },
}


def carregar_conjunto(caminho: str) -> list[dict]:
    casos = []
    with open(caminho, encoding="utf-8") as arquivo:
        for linha in arquivo:
            linha = linha.strip()
            if not linha or linha.startswith("#"):
                continue
            casos.append(json.loads(linha))
    return casos


def texto_do_cliente(caso: dict) -> str:
    mensagens = (caso.get("estado") or {}).get("mensagens") or []
    for mensagem in reversed(mensagens):
        if mensagem.get("papel") == "cliente":
            return mensagem.get("texto") or ""
    return ""


def main() -> int:
    parser = argparse.ArgumentParser(description="Matriz de confusão do gate G1")
    parser.add_argument("relatorio", help="JSON do `poph jev eval --json`")
    parser.add_argument("--conjunto", default="tests/jev/g1.jsonl", help="conjunto rotulado")
    parser.add_argument(
        "--gate", default="g1", choices=sorted([*GATES, "g4"]), help="gate medido"
    )
    parser.add_argument("--ate", type=int, default=None, help="mede só os N primeiros casos")
    args = parser.parse_args()

    if args.gate == "g4":
        return relatorio_g4(args)

    regra = GATES[args.gate]
    positivo = regra["positivo"]
    bruto = json.load(open(args.relatorio, encoding="utf-8"))
    relatorio = bruto.get("relatorio", bruto)
    detalhes = relatorio["detalhes"]
    try:
        casos = carregar_conjunto(args.conjunto)
    except OSError:
        casos = []

    limite = args.ate or len(detalhes)
    linhas = []
    for indice, detalhe in enumerate(detalhes[:limite], start=1):
        esperado = detalhe["esperado"]
        obtido = detalhe["obtido"]
        # O que vale para o turno: abstenção cai no fallback do gate ("continuar").
        decisao = obtido if obtido is not None else regra["padrao"]
        categoria = detalhe.get("categoria") or _categoria_por_posicao(indice)
        texto = texto_do_cliente(casos[indice - 1]) if indice <= len(casos) else ""
        linhas.append(
            {
                "n": indice,
                "esperado": esperado,
                "obtido": obtido,
                "decisao": decisao,
                "absteve": obtido is None,
                "categoria": categoria,
                "texto": texto,
                "confianca": detalhe.get("confianca"),
                "escolha": detalhe.get("escolha"),
                "motivo": detalhe.get("motivo"),
            }
        )

    def conta(recorte: list[dict]) -> dict:
        deve = [item for item in recorte if item["esperado"] == positivo]
        nao_deve = [item for item in recorte if item["esperado"] != positivo]
        tp = sum(1 for item in deve if item["decisao"] == positivo)
        fn = [item for item in deve if item["decisao"] != positivo]
        fp = [item for item in nao_deve if item["decisao"] == positivo]
        tn = sum(1 for item in nao_deve if item["decisao"] != positivo)
        return {"deve": len(deve), "nao_deve": len(nao_deve), "tp": tp, "fn": fn,
                "fp": fp, "tn": tn,
                "abstencoes": sum(1 for item in recorte if item["absteve"])}

    total = conta(linhas)
    print(
        f"casos: {len(linhas)}  (devia {positivo} {total['deve']} · "
        f"{regra['negativo']} {total['nao_deve']})"
    )
    print(
        f"{regra['nome_positivo']} quando devia: {total['tp']}/{total['deve']}"
        f"  · falso negativo: {len(total['fn'])}"
    )
    print(f"{regra['fp']}: {len(total['fp'])}  · abstenções: {total['abstencoes']}")
    recall = total["tp"] / total["deve"] if total["deve"] else 0.0
    precisao = total["tp"] / (total["tp"] + len(total["fp"])) if total["tp"] + len(total["fp"]) else 1.0
    print(f"recall {recall*100:.1f}% · precisão {precisao*100:.1f}%")

    print("\npor categoria:")
    categorias = sorted({item["categoria"] for item in linhas})
    for categoria in categorias:
        recorte = [item for item in linhas if item["categoria"] == categoria]
        resumo = conta(recorte)
        print(
            f"  {categoria:24} {len(recorte):3} casos · "
            f"deve {resumo['deve']:2} · {regra['nome_positivo']} {resumo['tp']:2} · "
            f"FN {len(resumo['fn']):2} · FP {len(resumo['fp']):2} · abstenção {resumo['abstencoes']:2}"
        )

    if total["fn"]:
        print(f"\nfalsos negativos ({regra['fn']}):")
        for item in total["fn"]:
            print(f"  #{item['n']:3} [{item['categoria']}] {item['texto'][:80]!r}")
            print(
                f"        jev escolheu {item['escolha']!r} confiança {item['confianca']}"
                + (f" · {item['motivo']}" if item["motivo"] else "")
            )
    if total["fp"]:
        print(f"\nfalsos positivos ({regra['fp']}):")
        for item in total["fp"]:
            print(f"  #{item['n']:3} [{item['categoria']}] {item['texto'][:80]!r}")
    return 0


def relatorio_g4(args) -> int:
    """Matriz do G4: escolha certa, escolha errada confiante, abstenção (cai no padrão)."""
    bruto = json.load(open(args.relatorio, encoding="utf-8"))
    relatorio = bruto.get("relatorio", bruto)
    detalhes = relatorio["detalhes"]
    try:
        casos = carregar_conjunto(args.conjunto)
    except OSError:
        casos = []

    linhas = []
    for indice, detalhe in enumerate(detalhes, start=1):
        categoria = detalhe.get("categoria") or "sem_categoria"
        texto = texto_do_cliente(casos[indice - 1]) if indice <= len(casos) else ""
        linhas.append(
            {
                "n": indice,
                "esperado": detalhe["esperado"],
                "obtido": detalhe["obtido"],
                "escolha": detalhe.get("escolha"),
                "absteve": detalhe["obtido"] is None,
                "acertou": detalhe["obtido"] == detalhe["esperado"],
                "categoria": categoria,
                "texto": texto,
                "confianca": detalhe.get("confianca"),
                "motivo": detalhe.get("motivo"),
                "falha": detalhe.get("falha"),
            }
        )

    def resumo(recorte: list[dict]) -> str:
        acertos = [item for item in recorte if item["acertou"]]
        erros = [item for item in recorte if not item["acertou"] and not item["absteve"]]
        abst = [item for item in recorte if item["absteve"]]
        return (
            f"{len(recorte):3} casos · acerto {len(acertos):2} · "
            f"escolha errada {len(erros):2} · abstenção {len(abst):2}"
        )

    total_acertos = sum(1 for item in linhas if item["acertou"])
    total_erros = [item for item in linhas if not item["acertou"] and not item["absteve"]]
    total_abst = [item for item in linhas if item["absteve"]]
    decididos = total_acertos + len(total_erros)
    print(f"casos: {len(linhas)}")
    print(
        f"acerto: {total_acertos}/{len(linhas)} ({total_acertos/len(linhas)*100:.1f}%)"
        f" · escolha errada confiante: {len(total_erros)}"
        f" · abstenção: {len(total_abst)}"
    )
    if decididos:
        print(
            f"precisão entre as decisões: {total_acertos/decididos*100:.1f}%"
            f" (a abstenção cai no departamento padrão, não em erro)"
        )

    print("\npor configuração de departamentos:")
    for categoria in sorted({item["categoria"] for item in linhas}):
        recorte = [item for item in linhas if item["categoria"] == categoria]
        print(f"  {categoria:12} {resumo(recorte)}")

    if total_erros:
        print("\nescolhas erradas confiantes (erro grave: conversa vai para o time errado):")
        for item in total_erros:
            print(
                f"  #{item['n']:3} [{item['categoria']}] esperado {item['esperado']!r} · "
                f"jev {item['escolha']!r} confiança {item['confianca']}"
            )
            print(f"        {item['texto'][:90]!r}")
    if total_abst:
        print("\nabstenções (caem no departamento padrão do agente):")
        for item in total_abst:
            motivo = item["motivo"] or item["falha"] or ""
            print(
                f"  #{item['n']:3} [{item['categoria']}] esperado {item['esperado']!r} · "
                f"jev {item['escolha']!r} confiança {item['confianca']} · {motivo[:70]}"
            )
    return 0


def _categoria_por_posicao(indice: int) -> str:
    """Conjunto antigo (sem categoria): bloco a que o caso pertence pela posição."""
    if indice <= 10:
        return "pedido_explicito*"
    if indice <= 15:
        return "situacao_configurada*"
    if indice <= 23:
        return "insatisfacao_urgencia*"
    if indice <= 27:
        return "risco_saude*"
    if 49 <= indice <= 54:
        return "armadilha_palavra_chave*"
    return "nao_transferir*"


if __name__ == "__main__":
    sys.exit(main())
