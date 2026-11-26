
import { afterEach, describe, expect, it } from "vitest";
import { setLocale } from "../i18n";
import {
    countTools,
    turnDuration,
    formatConfidence,
    formatToolContent,
    formatCostMicros,
    formatMargin,
    formatToolDuration,
    formatMs,
    formatTokens,
    phraseForStateWithoutResponse,
    interleaveTurns,
    readToolCalls,
    readTurnTools,
    readTurnJev,
    readTurnRetrieved,
    readTurnTimings,
    originLabel,
    turnStateLabel,
    endReasonLabel,
    refusalReasonLabel,
    gateTitle,
} from "./turnWhy";

const JEV_REAL = {
    "chamada": {
        "perguntas": 2,
        "decididas_por_cache": 1,
        ms: 310,
        tokens: 480,
        usage: { "entrada": 400, saida: 80, total: 480, "informado": true },
        "falha": null,
    },
    "decisoes": [
        {
            gate: "g3",
            "escolha": "base",
            "origem": "jev",
            "confianca": 0.92,
            "margem": 0.61,
            motivo: null,
            ms: 310,
            tokens: 240,
            "escolha_do_jev": "base",
        },
        {
            gate: "g1",
            "escolha": "continuar_agente",
            "origem": "fallback",
            "confianca": 0,
            "margem": 0,
            motivo: "o Jev não respondeu a tempo",
            ms: 0,
            tokens: 0,
        },
    ],
};

describe("lerJevDoTurno", () => {
    it("lê a chamada, as decisões e a escolha do Jev, sem inventar o que não veio", () => {
        const jev = readTurnJev(JEV_REAL);

        expect(jev?.["chamada"]).toEqual({
            "perguntas": 2,
            "decidedByCache": 1,
            ms: 310,
            tokens: 480,
            "entrada": 400,
            saida: 80,
            total: 480,
            "falha": null,
        });
        expect(jev?.["decisoes"]).toHaveLength(2);
        expect(jev?.["decisoes"][0]).toEqual({
            gate: "g3",
            "escolha": "base",
            "jevChoice": "base",
            "origem": "jev",
            "confianca": 0.92,
            "margem": 0.61,
            motivo: null,
            ms: 310,
            tokens: 240,
        });
        expect(jev?.["decisoes"][1]?.["jevChoice"]).toBeNull();
        expect(jev?.["decisoes"][1]?.motivo).toBe("o Jev não respondeu a tempo");
        expect(jev?.["revisao"]).toBeNull();
    });

    it("sem registro de Jev, devolve null (nenhum gate decidiu)", () => {
        expect(readTurnJev(null)).toBeNull();
        expect(readTurnJev(undefined)).toBeNull();
        expect(readTurnJev({})).toBeNull();
        expect(readTurnJev({ "decisoes": [] })).toBeNull();
    });

    it("lê a revisão do G6 quando ela existe", () => {
        const jev = readTurnJev({
            ...JEV_REAL,
            "revisao": {
                gate: "g6",
                "escolha": "revisar_tom",
                "confianca": 0.81,
                regenerou: true,
                "segunda_escolha": "aprovada",
                "resposta_segura": false,
            },
        });

        expect(jev?.["revisao"]).toEqual({
            "escolha": "revisar_tom",
            "confianca": 0.81,
            regenerou: true,
            "secondChoice": "aprovada",
            "responseSafe": false,
        });
    });

    it("campo com tipo errado vira não registrado, e a falha da chamada aparece", () => {
        const jev = readTurnJev({
            "chamada": { "perguntas": "dois", ms: null, "falha": "prazo de 8 s estourado", usage: "x" },
            "decisoes": [{ gate: "g5", "confianca": "alta", "escolha": 7 }, "lixo", null],
        });

        expect(jev?.["chamada"]?.["perguntas"]).toBeNull();
        expect(jev?.["chamada"]?.ms).toBeNull();
        expect(jev?.["chamada"]?.["entrada"]).toBeNull();
        expect(jev?.["chamada"]?.["falha"]).toBe("prazo de 8 s estourado");
        expect(jev?.["decisoes"]).toHaveLength(1);
        expect(jev?.["decisoes"][0]?.["confianca"]).toBeNull();
        expect(jev?.["decisoes"][0]?.["escolha"]).toBeNull();
    });
});

