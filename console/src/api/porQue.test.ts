/**
 * Leitura do "por quê" do turno (HR-47): o que o turno gravou vira dado da tela, e o que não foi
 * gravado (ou veio com outro formato) não vira número inventado.
 */

import { afterEach, describe, expect, it } from "vitest";
import { setLocale } from "../i18n";
import {
    contarFerramentas,
    duracaoDoTurno,
    formatarConfianca,
    formatarConteudoDeFerramenta,
    formatarCustoMicros,
    formatarMargem,
    formatarDuracaoDeFerramenta,
    formatarMs,
    formatarTokens,
    frasePeloEstadoSemResposta,
    intercalarTurnos,
    lerChamadasDeFerramenta,
    lerFerramentasDoTurno,
    lerJevDoTurno,
    lerRecuperadosDoTurno,
    lerTemposDoTurno,
    rotuloDaOrigem,
    rotuloDoEstadoDoTurno,
    rotuloDoMotivoFim,
    rotuloDoMotivoRecusa,
    tituloDoGate,
} from "./porQue";

/** Registro de Jev como o motor grava em `turns.jev_json` (HR-28/HR-45). */
const JEV_REAL = {
    chamada: {
        perguntas: 2,
        decididas_por_cache: 1,
        ms: 310,
        tokens: 480,
        usage: { entrada: 400, saida: 80, total: 480, informado: true },
        falha: null,
    },
    decisoes: [
        {
            gate: "g3",
            escolha: "base",
            origem: "jev",
            confianca: 0.92,
            margem: 0.61,
            motivo: null,
            ms: 310,
            tokens: 240,
            escolha_do_jev: "base",
        },
        {
            gate: "g1",
            escolha: "continuar_agente",
            origem: "fallback",
            confianca: 0,
            margem: 0,
            motivo: "o Jev não respondeu a tempo",
            ms: 0,
            tokens: 0,
        },
    ],
};

describe("lerJevDoTurno", () => {
    it("lê a chamada, as decisões e a escolha do Jev, sem inventar o que não veio", () => {
        const jev = lerJevDoTurno(JEV_REAL);

        expect(jev?.chamada).toEqual({
            perguntas: 2,
            decididasPorCache: 1,
            ms: 310,
            tokens: 480,
            entrada: 400,
            saida: 80,
            total: 480,
            falha: null,
        });
        expect(jev?.decisoes).toHaveLength(2);
        expect(jev?.decisoes[0]).toEqual({
            gate: "g3",
            escolha: "base",
            escolhaDoJev: "base",
            origem: "jev",
            confianca: 0.92,
            margem: 0.61,
            motivo: null,
            ms: 310,
            tokens: 240,
        });
        expect(jev?.decisoes[1]?.escolhaDoJev).toBeNull();
        expect(jev?.decisoes[1]?.motivo).toBe("o Jev não respondeu a tempo");
        expect(jev?.revisao).toBeNull();
    });

    it("sem registro de Jev, devolve null (nenhum gate decidiu)", () => {
        expect(lerJevDoTurno(null)).toBeNull();
        expect(lerJevDoTurno(undefined)).toBeNull();
        expect(lerJevDoTurno({})).toBeNull();
        expect(lerJevDoTurno({ decisoes: [] })).toBeNull();
    });

    it("lê a revisão do G6 quando ela existe", () => {
        const jev = lerJevDoTurno({
            ...JEV_REAL,
            revisao: {
                gate: "g6",
                escolha: "revisar_tom",
                confianca: 0.81,
                regenerou: true,
                segunda_escolha: "aprovada",
                resposta_segura: false,
            },
        });

        expect(jev?.revisao).toEqual({
            escolha: "revisar_tom",
            confianca: 0.81,
            regenerou: true,
            segundaEscolha: "aprovada",
            respostaSegura: false,
        });
    });

    it("campo com tipo errado vira não registrado, e a falha da chamada aparece", () => {
        const jev = lerJevDoTurno({
            chamada: { perguntas: "dois", ms: null, falha: "prazo de 8 s estourado", usage: "x" },
            decisoes: [{ gate: "g5", confianca: "alta", escolha: 7 }, "lixo", null],
        });

        expect(jev?.chamada?.perguntas).toBeNull();
        expect(jev?.chamada?.ms).toBeNull();
        expect(jev?.chamada?.entrada).toBeNull();
        expect(jev?.chamada?.falha).toBe("prazo de 8 s estourado");
        expect(jev?.decisoes).toHaveLength(1);
        expect(jev?.decisoes[0]?.confianca).toBeNull();
        expect(jev?.decisoes[0]?.escolha).toBeNull();
    });
});

