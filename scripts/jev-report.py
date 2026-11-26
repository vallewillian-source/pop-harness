#!/usr/bin/env python3
"""Reads the `--json` output of `poph jev eval` and prints the gate's confusion matrix.

`poph jev eval` counts hit/abstention/error. To decide whether a gate can be turned on, what matters is
something else: the error the client feels. Each gate has its own:

- **G1** (`--gate g1`, default): a false negative is the transfer that should have happened and did not
  (abstention falls on "continue", so it counts as a false negative); a false positive is an improper
  transfer.
- **G2** (`--gate g2`): a false negative is **processing too early** (the message was incomplete and the
  sentence came out cut off); a false positive is **waiting without need** (extra latency). Abstention falls
  on "process now" in the fixed window, so it counts as a false negative when the label was wait.
- **G4** (`--gate g4`): the serious error is a **confident wrong choice** (the conversation goes to the wrong
  team); abstention falls on the agent's default department, so it is the cheap error — even when the default
  is not the ideal one. The report separates the four things and groups by department configuration (2, 3, 5, 12).

Usage:
    poph jev eval g1 tests/jev/g1.jsonl --json > relatorio.json
    python3 scripts/jev-report.py relatorio.json [--conjunto tests/jev/g1.jsonl] [--gate g2]

The set is read to show the client's text for each wrong case and to group by `categoria`.
"""

from __future__ import annotations

import argparse
import json
import sys

GATES = {
    "g1": {
        "positive": "transferir_agora",
        "default": "continuar_agente",
        "positive_name": "transferiu",
        "fp": "transferiu indevidamente",
        "fn": "devia transferir, não transferiu",
        "negative": "não devia transferir",
    },
    "g2": {
        "positive": "esperar_mais",
        "default": "processar_agora",
        "positive_name": "esperou",
        "fp": "esperou sem necessidade",
        "fn": "processou cedo (frase partida)",
        "negative": "devia processar",
    },
}


def load_set(path: str) -> list[dict]:
    cases = []
    with open(path, encoding="utf-8") as file:
        for line in file:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            cases.append(json.loads(line))
    return cases


def client_text(case: dict) -> str:
    messages = (case.get("estado") or {}).get("mensagens") or []
    for message in reversed(messages):
        if message.get("papel") == "cliente":
            return message.get("texto") or ""
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
        return report_g4(args)

    rule = GATES[args.gate]
    positive = rule["positive"]
    raw = json.load(open(args.relatorio, encoding="utf-8"))
    report = raw.get("relatorio", raw)
    details = report["detalhes"]
    try:
        cases = load_set(args.conjunto)
    except OSError:
        cases = []

    limit = args.ate or len(details)
    rows = []
    for index, detail in enumerate(details[:limit], start=1):
        expected = detail["esperado"]
        obtained = detail["obtido"]
        decision = obtained if obtained is not None else rule["default"]
        category = detail.get("categoria") or _category_by_position(index)
        text = client_text(cases[index - 1]) if index <= len(cases) else ""
        rows.append(
            {
                "n": index,
                "expected": expected,
                "obtained": obtained,
                "decision": decision,
                "abstained": obtained is None,
                "category": category,
                "text": text,
                "confidence": detail.get("confianca"),
                "choice": detail.get("escolha"),
                "reason": detail.get("motivo"),
            }
        )

    def count(slice_: list[dict]) -> dict:
        should = [item for item in slice_ if item["expected"] == positive]
        should_not = [item for item in slice_ if item["expected"] != positive]
        tp = sum(1 for item in should if item["decision"] == positive)
        fn = [item for item in should if item["decision"] != positive]
        fp = [item for item in should_not if item["decision"] == positive]
        tn = sum(1 for item in should_not if item["decision"] != positive)
        return {"should": len(should), "should_not": len(should_not), "tp": tp, "fn": fn,
                "fp": fp, "tn": tn,
                "abstentions": sum(1 for item in slice_ if item["abstained"])}

    total = count(rows)
    print(
        f"casos: {len(rows)}  (devia {positive} {total['should']} · "
        f"{rule['negative']} {total['should_not']})"
    )
    print(
        f"{rule['positive_name']} quando devia: {total['tp']}/{total['should']}"
        f"  · falso negativo: {len(total['fn'])}"
    )
    print(f"{rule['fp']}: {len(total['fp'])}  · abstenções: {total['abstentions']}")
    recall = total["tp"] / total["should"] if total["should"] else 0.0
    precision = total["tp"] / (total["tp"] + len(total["fp"])) if total["tp"] + len(total["fp"]) else 1.0
    print(f"recall {recall*100:.1f}% · precisão {precision*100:.1f}%")

    print("\npor categoria:")
    categories = sorted({item["category"] for item in rows})
    for category in categories:
        slice_ = [item for item in rows if item["category"] == category]
        summary = count(slice_)
        print(
            f"  {category:24} {len(slice_):3} casos · "
            f"deve {summary['should']:2} · {rule['positive_name']} {summary['tp']:2} · "
            f"FN {len(summary['fn']):2} · FP {len(summary['fp']):2} · abstenção {summary['abstentions']:2}"
        )

    if total["fn"]:
        print(f"\nfalsos negativos ({rule['fn']}):")
        for item in total["fn"]:
            print(f"  #{item['n']:3} [{item['category']}] {item['text'][:80]!r}")
            print(
                f"        jev escolheu {item['choice']!r} confiança {item['confidence']}"
                + (f" · {item['reason']}" if item["reason"] else "")
            )
    if total["fp"]:
        print(f"\nfalsos positivos ({rule['fp']}):")
        for item in total["fp"]:
            print(f"  #{item['n']:3} [{item['category']}] {item['text'][:80]!r}")
    return 0