describe("lerRecuperadosDoTurno", () => {
    it("separa trechos e produtos pelo tipo e mantém a fonte do trecho", () => {
        const retrieved = readTurnRetrieved([
            {
                tipo: "trecho",
                item: "item-1",
                "titulo": "Prazo de entrega",
                "grupo": "Logística",
                texto: "Entregamos em 5 dias úteis.",
                "similaridade": 0.83,
                "relevancia": 0.0163,
            },
            {
                tipo: "produto",
                "produto": "prod-1",
                nome: "Dedetização",
                "preco_centavos": 45000,
                "preco_texto": "R$ 450,00 por visita",
                "unidade": "por visita",
                "condicoes": "Até 50 m²",
            },
        ]);

        expect(retrieved["trechos"]).toEqual([
            {
                item: "item-1",
                "titulo": "Prazo de entrega",
                "grupo": "Logística",
                texto: "Entregamos em 5 dias úteis.",
                "similaridade": 0.83,
                "relevancia": 0.0163,
            },
        ]);
        expect(retrieved["produtos"]).toEqual([
            {
                "produto": "prod-1",
                nome: "Dedetização",
                "precoTexto": "R$ 450,00 por visita",
                "priceCents": 45000,
                "unidade": "por visita",
                "condicoes": "Até 50 m²",
            },
        ]);
    });

    it("sem uso nenhum, ou com formato inesperado, devolve listas vazias", () => {
        expect(readTurnRetrieved([])).toEqual({ "trechos": [], "produtos": [] });
        expect(readTurnRetrieved(null)).toEqual({ "trechos": [], "produtos": [] });
        expect(readTurnRetrieved({ "trechos": [] })).toEqual({ "trechos": [], "produtos": [] });
        expect(readTurnRetrieved(["texto solto", 3])).toEqual({ "trechos": [], "produtos": [] });
    });
});

describe("ferramentas e tempos", () => {
    it("ferramentas vêm só como nomes; a contagem respeita a ordem da primeira chamada", () => {
        const names = readTurnTools(["consultar_base", "buscar_produto", "consultar_base"]);
        expect(names).toEqual(["consultar_base", "buscar_produto", "consultar_base"]);
        expect(countTools(names)).toEqual([
            { nome: "consultar_base", "times": 2 },
            { nome: "buscar_produto", "times": 1 },
        ]);
        expect(readTurnTools(null)).toEqual([]);
    });

    it("lê os tempos por etapa e o motivo do fim", () => {
        expect(
            readTurnTimings({
                "fila_ms": 3,
                "montagem_ms": 40,
                "primeiro_texto_ms": 480,
                total_ms: 920,
                "rodadas_ferramenta": 1,
                modelo: "gemini-3.8-flash-tiered",
                motivo_fim: "concluido",
            }),
        ).toEqual({
            "queueMs": 3,
            "buildMs": 40,
            "firstTextMs": 480,
            totalMs: 920,
            "roundsTool": 1,
            modelo: "gemini-3.8-flash-tiered",
            "reasonEnd": "concluido",
        });
        expect(readTurnTimings(null)).toBeNull();
    });

    it("duração: o total medido pelo turno; sem ele, início e fim; em andamento, nada", () => {
        expect(turnDuration(1000, 1500, { "queueMs": null, "buildMs": null, "firstTextMs": null, totalMs: 920, "roundsTool": null, modelo: null, "reasonEnd": null })).toBe(920);
        expect(turnDuration(1000, 1500, null)).toBe(500);
        expect(turnDuration(1000, null, null)).toBeNull();
    });
});