describe("lerRecuperadosDoTurno", () => {
    it("separa trechos e produtos pelo tipo e mantém a fonte do trecho", () => {
        const recuperados = lerRecuperadosDoTurno([
            {
                tipo: "trecho",
                item: "item-1",
                titulo: "Prazo de entrega",
                grupo: "Logística",
                texto: "Entregamos em 5 dias úteis.",
                similaridade: 0.83,
                relevancia: 0.0163,
            },
            {
                tipo: "produto",
                produto: "prod-1",
                nome: "Dedetização",
                preco_centavos: 45000,
                preco_texto: "R$ 450,00 por visita",
                unidade: "por visita",
                condicoes: "Até 50 m²",
            },
        ]);

        expect(recuperados.trechos).toEqual([
            {
                item: "item-1",
                titulo: "Prazo de entrega",
                grupo: "Logística",
                texto: "Entregamos em 5 dias úteis.",
                similaridade: 0.83,
                relevancia: 0.0163,
            },
        ]);
        expect(recuperados.produtos).toEqual([
            {
                produto: "prod-1",
                nome: "Dedetização",
                precoTexto: "R$ 450,00 por visita",
                precoCentavos: 45000,
                unidade: "por visita",
                condicoes: "Até 50 m²",
            },
        ]);
    });

    it("sem uso nenhum, ou com formato inesperado, devolve listas vazias", () => {
        expect(lerRecuperadosDoTurno([])).toEqual({ trechos: [], produtos: [] });
        expect(lerRecuperadosDoTurno(null)).toEqual({ trechos: [], produtos: [] });
        expect(lerRecuperadosDoTurno({ trechos: [] })).toEqual({ trechos: [], produtos: [] });
        expect(lerRecuperadosDoTurno(["texto solto", 3])).toEqual({ trechos: [], produtos: [] });
    });
});

describe("ferramentas e tempos", () => {
    it("ferramentas vêm só como nomes; a contagem respeita a ordem da primeira chamada", () => {
        const nomes = lerFerramentasDoTurno(["consultar_base", "buscar_produto", "consultar_base"]);
        expect(nomes).toEqual(["consultar_base", "buscar_produto", "consultar_base"]);
        expect(contarFerramentas(nomes)).toEqual([
            { nome: "consultar_base", vezes: 2 },
            { nome: "buscar_produto", vezes: 1 },
        ]);
        expect(lerFerramentasDoTurno(null)).toEqual([]);
    });

    it("lê os tempos por etapa e o motivo do fim", () => {
        expect(
            lerTemposDoTurno({
                fila_ms: 3,
                montagem_ms: 40,
                primeiro_texto_ms: 480,
                total_ms: 920,
                rodadas_ferramenta: 1,
                modelo: "gemini-3.8-flash-tiered",
                motivo_fim: "concluido",
            }),
        ).toEqual({
            filaMs: 3,
            montagemMs: 40,
            primeiroTextoMs: 480,
            totalMs: 920,
            rodadasFerramenta: 1,
            modelo: "gemini-3.8-flash-tiered",
            motivoFim: "concluido",
        });
        expect(lerTemposDoTurno(null)).toBeNull();
    });

    it("duração: o total medido pelo turno; sem ele, início e fim; em andamento, nada", () => {
        expect(duracaoDoTurno(1000, 1500, { filaMs: null, montagemMs: null, primeiroTextoMs: null, totalMs: 920, rodadasFerramenta: null, modelo: null, motivoFim: null })).toBe(920);
        expect(duracaoDoTurno(1000, 1500, null)).toBe(500);
        expect(duracaoDoTurno(1000, null, null)).toBeNull();
    });
});

