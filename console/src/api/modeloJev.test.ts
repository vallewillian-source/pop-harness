/**
 * Regras puras da aba Modelo e Jev (HR-45): leitura dos gates com a mesma precedência do motor,
 * montagem do `jev_json` que preserva as chaves que a tela não edita, e as validações de faixa.
 *
 * A prova de que nenhuma chave some está aqui e também na tela (`AgenteModeloJev.test.tsx`).
 */

import { describe, expect, it } from "vitest";
import {
    PADRAO_DE_FABRICA,
    efeitoDoGate,
    jevDoAgente,
    modeloDoAgente,
    mudancaDoJev,
    mudancasDoModelo,
    validarModelo,
    type ModeloTexto,
    type ValoresJev,
} from "./modeloJev";
import type { Agente } from "./agentes";

const JEV_COMPLETO = {
    ativo: true,
    despedida: "Até logo, foi um prazer!",
    situacoes_de_handoff: ["cliente pede segunda via"],
    resposta_segura: "Vou confirmar isso e já te respondo.",
    g7_acao: "responder_padrao",
    g7_resposta: "Não consigo ajudar com isso.",
    chave_desconhecida: { nested: [1, 2, 3] },
    gates: { g6: true, chave_do_gates: "fica" },
    g1: true,
};

function valoresDe(jev: Record<string, unknown>): ValoresJev {
    return jevDoAgente(jev);
}

describe("leitura dos gates (mesma precedência do motor)", () => {
    it("valor aninhado em gates vence o da raiz; ausente = herdar", () => {
        const valores = jevDoAgente({ g1: false, gates: { g1: true } });
        expect(valores.gates.g1).toBe("ligado");
        expect(valores.gates.g2).toBe("herdar");
    });

    it("valor só na raiz conta, e não-booleano é ignorado", () => {
        const valores = jevDoAgente({ g4: true, g5: "sim" });
        expect(valores.gates.g4).toBe("ligado");
        expect(valores.gates.g5).toBe("herdar");
    });

    it("ativo: false vira interruptor desligado; ativo ausente não", () => {
        expect(jevDoAgente({ ativo: false }).jevDesligado).toBe(true);
        expect(jevDoAgente({ ativo: true }).jevDesligado).toBe(false);
        expect(jevDoAgente(undefined).jevDesligado).toBe(false);
    });

    it("efeito hoje: herdar usa o padrão de fábrica; interruptor vence tudo", () => {
        expect(efeitoDoGate(jevDoAgente({}), "g3")).toBe(PADRAO_DE_FABRICA.g3);
        expect(efeitoDoGate(jevDoAgente({}), "g1")).toBe(false);
        expect(efeitoDoGate(jevDoAgente({ ativo: false, gates: { g3: true } }), "g3")).toBe(false);
    });
});

describe("mudancaDoJev: preserva o que a tela não edita", () => {
    it("ligar um gate não toca em nenhuma outra chave", () => {
        const base = valoresDe(JEV_COMPLETO);
        const rascunho = structuredClone(base);
        rascunho.gates.g2 = "ligado";

        const texto = mudancaDoJev(JEV_COMPLETO, base, rascunho)!;
        const novo = JSON.parse(texto);

        expect(novo.despedida).toBe(JEV_COMPLETO.despedida);
        expect(novo.situacoes_de_handoff).toEqual(JEV_COMPLETO.situacoes_de_handoff);
        expect(novo.resposta_segura).toBe(JEV_COMPLETO.resposta_segura);
        expect(novo.g7_acao).toBe("responder_padrao");
        expect(novo.g7_resposta).toBe("Não consigo ajudar com isso.");
        expect(novo.chave_desconhecida).toEqual({ nested: [1, 2, 3] });
        expect(novo.gates).toEqual({ g6: true, chave_do_gates: "fica", g2: true });
        expect(novo.ativo).toBe(true);
    });

    it("gate que muda sai da raiz, para não voltar a valer por trás da tela", () => {
        const base = valoresDe(JEV_COMPLETO);
        const rascunho = structuredClone(base);
        rascunho.gates.g1 = "herdar";

        const novo = JSON.parse(mudancaDoJev(JEV_COMPLETO, base, rascunho)!);
        expect(novo).not.toHaveProperty("g1");
        expect(novo.gates).toEqual({ g6: true, chave_do_gates: "fica" });
    });

    it("gates vazios depois da mudança saem do jev_json, sem apagar outras chaves", () => {
        const jev = { despedida: "Tchau", gates: { g6: true } };
        const base = valoresDe(jev);
        const rascunho = structuredClone(base);
        rascunho.gates.g6 = "herdar";

        const novo = JSON.parse(mudancaDoJev(jev, base, rascunho)!);
        expect(novo).toEqual({ despedida: "Tchau" });
    });

    it("interruptor ativo: false ligado e desligado, sem perder o resto", () => {
        const base = valoresDe({ despedida: "Tchau" });
        const ligado = structuredClone(base);
        ligado.jevDesligado = true;
        expect(JSON.parse(mudancaDoJev({ despedida: "Tchau" }, base, ligado)!)).toEqual({
            despedida: "Tchau",
            ativo: false,
        });

        const desligadoDeVolta = structuredClone(ligado);
        desligadoDeVolta.jevDesligado = false;
        expect(
            JSON.parse(mudancaDoJev({ despedida: "Tchau", ativo: false }, ligado, desligadoDeVolta)!),
        ).toEqual({ despedida: "Tchau" });
    });

    it("sem mudança de gate nem de interruptor, não há jev_json para mandar", () => {
        const base = valoresDe(JEV_COMPLETO);
        expect(mudancaDoJev(JEV_COMPLETO, base, structuredClone(base))).toBeUndefined();
    });

    it("usa o jev atual (relido), não o que a tela tinha: outra mudança de quem salvou antes fica", () => {
        const base = valoresDe({ gates: { g6: true } });
        const rascunho = structuredClone(base);
        rascunho.gates.g2 = "ligado";
        // Outra pessoa, entre a leitura e o salvamento, trocou a despedida e ligou o G7.
        const atual = { gates: { g6: true }, despedida: "Nova despedida", g7: true };

        const novo = JSON.parse(mudancaDoJev(atual, base, rascunho)!);
        expect(novo.despedida).toBe("Nova despedida");
        expect(novo.g7).toBe(true);
        expect(novo.gates).toEqual({ g6: true, g2: true });
    });

    it("não quebra quando gates no jev atual não é objeto", () => {
        const jev = { gates: "texto estranho" } as unknown as Record<string, unknown>;
        const base = valoresDe({});
        const rascunho = structuredClone(base);
        rascunho.gates.g3 = "desligado";
        const novo = JSON.parse(mudancaDoJev(jev, base, rascunho)!);
        expect(novo.gates).toEqual({ g3: false });
    });
});