def report_g4(args) -> int:
    """G4 matrix: right choice, confident wrong choice, abstention (falls on the default)."""
    raw = json.load(open(args.relatorio, encoding="utf-8"))
    report = raw.get("relatorio", raw)
    details = report["detalhes"]
    try:
        cases = load_set(args.conjunto)
    except OSError:
        cases = []

    rows = []
    for index, detail in enumerate(details, start=1):
        category = detail.get("categoria") or "sem_categoria"
        text = client_text(cases[index - 1]) if index <= len(cases) else ""
        rows.append(
            {
                "n": index,
                "expected": detail["esperado"],
                "obtained": detail["obtido"],
                "choice": detail.get("escolha"),
                "abstained": detail["obtido"] is None,
                "hit": detail["obtido"] == detail["esperado"],
                "category": category,
                "text": text,
                "confidence": detail.get("confianca"),
                "reason": detail.get("motivo"),
                "failure": detail.get("falha"),
            }
        )

    def summary(slice_: list[dict]) -> str:
        hits = [item for item in slice_ if item["hit"]]
        errors = [item for item in slice_ if not item["hit"] and not item["abstained"]]
        abstained = [item for item in slice_ if item["abstained"]]
        return (
            f"{len(slice_):3} casos · acerto {len(hits):2} · "
            f"escolha errada {len(errors):2} · abstenção {len(abstained):2}"
        )

    total_hits = sum(1 for item in rows if item["hit"])
    total_errors = [item for item in rows if not item["hit"] and not item["abstained"]]
    total_abstained = [item for item in rows if item["abstained"]]
    decided = total_hits + len(total_errors)
    print(f"casos: {len(rows)}")
    print(
        f"acerto: {total_hits}/{len(rows)} ({total_hits/len(rows)*100:.1f}%)"
        f" · escolha errada confiante: {len(total_errors)}"
        f" · abstenção: {len(total_abstained)}"
    )
    if decided:
        print(
            f"precisão entre as decisões: {total_hits/decided*100:.1f}%"
            f" (a abstenção cai no departamento padrão, não em erro)"
        )

    print("\npor configuração de departamentos:")
    for category in sorted({item["category"] for item in rows}):
        slice_ = [item for item in rows if item["category"] == category]
        print(f"  {category:12} {summary(slice_)}")

    if total_errors:
        print("\nescolhas erradas confiantes (erro grave: conversa vai para o time errado):")
        for item in total_errors:
            print(
                f"  #{item['n']:3} [{item['category']}] esperado {item['expected']!r} · "
                f"jev {item['choice']!r} confiança {item['confidence']}"
            )
            print(f"        {item['text'][:90]!r}")
    if total_abstained:
        print("\nabstenções (caem no departamento padrão do agente):")
        for item in total_abstained:
            reason = item["reason"] or item["failure"] or ""
            print(
                f"  #{item['n']:3} [{item['category']}] esperado {item['expected']!r} · "
                f"jev {item['choice']!r} confiança {item['confidence']} · {reason[:70]}"
            )
    return 0


def _category_by_position(index: int) -> str:
    """Old set (no category): the block the case belongs to by its position."""
    if index <= 10:
        return "pedido_explicito*"
    if index <= 15:
        return "situacao_configurada*"
    if index <= 23:
        return "insatisfacao_urgencia*"
    if index <= 27:
        return "risco_saude*"
    if 49 <= index <= 54:
        return "armadilha_palavra_chave*"
    return "nao_transferir*"


if __name__ == "__main__":
    sys.exit(main())