describe("rótulos e formatação", () => {
    it("traduz origem e motivo do fim, e não inventa quando faltam", () => {
        expect(originLabel("cache")).toBe("Reaproveitada do cache (sem chamada ao Jev)");
        expect(originLabel("fallback")).toBe("Valeu o padrão do gate; a resposta do Jev não foi usada");
        expect(originLabel("algo_novo")).toBe("não registrado");
        expect(endReasonLabel("limite_de_tokens")).toBe("Parou no limite de tokens de saída");
        expect(endReasonLabel("motivo_que_nao_conheço")).toBe("motivo_que_nao_conheço");
        expect(endReasonLabel(null)).toBe("não registrado");
    });

    it("gate usa o título da aba Modelo e Jev; id desconhecido aparece cru", () => {
        expect(gateTitle("g1")).toBe("G1 · Transferir para humano agora?");
        expect(gateTitle("g9")).toBe("G9");
    });

    it("custo sem valor diz que não foi gravado; com valor, diz a unidade sem converter", () => {
        expect(formatCostMicros(null)).toBe("não calculado neste turno (o turno ainda não grava o custo)");
        expect(formatCostMicros(0)).toBe("não calculado neste turno (o turno ainda não grava o custo)");
        expect(formatCostMicros(1820)).toBe("1.820 micros de moeda (a API não informa a moeda)");
    });

    it("milissegundos em pt-BR e frase para turno sem resposta", () => {
        expect(formatMs(null)).toBe("não registrado");
        expect(formatMs(1234)).toBe("1.234 ms");
        expect(phraseForStateWithoutResponse("failed")).toBe("O turno falhou antes de a IA enviar uma resposta.");
        expect(phraseForStateWithoutResponse("running")).toBe("O turno ainda está em andamento.");
    });
});

describe("intercalarTurnos", () => {
    it("põe o turno sem resposta entre as mensagens em que ele aconteceu", () => {
        const messages = [
            { id: "m1", created_at: 100 },
            { id: "m2", created_at: 400 },
            { id: "m3", created_at: 900 },
        ];
        const turns = [
            { id: "t1", started_at: 150 },
            { id: "t2", started_at: 500 },
        ];

        const items = interleaveTurns(messages, turns).map((item) =>
            item.tipo === "mensagem" ? item.mensagem.id : item.turno.id,
        );

        expect(items).toEqual(["m1", "t1", "m2", "t2", "m3"]);
    });

    it("turno que começou depois da última mensagem vai para o fim; sem mensagens, só turnos", () => {
        expect(
            interleaveTurns([{ id: "m1", created_at: 100 }], [{ id: "t9", started_at: 999 }]).map((item) =>
                item.tipo === "mensagem" ? item.mensagem.id : item.turno.id,
            ),
        ).toEqual(["m1", "t9"]);
        expect(interleaveTurns([], [{ id: "t1", started_at: 1 }]).map((item) => item.tipo)).toEqual(["turno"]);
    });
});