describe("modelo e RAG: validação e corpo do PATCH", () => {
    const base: ModeloTexto = {
        provider: "antigravity2api",
        model: "gemini-3.8-flash-tiered",
        temperature: "",
        max_output_tokens: "",
        max_context_tokens: "64000",
        grouping_window_ms: "2000",
        max_tool_rounds: "4",
        max_tool_calls: "10",
        rag_top_k: "3",
        rag_min_score: "0",
    };

    it("valores válidos passam sem erro", () => {
        expect(validarModelo(base, base)).toEqual({});
    });

    it("janela acima de 8000 ms e top-k fora de 1–50 são recusados na tela", () => {
        const erros = validarModelo({ ...base, grouping_window_ms: "9000", rag_top_k: "51" }, base);
        expect(erros.grouping_window_ms).toContain("8000");
        expect(erros.rag_top_k).toContain("50");
    });

    it("faixas do banco: contexto mínimo 1000; rodadas 0–20; ferramentas 0–100", () => {
        expect(validarModelo({ ...base, max_context_tokens: "999" }, base).max_context_tokens).toContain("1000");
        expect(validarModelo({ ...base, max_context_tokens: "1000" }, base)).toEqual({});
        expect(validarModelo({ ...base, max_tool_rounds: "0" }, base)).toEqual({});
        expect(validarModelo({ ...base, max_tool_rounds: "21" }, base).max_tool_rounds).toContain("20");
        expect(validarModelo({ ...base, max_tool_calls: "101" }, base).max_tool_calls).toContain("100");
    });

    it("corte fora de 0–1 e temperatura fora de 0–2 são recusados; vírgula decimal vale", () => {
        expect(validarModelo({ ...base, rag_min_score: "1,5" }, base).rag_min_score).toBeDefined();
        expect(validarModelo({ ...base, temperature: "2,1" }, base).temperature).toBeDefined();
        expect(validarModelo({ ...base, temperature: "0,7" }, base)).toEqual({});
    });

    it("temperatura ou máximo de saída já definidos não podem voltar a vazio pela tela", () => {
        const comValor = { ...base, temperature: "0.7", max_output_tokens: "2000" };
        const apagados = { ...comValor, temperature: "", max_output_tokens: "" };
        const erros = validarModelo(apagados, comValor);
        expect(erros.temperature).toContain("não volta");
        expect(erros.max_output_tokens).toContain("não volta");
    });

    it("gravação manda só os campos que mudaram, com números convertidos", () => {
        const texto = { ...base, rag_top_k: "5", rag_min_score: "0,35" };
        expect(mudancasDoModelo(base, texto)).toEqual({ rag_top_k: 5, rag_min_score: 0.35 });
    });

    it("modeloDoAgente mostra campos nulos como vazio (herda o padrão)", () => {
        const agente = {
            provider: "antigravity2api",
            model: "m",
            temperature: null,
            max_output_tokens: null,
            max_context_tokens: 64000,
            grouping_window_ms: 2000,
            max_tool_rounds: 4,
            max_tool_calls_per_message: 10,
            rag_top_k: 3,
            rag_min_score: 0,
        } as unknown as Agente;
        const modelo = modeloDoAgente(agente);
        expect(modelo.temperature).toBe("");
        expect(modelo.max_tool_calls).toBe("10");
    });
});