describe("rótulos e formatação", () => {
    it("traduz origem e motivo do fim, e não inventa quando faltam", () => {
        expect(rotuloDaOrigem("cache")).toBe("Reaproveitada do cache (sem chamada ao Jev)");
        expect(rotuloDaOrigem("fallback")).toBe("Valeu o padrão do gate; a resposta do Jev não foi usada");
        expect(rotuloDaOrigem("algo_novo")).toBe("não registrado");
        expect(rotuloDoMotivoFim("limite_de_tokens")).toBe("Parou no limite de tokens de saída");
        expect(rotuloDoMotivoFim("motivo_que_nao_conheço")).toBe("motivo_que_nao_conheço");
        expect(rotuloDoMotivoFim(null)).toBe("não registrado");
    });

    it("gate usa o título da aba Modelo e Jev; id desconhecido aparece cru", () => {
        expect(tituloDoGate("g1")).toBe("G1 · Transferir para humano agora?");
        expect(tituloDoGate("g9")).toBe("G9");
    });

    it("custo sem valor diz que não foi gravado; com valor, diz a unidade sem converter", () => {
        expect(formatarCustoMicros(null)).toBe("não calculado neste turno (o turno ainda não grava o custo)");
        expect(formatarCustoMicros(0)).toBe("não calculado neste turno (o turno ainda não grava o custo)");
        expect(formatarCustoMicros(1820)).toBe("1.820 micros de moeda (a API não informa a moeda)");
    });

    it("milissegundos em pt-BR e frase para turno sem resposta", () => {
        expect(formatarMs(null)).toBe("não registrado");
        expect(formatarMs(1234)).toBe("1.234 ms");
        expect(frasePeloEstadoSemResposta("failed")).toBe("O turno falhou antes de a IA enviar uma resposta.");
        expect(frasePeloEstadoSemResposta("running")).toBe("O turno ainda está em andamento.");
    });
});

describe("intercalarTurnos", () => {
    it("põe o turno sem resposta entre as mensagens em que ele aconteceu", () => {
        const mensagens = [
            { id: "m1", created_at: 100 },
            { id: "m2", created_at: 400 },
            { id: "m3", created_at: 900 },
        ];
        const turnos = [
            { id: "t1", started_at: 150 },
            { id: "t2", started_at: 500 },
        ];

        const itens = intercalarTurnos(mensagens, turnos).map((item) =>
            item.tipo === "mensagem" ? item.mensagem.id : item.turno.id,
        );

        expect(itens).toEqual(["m1", "t1", "m2", "t2", "m3"]);
    });

    it("turno que começou depois da última mensagem vai para o fim; sem mensagens, só turnos", () => {
        expect(
            intercalarTurnos([{ id: "m1", created_at: 100 }], [{ id: "t9", started_at: 999 }]).map((item) =>
                item.tipo === "mensagem" ? item.mensagem.id : item.turno.id,
            ),
        ).toEqual(["m1", "t9"]);
        expect(intercalarTurnos([], [{ id: "t1", started_at: 1 }]).map((item) => item.tipo)).toEqual(["turno"]);
    });
});