describe("chamadas de ferramenta (HR-47c)", () => {
    it("lê as chamadas na ordem de rodada e de chamada, com estado e motivo", () => {
        const readCalls = readToolCalls([
            { nome: "cart_add_item", "rodada": 1, ordem: 3, estado: "recusada", motivo: "nao_habilitada", "duracao_ms": 0, "argumentos": "{}", resultado: "erro" },
            { nome: "enviar_arquivo", "rodada": 0, ordem: 2, estado: "executada", motivo: null, "duracao_ms": 7, "argumentos": "{}", resultado: "ok" },
            { nome: "buscar_produto", "rodada": 0, ordem: 1, estado: "executada", motivo: null, "duracao_ms": 3, "argumentos": "{}", resultado: "ok" },
        ]);
        expect(readCalls.map((c) => c.nome)).toEqual(["buscar_produto", "enviar_arquivo", "cart_add_item"]);
        expect(readCalls[2]).toMatchObject({ estado: "recusada", motivo: "nao_habilitada", "durationMs": 0 });
        expect(readCalls[0]).toMatchObject({ estado: "executada", motivo: null, "durationMs": 3 });
    });

    it("turno antigo, ausente, nulo ou com outro formato vira lista vazia, sem erro", () => {
        expect(readToolCalls(undefined)).toEqual([]);
        expect(readToolCalls(null)).toEqual([]);
        expect(readToolCalls({ "nomes": ["x"] })).toEqual([]);
        expect(readToolCalls([null, 3, "texto", { "no_name": true }])).toEqual([]);
    });

    it("estado desconhecido não vira executada nem recusada; motivo só existe quando recusada", () => {
        const readCalls = readToolCalls([
            { nome: "x", estado: "talvez", motivo: "nao_habilitada", "argumentos": "a", resultado: "b" },
        ]);
        expect(readCalls).toEqual([
            expect.objectContaining({
                estado: "nao_registrado",
                motivo: null,
                "rodada": null,
                "durationMs": null,
            }),
        ]);

        const executed = readToolCalls([{ nome: "y", estado: "executada", motivo: "nao_habilitada" }]);
        expect(executed).toEqual([expect.objectContaining({ estado: "executada", motivo: null })]);
    });

    it("campos ilegíveis viram null, nunca número ou texto inventado", () => {
        const readCalls = readToolCalls([
            { nome: "z", estado: "executada", "rodada": "1", "duracao_ms": "9", "argumentos": "  ", resultado: 4 },
        ]);
        expect(readCalls).toEqual([
            expect.objectContaining({ "rodada": null, "durationMs": null, "argumentos": null, resultado: null }),
        ]);
    });

    it("cada motivo de recusa tem texto em português, e motivo desconhecido aparece como veio", () => {
        expect(refusalReasonLabel("nao_habilitada")).toBe("ferramenta não habilitada neste agente");
        expect(refusalReasonLabel("desconhecida")).toBe("ferramenta desconhecida (o nome não existe no sistema)");
        expect(refusalReasonLabel("argumento_invalido")).toBe("argumento inválido");
        expect(refusalReasonLabel("erro_de_execucao")).toBe("falha ao executar a ferramenta");
        expect(refusalReasonLabel("limite_de_chamadas")).toBe("passou do limite de chamadas de ferramenta nesta mensagem");
        expect(refusalReasonLabel("fora_do_turno")).toBe("fora do escopo deste turno");
        expect(refusalReasonLabel(null)).toBe("motivo não registrado");
        expect(refusalReasonLabel("motivo_novo")).toBe("motivo_novo");
    });

    it("duração zero é menos de 1 ms, e ausência é não registrada", () => {
        expect(formatToolDuration(0)).toBe("menos de 1 ms");
        expect(formatToolDuration(null)).toBe("não registrada");
        expect(formatToolDuration(42)).toBe(formatMs(42));
    });
});

