
import { afterEach, describe, expect, it } from "vitest";
import { setLocale } from "../i18n";

afterEach(() => {
    setLocale("pt-BR");
});
import {
    FACTORY_DEFAULTS,
    gateEffect,
    agentJev,
    agentModel,
    jevChange,
    modelChanges,
    validateModel,
    type TextModel,
    type JevValues,
} from "./modelJev";
import type { Agente } from "./agents";

const FULL_JEV = {
    ativo: true,
    "despedida": "Até logo, foi um prazer!",
    "situacoes_de_handoff": ["cliente pede segunda via"],
    "resposta_segura": "Vou confirmar isso e já te respondo.",
    "g7_acao": "responder_padrao",
    "g7_resposta": "Não consigo ajudar com isso.",
    "unknown_key": { nested: [1, 2, 3] },
    gates: { g6: true, "gates_key": "fica" },
    g1: true,
};

function valuesOf(jev: Record<string, unknown>): JevValues {
    return agentJev(jev);
}

describe("leitura dos gates (mesma precedência do motor)", () => {
    it("valor aninhado em gates vence o da raiz; ausente = herdar", () => {
        const values = agentJev({ g1: false, gates: { g1: true } });
        expect(values.gates.g1).toBe("ligado");
        expect(values.gates.g2).toBe("herdar");
    });

    it("valor só na raiz conta, e não-booleano é ignorado", () => {
        const values = agentJev({ g4: true, g5: "sim" });
        expect(values.gates.g4).toBe("ligado");
        expect(values.gates.g5).toBe("herdar");
    });

    it("ativo: false vira interruptor desligado; ativo ausente não", () => {
        expect(agentJev({ ativo: false })["jevOff"]).toBe(true);
        expect(agentJev({ ativo: true })["jevOff"]).toBe(false);
        expect(agentJev(undefined)["jevOff"]).toBe(false);
    });

    it("efeito hoje: herdar usa o padrão de fábrica; interruptor vence tudo", () => {
        expect(gateEffect(agentJev({}), "g3")).toBe(FACTORY_DEFAULTS.g3);
        expect(gateEffect(agentJev({}), "g1")).toBe(false);
        expect(gateEffect(agentJev({ ativo: false, gates: { g3: true } }), "g3")).toBe(false);
    });
});

describe("mudancaDoJev: preserva o que a tela não edita", () => {
    it("ligar um gate não toca em nenhuma outra chave", () => {
        const base = valuesOf(FULL_JEV);
        const draft = structuredClone(base);
        draft.gates.g2 = "ligado";

        const texto = jevChange(FULL_JEV, base, draft)!;
        const updated = JSON.parse(texto);

        expect(updated["despedida"]).toBe(FULL_JEV["despedida"]);
        expect(updated["situacoes_de_handoff"]).toEqual(FULL_JEV["situacoes_de_handoff"]);
        expect(updated["resposta_segura"]).toBe(FULL_JEV["resposta_segura"]);
        expect(updated["g7_acao"]).toBe("responder_padrao");
        expect(updated["g7_resposta"]).toBe("Não consigo ajudar com isso.");
        expect(updated["unknown_key"]).toEqual({ nested: [1, 2, 3] });
        expect(updated.gates).toEqual({ g6: true, "gates_key": "fica", g2: true });
        expect(updated.ativo).toBe(true);
    });

    it("gate que muda sai da raiz, para não voltar a valer por trás da tela", () => {
        const base = valuesOf(FULL_JEV);
        const draft = structuredClone(base);
        draft.gates.g1 = "herdar";

        const updated = JSON.parse(jevChange(FULL_JEV, base, draft)!);
        expect(updated).not.toHaveProperty("g1");
        expect(updated.gates).toEqual({ g6: true, "gates_key": "fica" });
    });

    it("gates vazios depois da mudança saem do jev_json, sem apagar outras chaves", () => {
        const jev = { "despedida": "Tchau", gates: { g6: true } };
        const base = valuesOf(jev);
        const draft = structuredClone(base);
        draft.gates.g6 = "herdar";

        const updated = JSON.parse(jevChange(jev, base, draft)!);
        expect(updated).toEqual({ "despedida": "Tchau" });
    });

    it("interruptor ativo: false ligado e desligado, sem perder o resto", () => {
        const base = valuesOf({ "despedida": "Tchau" });
        const enabled = structuredClone(base);
        enabled["jevOff"] = true;
        expect(JSON.parse(jevChange({ "despedida": "Tchau" }, base, enabled)!)).toEqual({
            "despedida": "Tchau",
            ativo: false,
        });

        const jevOffCopy = structuredClone(enabled);
        jevOffCopy["jevOff"] = false;
        expect(
            JSON.parse(jevChange({ "despedida": "Tchau", ativo: false }, enabled, jevOffCopy)!),
        ).toEqual({ "despedida": "Tchau" });
    });

    it("sem mudança de gate nem de interruptor, não há jev_json para mandar", () => {
        const base = valuesOf(FULL_JEV);
        expect(jevChange(FULL_JEV, base, structuredClone(base))).toBeUndefined();
    });

    it("usa o jev atual (relido), não o que a tela tinha: outra mudança de quem salvou antes fica", () => {
        const base = valuesOf({ gates: { g6: true } });
        const draft = structuredClone(base);
        draft.gates.g2 = "ligado";
        const current = { gates: { g6: true }, "despedida": "Nova despedida", g7: true };

        const updated = JSON.parse(jevChange(current, base, draft)!);
        expect(updated["despedida"]).toBe("Nova despedida");
        expect(updated.g7).toBe(true);
        expect(updated.gates).toEqual({ g6: true, g2: true });
    });

    it("não quebra quando gates no jev atual não é objeto", () => {
        const jev = { gates: "texto estranho" } as unknown as Record<string, unknown>;
        const base = valuesOf({});
        const draft = structuredClone(base);
        draft.gates.g3 = "desligado";
        const updated = JSON.parse(jevChange(jev, base, draft)!);
        expect(updated.gates).toEqual({ g3: false });
    });
});