describe("chamadas de ferramenta (HR-47c)", () => {
    it("lê as chamadas na ordem de rodada e de chamada, com estado e motivo", () => {
        const lidas = lerChamadasDeFerramenta([
            { nome: "cart_add_item", rodada: 1, ordem: 3, estado: "recusada", motivo: "nao_habilitada", duracao_ms: 0, argumentos: "{}", resultado: "erro" },
            { nome: "enviar_arquivo", rodada: 0, ordem: 2, estado: "executada", motivo: null, duracao_ms: 7, argumentos: "{}", resultado: "ok" },
            { nome: "buscar_produto", rodada: 0, ordem: 1, estado: "executada", motivo: null, duracao_ms: 3, argumentos: "{}", resultado: "ok" },
        ]);
        expect(lidas.map((c) => c.nome)).toEqual(["buscar_produto", "enviar_arquivo", "cart_add_item"]);
        expect(lidas[2]).toMatchObject({ estado: "recusada", motivo: "nao_habilitada", duracaoMs: 0 });
        expect(lidas[0]).toMatchObject({ estado: "executada", motivo: null, duracaoMs: 3 });
    });

    it("turno antigo, ausente, nulo ou com outro formato vira lista vazia, sem erro", () => {
        expect(lerChamadasDeFerramenta(undefined)).toEqual([]);
        expect(lerChamadasDeFerramenta(null)).toEqual([]);
        expect(lerChamadasDeFerramenta({ nomes: ["x"] })).toEqual([]);
        expect(lerChamadasDeFerramenta([null, 3, "texto", { sem_nome: true }])).toEqual([]);
    });

    it("estado desconhecido não vira executada nem recusada; motivo só existe quando recusada", () => {
        const lidas = lerChamadasDeFerramenta([
            { nome: "x", estado: "talvez", motivo: "nao_habilitada", argumentos: "a", resultado: "b" },
        ]);
        expect(lidas).toEqual([
            expect.objectContaining({
                estado: "nao_registrado",
                motivo: null,
                rodada: null,
                duracaoMs: null,
            }),
        ]);

        const executadas = lerChamadasDeFerramenta([{ nome: "y", estado: "executada", motivo: "nao_habilitada" }]);
        expect(executadas).toEqual([expect.objectContaining({ estado: "executada", motivo: null })]);
    });

    it("campos ilegíveis viram null, nunca número ou texto inventado", () => {
        const lidas = lerChamadasDeFerramenta([
            { nome: "z", estado: "executada", rodada: "1", duracao_ms: "9", argumentos: "  ", resultado: 4 },
        ]);
        expect(lidas).toEqual([
            expect.objectContaining({ rodada: null, duracaoMs: null, argumentos: null, resultado: null }),
        ]);
    });

    it("cada motivo de recusa tem texto em português, e motivo desconhecido aparece como veio", () => {
        expect(rotuloDoMotivoRecusa("nao_habilitada")).toBe("ferramenta não habilitada neste agente");
        expect(rotuloDoMotivoRecusa("desconhecida")).toBe("ferramenta desconhecida (o nome não existe no sistema)");
        expect(rotuloDoMotivoRecusa("argumento_invalido")).toBe("argumento inválido");
        expect(rotuloDoMotivoRecusa("erro_de_execucao")).toBe("falha ao executar a ferramenta");
        expect(rotuloDoMotivoRecusa("limite_de_chamadas")).toBe("passou do limite de chamadas de ferramenta nesta mensagem");
        expect(rotuloDoMotivoRecusa("fora_do_turno")).toBe("fora do escopo deste turno");
        expect(rotuloDoMotivoRecusa(null)).toBe("motivo não registrado");
        expect(rotuloDoMotivoRecusa("motivo_novo")).toBe("motivo_novo");
    });

    it("duração zero é menos de 1 ms, e ausência é não registrada", () => {
        expect(formatarDuracaoDeFerramenta(0)).toBe("menos de 1 ms");
        expect(formatarDuracaoDeFerramenta(null)).toBe("não registrada");
        expect(formatarDuracaoDeFerramenta(42)).toBe(formatarMs(42));
    });
});