describe("formatarConteudoDeFerramenta (HR-49b)", () => {
    it("indenta objeto e lista JSON com 2 espaços, sem mudar os valores", () => {
        expect(formatToolContent('{"a":1,"b":[2,3],"c":{"d":"e:f, g"}}')).toBe(
            '{\n  "a": 1,\n  "b": [\n    2,\n    3\n  ],\n  "c": {\n    "d": "e:f, g"\n  }\n}',
        );
        expect(formatToolContent('[{"x":null,"y":true}]')).toBe('[\n  {\n    "x": null,\n    "y": true\n  }\n]');
    });

    it("mantém vazios e números decimais como foram gravados", () => {
        expect(formatToolContent('{"vazio":{},"lista":[],"preco":10.50,"neg":-0.0}')).toBe(
            '{\n  "vazio": {},\n  "lista": [],\n  "preco": 10.50,\n  "neg": -0.0\n}',
        );
    });

    it("não mexe em texto com escape ou com pontuação dentro de aspas", () => {
        const texto = '{"frase":"aspas \\"x\\", chave: [valor]"}';
        const saida = formatToolContent(texto);
        expect(saida).toBe('{\n  "frase": "aspas \\"x\\", chave: [valor]"\n}');
        expect(JSON.parse(saida)).toEqual(JSON.parse(texto));
    });

    it("texto cortado, texto livre e JSON quebrado saem como vieram", () => {
        for (const texto of [
            '{"consulta":"abc…[cortado]',
            "Arquivo enfileirado para envio.",
            "[sem fechar",
            '{"ok":true}…[cortado]',
            "",
            "42",
            '"só um texto"',
        ]) {
            expect(formatToolContent(texto)).toBe(texto);
        }
    });

    it("número com 16 ou mais dígitos sai como veio, para não perder precisão", () => {
        const texto = '{"id":12345678901234567890}';
        expect(formatToolContent(texto)).toBe(texto);
    });

    it("preserva todos os caracteres não-espaço, em ordem", () => {
        const texto = '{"a":[1,{"b":"c d"}],"e":"f\\ng"}';
        const withoutSpaces = (value: string) => value.replace(/\s+/g, "");
        expect(withoutSpaces(formatToolContent(texto))).toBe(withoutSpaces(texto));
    });
});


describe("I18N-10: rótulos e formatação em en-US", () => {
    afterEach(() => {
        setLocale("pt-BR");
    });

    it("rótulos de estado, origem e fim do turno saem do catálogo em inglês", () => {
        setLocale("en-US");
        expect(turnStateLabel("done")).toBe("Done");
        expect(turnStateLabel("failed")).toBe("Failed");
        expect(turnStateLabel("skipped")).toBe("Did not answer");
        expect(turnStateLabel("running")).toBe("In progress");
        expect(originLabel("jev")).toBe("Answered by Jev");
        expect(originLabel("cache")).toBe("Reused from cache (no Jev call)");
        expect(originLabel(null)).toBe("not recorded");
        expect(endReasonLabel("concluido")).toBe("Complete response");
        expect(endReasonLabel("limite_de_tokens")).toBe("Stopped at the output token limit");
        expect(phraseForStateWithoutResponse("failed")).toBe("The turn failed before the AI sent a response.");
        expect(phraseForStateWithoutResponse(null as unknown as string)).toBe("No response was sent in this turn.");
    });

    it("motivos de recusa e duração das ferramentas em inglês", () => {
        setLocale("en-US");
        expect(refusalReasonLabel("nao_habilitada")).toBe("tool not enabled for this agent");
        expect(refusalReasonLabel("argumento_invalido")).toBe("invalid argument");
        expect(refusalReasonLabel(null)).toBe("reason not recorded");
        expect(formatToolDuration(null)).toBe("not recorded");
        expect(formatToolDuration(0)).toBe("less than 1 ms");
    });

    it("números, milissegundos, tokens e custo seguem o formato inglês", () => {
        setLocale("en-US");
        expect(formatMs(1234)).toBe("1,234 ms");
        expect(formatTokens(1234)).toBe("1,234 tokens");
        expect(formatCostMicros(1820)).toBe("1,820 currency micros (the API does not report the currency)");
        expect(formatCostMicros(0)).toBe(
            "not calculated in this turn (the turn does not record the cost yet)",
        );
        expect(formatConfidence(0.92)).toBe("92%");
        expect(formatConfidence(null)).toBe("not recorded");
        expect(formatMargin(0.61)).toBe("61.0%");
    });

    it("nome de reserva do produto e do trecho também trocam de idioma", () => {
        setLocale("en-US");
        const retrieved = readTurnRetrieved([
            { tipo: "produto", "produto": "p1", "preco_centavos": null },
            { tipo: "trecho", item: "i1" },
        ]);
        expect(retrieved["produtos"][0]?.nome).toBe("Product without a name");
        expect(retrieved["trechos"][0]?.["titulo"]).toBe("Snippet without a title");
    });
});