describe("modelo e RAG: validação e corpo do PATCH", () => {
    const base: TextModel = {
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
        expect(validateModel(base, base)).toEqual({});
    });

    it("janela acima de 8000 ms e top-k fora de 1–50 são recusados na tela", () => {
        const errors = validateModel({ ...base, grouping_window_ms: "9000", rag_top_k: "51" }, base);
        expect(errors.grouping_window_ms).toContain("8000");
        expect(errors.rag_top_k).toContain("50");
    });

    it("faixas do banco: contexto mínimo 1000; rodadas 0–20; ferramentas 0–100", () => {
        expect(validateModel({ ...base, max_context_tokens: "999" }, base).max_context_tokens).toContain("1000");
        expect(validateModel({ ...base, max_context_tokens: "1000" }, base)).toEqual({});
        expect(validateModel({ ...base, max_tool_rounds: "0" }, base)).toEqual({});
        expect(validateModel({ ...base, max_tool_rounds: "21" }, base).max_tool_rounds).toContain("20");
        expect(validateModel({ ...base, max_tool_calls: "101" }, base).max_tool_calls).toContain("100");
    });

    it("corte fora de 0–1 e temperatura fora de 0–2 são recusados; vírgula decimal vale", () => {
        expect(validateModel({ ...base, rag_min_score: "1,5" }, base).rag_min_score).toBeDefined();
        expect(validateModel({ ...base, temperature: "2,1" }, base).temperature).toBeDefined();
        expect(validateModel({ ...base, temperature: "0,7" }, base)).toEqual({});
    });

    it("temperatura ou máximo de saída já definidos não podem voltar a vazio pela tela", () => {
        const withValue = { ...base, temperature: "0.7", max_output_tokens: "2000" };
        const removed = { ...withValue, temperature: "", max_output_tokens: "" };
        const errors = validateModel(removed, withValue);
        expect(errors.temperature).toContain("não volta");
        expect(errors.max_output_tokens).toContain("não volta");
    });

    it("gravação manda só os campos que mudaram, com números convertidos", () => {
        const texto = { ...base, rag_top_k: "5", rag_min_score: "0,35" };
        expect(modelChanges(base, texto)).toEqual({ rag_top_k: 5, rag_min_score: 0.35 });
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
        const modelo = agentModel(agente);
        expect(modelo.temperature).toBe("");
        expect(modelo.max_tool_calls).toBe("10");
    });

    it("emite validações em inglês quando o idioma ativo é en-US", () => {
        setLocale("en-US");
        const errors = validateModel({ ...base, provider: "", grouping_window_ms: "9000" }, base);
        expect(errors.provider).toBe("enter the provider");
        expect(errors.grouping_window_ms).toBe("use a number from 0 to 8000 ms");
        setLocale("pt-BR");
    });
});