describe("formatarConteudoDeFerramenta (HR-49b)", () => {
    it("indenta objeto e lista JSON com 2 espaços, sem mudar os valores", () => {
        expect(formatarConteudoDeFerramenta('{"a":1,"b":[2,3],"c":{"d":"e:f, g"}}')).toBe(
            '{\n  "a": 1,\n  "b": [\n    2,\n    3\n  ],\n  "c": {\n    "d": "e:f, g"\n  }\n}',
        );
        expect(formatarConteudoDeFerramenta('[{"x":null,"y":true}]')).toBe('[\n  {\n    "x": null,\n    "y": true\n  }\n]');
    });

    it("mantém vazios e números decimais como foram gravados", () => {
        expect(formatarConteudoDeFerramenta('{"vazio":{},"lista":[],"preco":10.50,"neg":-0.0}')).toBe(
            '{\n  "vazio": {},\n  "lista": [],\n  "preco": 10.50,\n  "neg": -0.0\n}',
        );
    });

    it("não mexe em texto com escape ou com pontuação dentro de aspas", () => {
        const texto = '{"frase":"aspas \\"x\\", chave: [valor]"}';
        const saida = formatarConteudoDeFerramenta(texto);
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
            expect(formatarConteudoDeFerramenta(texto)).toBe(texto);
        }
    });

    it("número com 16 ou mais dígitos sai como veio, para não perder precisão", () => {
        const texto = '{"id":12345678901234567890}';
        expect(formatarConteudoDeFerramenta(texto)).toBe(texto);
    });

    it("preserva todos os caracteres não-espaço, em ordem", () => {
        const texto = '{"a":[1,{"b":"c d"}],"e":"f\\ng"}';
        const semEspacos = (valor: string) => valor.replace(/\s+/g, "");
        expect(semEspacos(formatarConteudoDeFerramenta(texto))).toBe(semEspacos(texto));
    });
});


describe("I18N-10: rótulos e formatação em en-US", () => {
    afterEach(() => {
        setLocale("pt-BR");
    });

    it("rótulos de estado, origem e fim do turno saem do catálogo em inglês", () => {
        setLocale("en-US");
        expect(rotuloDoEstadoDoTurno("done")).toBe("Done");
        expect(rotuloDoEstadoDoTurno("failed")).toBe("Failed");
        expect(rotuloDoEstadoDoTurno("skipped")).toBe("Did not answer");
        expect(rotuloDoEstadoDoTurno("running")).toBe("In progress");
        expect(rotuloDaOrigem("jev")).toBe("Answered by Jev");
        expect(rotuloDaOrigem("cache")).toBe("Reused from cache (no Jev call)");
        expect(rotuloDaOrigem(null)).toBe("not recorded");
        expect(rotuloDoMotivoFim("concluido")).toBe("Complete response");
        expect(rotuloDoMotivoFim("limite_de_tokens")).toBe("Stopped at the output token limit");
        expect(frasePeloEstadoSemResposta("failed")).toBe("The turn failed before the AI sent a response.");
        expect(frasePeloEstadoSemResposta(null as unknown as string)).toBe("No response was sent in this turn.");
    });

    it("motivos de recusa e duração das ferramentas em inglês", () => {
        setLocale("en-US");
        expect(rotuloDoMotivoRecusa("nao_habilitada")).toBe("tool not enabled for this agent");
        expect(rotuloDoMotivoRecusa("argumento_invalido")).toBe("invalid argument");
        expect(rotuloDoMotivoRecusa(null)).toBe("reason not recorded");
        expect(formatarDuracaoDeFerramenta(null)).toBe("not recorded");
        expect(formatarDuracaoDeFerramenta(0)).toBe("less than 1 ms");
    });

    it("números, milissegundos, tokens e custo seguem o formato inglês", () => {
        setLocale("en-US");
        expect(formatarMs(1234)).toBe("1,234 ms");
        expect(formatarTokens(1234)).toBe("1,234 tokens");
        expect(formatarCustoMicros(1820)).toBe("1,820 currency micros (the API does not report the currency)");
        expect(formatarCustoMicros(0)).toBe(
            "not calculated in this turn (the turn does not record the cost yet)",
        );
        expect(formatarConfianca(0.92)).toBe("92%");
        expect(formatarConfianca(null)).toBe("not recorded");
        expect(formatarMargem(0.61)).toBe("61.0%");
    });

    it("nome de reserva do produto e do trecho também trocam de idioma", () => {
        setLocale("en-US");
        const recuperados = lerRecuperadosDoTurno([
            { tipo: "produto", produto: "p1", preco_centavos: null },
            { tipo: "trecho", item: "i1" },
        ]);
        expect(recuperados.produtos[0]?.nome).toBe("Product without a name");
        expect(recuperados.trechos[0]?.titulo).toBe("Snippet without a title");
    });
});
