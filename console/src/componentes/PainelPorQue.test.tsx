/**
 * Painel do "por quê" (HR-47), testado sozinho com um turno montado à mão: o que aparece quando o
 * turno gravou tudo, o que aparece quando não gravou nada, o erro de leitura com "Tentar de novo" e
 * o foco preso, `Esc` fechando e o foco voltando ao botão que abriu.
 */

import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { TurnoTeste } from "../api/testeChat";
import { setLocale } from "../i18n";
import { PainelPorQue } from "./PainelPorQue";

const TURNO_COMPLETO: TurnoTeste = {
    id: "turno-1",
    state: "done",
    files_out: [],
    jev: {
        chamada: {
            perguntas: 2,
            decididas_por_cache: 0,
            ms: 310,
            tokens: 480,
            usage: { entrada: 400, saida: 80, total: 480, informado: true },
            falha: null,
        },
        decisoes: [
            {
                gate: "g3",
                escolha: "base",
                escolha_do_jev: "base",
                origem: "jev",
                confianca: 0.92,
                margem: 0.61,
                motivo: null,
                ms: 310,
                tokens: 240,
            },
        ],
    },
    retrieved: [
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
    ],
    tools: ["consultar_base", "buscar_produto"],
    timings: {
        fila_ms: 3,
        montagem_ms: 40,
        primeiro_texto_ms: 480,
        total_ms: 920,
        rodadas_ferramenta: 1,
        modelo: "gemini-3.8-flash-tiered",
        motivo_fim: "concluido",
    },
    input_tokens: 120,
    output_tokens: 16,
    cost_micros: null,
    error: null,
    started_at: 1_700_000_000_000,
    finished_at: 1_700_000_000_920,
};

type Opcoes = {
    turno?: TurnoTeste | null;
    erro?: string | null;
    atualizando?: boolean;
    aoTentarDeNovo?: () => void;
};

/** Um botão que abre o painel, como a mensagem do agente faz na conversa. */
function Vitrine({ turno, erro = null, atualizando = false, aoTentarDeNovo = () => {} }: Opcoes) {
    const [aberto, setAberto] = useState(false);
    return (
        <>
            <button type="button" onClick={() => setAberto(true)}>
                Ver por quê (turno 1)
            </button>
            <PainelPorQue
                aberto={aberto}
                numero={1}
                turno={turno === undefined ? TURNO_COMPLETO : turno}
                atualizando={atualizando}
                erroDeLeitura={erro}
                aoFechar={() => setAberto(false)}
                aoTentarDeNovo={aoTentarDeNovo}
            />
        </>
    );
}

function abrir(opcoes: Opcoes = {}) {
    render(<Vitrine {...opcoes} />);
    return userEvent.setup();
}

afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
});

describe("conteúdo do painel", () => {
    it("mostra decisões do Jev, base e produtos com a fonte, ferramentas, tempos e tokens", async () => {
        const usuario = abrir();
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));

        const dialogo = await screen.findByRole("dialog", { name: "Por quê do turno 1" });

        // Cabeçalho: tokens e custo sem valor gravado
        expect(within(dialogo).getByText("Resumo do turno")).toBeInTheDocument();
        expect(within(dialogo).getByText(/136 tokens \(entrada 120 tokens, saída 16 tokens\)/)).toBeInTheDocument();
        expect(within(dialogo).getByText("não calculado neste turno (o turno ainda não grava o custo)")).toBeInTheDocument();

        // Jev: gate com título, escolha, origem, confiança, margem e tempo
        expect(within(dialogo).getByRole("heading", { name: "G3 · Precisa de base, de produto ou de nada?" })).toBeInTheDocument();
        expect(within(dialogo).getByText("Respondida pelo Jev")).toBeInTheDocument();
        expect(within(dialogo).getByText("92%")).toBeInTheDocument();
        expect(within(dialogo).getByText("61,0%")).toBeInTheDocument();
        expect(within(dialogo).getByText("2 pergunta(s), 0 reaproveitada(s) do cache")).toBeInTheDocument();

        // Base e catálogo com fonte
        expect(within(dialogo).getByText("Prazo de entrega")).toBeInTheDocument();
        expect(within(dialogo).getByText(/Grupo: Logística/)).toBeInTheDocument();
        expect(within(dialogo).getByText("Dedetização")).toBeInTheDocument();
        expect(within(dialogo).getByText(/R\$ 450,00 por visita/)).toBeInTheDocument();

        // Ferramentas e tempos
        expect(within(dialogo).getByText("consultar_base")).toBeInTheDocument();
        expect(within(dialogo).getByText("Resposta completa")).toBeInTheDocument();
        expect(within(dialogo).getByRole("progressbar", { name: "Até o primeiro texto: 480 ms de 920 ms" })).toBeInTheDocument();
    });

    it("declara o que não foi gravado, em vez de inventar", async () => {
        const usuario = abrir();
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");

        expect(within(dialogo).getByText(/o texto da pergunta enviada ao Jev e as probabilidades/)).toBeInTheDocument();
        expect(within(dialogo).getByText(/link para a fonte oficial e indicação de corte/)).toBeInTheDocument();
        expect(within(dialogo).getByText(/a estrutura completa do retorno de cada ferramenta/)).toBeInTheDocument();
        expect(within(dialogo).getByText(/o tempo da busca na base separado/)).toBeInTheDocument();
    });

    it("turno sem Jev, sem recuperados e sem ferramentas mostra os vazios sem quebrar", async () => {
        const usuario = abrir({
            turno: { ...TURNO_COMPLETO, jev: null, retrieved: [], tools: [], timings: null },
        });
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");

        expect(within(dialogo).getByText("Nenhum gate decidiu neste turno.")).toBeInTheDocument();
        expect(within(dialogo).getByText("Nenhum trecho da base nem produto entrou na resposta deste turno.")).toBeInTheDocument();
        expect(within(dialogo).getByText("Nenhuma ferramenta foi chamada neste turno.")).toBeInTheDocument();
        expect(within(dialogo).getByText("Os tempos não foram registrados neste turno.")).toBeInTheDocument();
    });

    it("turno falho mostra o estado e o erro gravado", async () => {
        const usuario = abrir({
            turno: {
                ...TURNO_COMPLETO,
                state: "failed",
                error: "chamada ao LLM cancelada",
                output_tokens: null,
                finished_at: null,
                timings: null,
            },
        });
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");

        expect(within(dialogo).getByText("Falhou")).toBeInTheDocument();
        expect(within(dialogo).getByText("Erro registrado")).toBeInTheDocument();
        expect(within(dialogo).getAllByText("chamada ao LLM cancelada").length).toBeGreaterThan(0);
        expect(within(dialogo).getByText("não registrada")).toBeInTheDocument();
    });

    it("tokens zerados (turno que não usou o modelo) não aparecem como medição", async () => {
        const usuario = abrir({
            turno: { ...TURNO_COMPLETO, state: "failed", input_tokens: 0, output_tokens: 0, error: "cancelado" },
        });
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");

        const tokens = within(dialogo).getByText("Tokens do modelo").closest("div") as HTMLElement;
        expect(within(tokens).getByText("não registrado")).toBeInTheDocument();
        expect(tokens).not.toHaveTextContent(/\b0 tokens\b/);
    });

    it("o custo, quando a API manda, aparece com a unidade e sem conversão", async () => {
        const usuario = abrir({ turno: { ...TURNO_COMPLETO, cost_micros: 1820 } });
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");

        expect(within(dialogo).getByText("1.820 micros de moeda (a API não informa a moeda)")).toBeInTheDocument();
    });
});

describe("leitura, foco e fechamento", () => {
    it("erro de leitura aparece no painel com Tentar de novo, e o que já estava na tela continua", async () => {
        const tentar = vi.fn();
        const usuario = abrir({ erro: "falha de conexão com o servidor", aoTentarDeNovo: tentar });
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");

        expect(within(dialogo).getByText("Não consegui atualizar os dados deste turno")).toBeInTheDocument();
        expect(within(dialogo).getByText("falha de conexão com o servidor")).toBeInTheDocument();
        expect(within(dialogo).getByText("Prazo de entrega")).toBeInTheDocument();

        await usuario.click(within(dialogo).getByRole("button", { name: "Tentar de novo" }));
        expect(tentar).toHaveBeenCalledTimes(1);
    });

    it("sem o turno na leitura, diz para atualizar a conversa", async () => {
        const usuario = abrir({ turno: null });
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");

        expect(within(dialogo).getByText(/não está na leitura atual da conversa/)).toBeInTheDocument();
    });

    it("Esc fecha e devolve o foco ao botão que abriu", async () => {
        const usuario = abrir();
        const botao = screen.getByRole("button", { name: "Ver por quê (turno 1)" });
        await usuario.click(botao);
        await screen.findByRole("dialog");

        await usuario.keyboard("{Escape}");

        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
        await waitFor(() => expect(botao).toHaveFocus());
    });

    it("o foco fica preso no painel enquanto ele está aberto", async () => {
        const usuario = abrir();
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");
        // O prendedor de foco (Floating UI) devolve o foco ao painel com um atraso curto; sob carga
        // (CI em paralelo) esse atraso cresce. O que vale é o foco voltar ao painel, não a hora exata.
        await waitFor(() => expect(dialogo.contains(document.activeElement)).toBe(true));
        for (let i = 0; i < 5; i += 1) {
            await usuario.tab();
            await waitFor(() => expect(dialogo.contains(document.activeElement)).toBe(true));
        }
    });
});

describe("chamadas de ferramenta (HR-47c, tool_calls)", () => {
    const EXECUTADA = {
        nome: "enviar_arquivo",
        rodada: 0,
        ordem: 1,
        estado: "executada",
        motivo: null,
        duracao_ms: 12,
        argumentos: '{"id":"arq-1"}',
        resultado: "Arquivo enfileirado para envio.",
    };

    const RECUSADA = {
        nome: "cart_add_item",
        rodada: 0,
        ordem: 2,
        estado: "recusada",
        motivo: "nao_habilitada",
        duracao_ms: 0,
        argumentos: '{"token":"[redigido]","produto_id":"p-1"}',
        resultado: "erro: a ferramenta `cart_add_item` não está habilitada para este agente",
    };

    it("chamada executada mostra nome, rodada, duração, argumentos e resultado", async () => {
        const usuario = abrir({ turno: { ...TURNO_COMPLETO, tools: ["enviar_arquivo"], tool_calls: [EXECUTADA] } });
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");

        expect(within(dialogo).getByText("enviar_arquivo")).toBeInTheDocument();
        expect(within(dialogo).getByText("Executada")).toBeInTheDocument();
        expect(within(dialogo).getByText("rodada 1, chamada 1")).toBeInTheDocument();
        expect(within(dialogo).getByText("12 ms")).toBeInTheDocument();
        // JSON válido sai indentado com 2 espaços (HR-49b); o valor não muda.
        expect(within(dialogo).getByLabelText("Argumentos de enviar_arquivo").textContent).toBe('{\n  "id": "arq-1"\n}');
        expect(within(dialogo).getByLabelText("Resultado de enviar_arquivo")).toHaveTextContent("Arquivo enfileirado para envio.");
        expect(within(dialogo).queryByText(/Motivo:/)).not.toBeInTheDocument();
    });

    it("argumentos JSON indentados mantêm cada valor como foi gravado (10.50 continua 10.50)", async () => {
        const argumentos = '{"produto":{"nome":"Clareamento","preco":10.50},"itens":[1,2],"vazio":{},"lista":[]}';
        const usuario = abrir({ turno: { ...TURNO_COMPLETO, tools: ["cart_add_item"], tool_calls: [{ ...EXECUTADA, argumentos }] } });
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");

        expect(within(dialogo).getByLabelText("Argumentos de enviar_arquivo").textContent).toBe(
            '{\n  "produto": {\n    "nome": "Clareamento",\n    "preco": 10.50\n  },\n  "itens": [\n    1,\n    2\n  ],\n  "vazio": {},\n  "lista": []\n}',
        );
    });

    it("argumento cortado (…[cortado]) e texto que não é JSON aparecem exatamente como vieram", async () => {
        const cortado = `{"consulta":"${"x".repeat(40)}…[cortado]`;
        const textoLivre = "id=arq-1; sem formato de JSON";
        const usuario = abrir({
            turno: {
                ...TURNO_COMPLETO,
                tools: ["consultar_base", "enviar_arquivo"],
                tool_calls: [
                    { ...EXECUTADA, nome: "consultar_base", argumentos: cortado, resultado: textoLivre },
                    { ...EXECUTADA, ordem: 2, argumentos: "[sem fechar", resultado: `{"ok":true}…[cortado]` },
                ],
            },
        });
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");

        expect(within(dialogo).getByLabelText("Argumentos de consultar_base").textContent).toBe(cortado);
        expect(within(dialogo).getByLabelText("Resultado de consultar_base").textContent).toBe(textoLivre);
        expect(within(dialogo).getByLabelText("Argumentos de enviar_arquivo").textContent).toBe("[sem fechar");
        expect(within(dialogo).getByLabelText("Resultado de enviar_arquivo").textContent).toBe('{"ok":true}…[cortado]');
    });

    it("chamada recusada mostra o motivo traduzido e não se confunde com executada", async () => {
        const usuario = abrir({ turno: { ...TURNO_COMPLETO, tools: ["cart_add_item"], tool_calls: [RECUSADA] } });
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");

        expect(within(dialogo).getByText("Recusada, não rodou")).toBeInTheDocument();
        expect(within(dialogo).queryByText("Executada")).not.toBeInTheDocument();
        expect(within(dialogo).getByText(/Motivo: ferramenta não habilitada neste agente/)).toBeInTheDocument();
        expect(within(dialogo).getByText("Erro devolvido ao modelo")).toBeInTheDocument();
        expect(within(dialogo).getByLabelText("Argumentos de cart_add_item")).toHaveTextContent("[redigido]");
        // duração zero é "menos de 1 ms", não "não medido"
        expect(within(dialogo).getByText("menos de 1 ms")).toBeInTheDocument();
        // a recusa aparece com o cartão de atenção, não com o de sucesso
        const cartao = within(dialogo).getByText("cart_add_item").closest("li") as HTMLElement;
        expect(cartao).toHaveClass("poph-porque__chamada--recusada");
    });

    it("cada motivo de recusa aparece com o texto em português", async () => {
        const motivos: Array<[string, string]> = [
            ["nao_habilitada", "ferramenta não habilitada neste agente"],
            ["desconhecida", "ferramenta desconhecida (o nome não existe no sistema)"],
            ["argumento_invalido", "argumento inválido"],
            ["erro_de_execucao", "falha ao executar a ferramenta"],
            ["limite_de_chamadas", "passou do limite de chamadas de ferramenta nesta mensagem"],
            ["fora_do_turno", "fora do escopo deste turno"],
        ];
        const chamadas = motivos.map(([motivo], indice) => ({
            ...RECUSADA,
            nome: `ferramenta_${indice}`,
            ordem: indice + 1,
            motivo,
        }));
        const usuario = abrir({ turno: { ...TURNO_COMPLETO, tool_calls: chamadas } });
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");

        for (const [, rotulo] of motivos) {
            expect(within(dialogo).getByText(new RegExp(`Motivo: ${rotulo.replace(/[()]/g, "\\$&")}`))).toBeInTheDocument();
        }
        expect(within(dialogo).queryByText(/nao_habilitada|limite_de_chamadas/)).not.toBeInTheDocument();
    });

    it("turno sem chamada e sem ferramenta mostra o vazio, sem lista", async () => {
        const usuario = abrir({ turno: { ...TURNO_COMPLETO, tools: [], tool_calls: [] } });
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");

        expect(within(dialogo).getByText("Nenhuma ferramenta foi chamada neste turno.")).toBeInTheDocument();
        expect(within(dialogo).queryByRole("list", { name: "Chamadas de ferramenta, na ordem em que aconteceram" })).not.toBeInTheDocument();
    });

    it("turno antigo (tools preenchido, sem tool_calls) mostra os nomes e declara o que não foi gravado", async () => {
        const semCampo = { ...TURNO_COMPLETO, tools: ["consultar_base", "consultar_base"] } as Partial<TurnoTeste>;
        delete semCampo.tool_calls;
        const usuario = abrir({ turno: semCampo as TurnoTeste });
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");

        expect(within(dialogo).getByText("consultar_base")).toBeInTheDocument();
        expect(within(dialogo).getByText("chamada 2 vezes")).toBeInTheDocument();
        expect(within(dialogo).getByText(/Estado, motivo,\s+duração, argumentos e resultado não foram gravados/)).toBeInTheDocument();
    });

    it("tool_calls vazio com tools preenchido não quebra e não mostra lista de detalhes", async () => {
        const usuario = abrir({ turno: { ...TURNO_COMPLETO, tools: ["buscar_produto"], tool_calls: [] } });
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");

        expect(within(dialogo).getByText("buscar_produto")).toBeInTheDocument();
        expect(within(dialogo).queryByText("Executada")).not.toBeInTheDocument();
        expect(within(dialogo).queryByText("Recusada, não rodou")).not.toBeInTheDocument();
    });

    it("argumentos ou resultado ausentes aparecem como não registrados", async () => {
        const usuario = abrir({
            turno: { ...TURNO_COMPLETO, tool_calls: [{ ...EXECUTADA, argumentos: null, resultado: null, duracao_ms: null }] },
        });
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");

        expect(within(dialogo).getByText("não registrada")).toBeInTheDocument();
        expect(within(dialogo).getAllByText("não registrados").length).toBeGreaterThan(0);
        expect(within(dialogo).getAllByText("não registrado").length).toBeGreaterThan(0);
    });
});

describe("I18N-10: painel em en-US", () => {
    afterEach(() => {
        act(() => setLocale("pt-BR"));
    });

    it("o resumo, as decisões do Jev e os tempos falam inglês", async () => {
        setLocale("en-US");
        const usuario = abrir();
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));

        const dialogo = await screen.findByRole("dialog", { name: "Why for turn 1" });

        expect(within(dialogo).getByText("Turn summary")).toBeInTheDocument();
        expect(
            within(dialogo).getByText(/136 tokens \(input 120 tokens, output 16 tokens\)/),
        ).toBeInTheDocument();
        expect(
            within(dialogo).getByText("not calculated in this turn (the turn does not record the cost yet)"),
        ).toBeInTheDocument();
        expect(within(dialogo).getByText("Answered by Jev")).toBeInTheDocument();
        expect(within(dialogo).getByText("92%")).toBeInTheDocument();
        expect(within(dialogo).getByText("61.0%")).toBeInTheDocument();
        expect(within(dialogo).getByText("2 question(s), 0 reused from cache")).toBeInTheDocument();
        expect(within(dialogo).getByText(/Group: Logística · Similarity: 83% · Relevance \(ranking\): 0.0163/)).toBeInTheDocument();
        expect(within(dialogo).getByText(/Price the customer saw:/)).toBeInTheDocument();
        expect(within(dialogo).getByText("Complete response")).toBeInTheDocument();
        expect(
            within(dialogo).getByRole("progressbar", { name: "Until the first text: 480 ms of 920 ms" }),
        ).toBeInTheDocument();
    });

    it("chamada recusada usa os rótulos em inglês", async () => {
        setLocale("en-US");
        const usuario = abrir({
            turno: {
                ...TURNO_COMPLETO,
                tools: ["cart_add_item"],
                tool_calls: [
                    {
                        nome: "cart_add_item",
                        rodada: 0,
                        ordem: 2,
                        estado: "recusada",
                        motivo: "nao_habilitada",
                        duracao_ms: 0,
                        argumentos: '{"token":"[redigido]"}',
                        resultado: "erro",
                    },
                ],
            },
        });
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog");

        expect(within(dialogo).getByText("Rejected, did not run")).toBeInTheDocument();
        expect(within(dialogo).getByText(/Reason: tool not enabled for this agent/)).toBeInTheDocument();
        expect(within(dialogo).getByText("Error returned to the model")).toBeInTheDocument();
        expect(within(dialogo).getByLabelText("Arguments of cart_add_item")).toHaveTextContent("[redigido]");
        expect(within(dialogo).getByText("less than 1 ms")).toBeInTheDocument();
    });

    it("a troca para inglês reescreve o painel aberto sem recarregar", async () => {
        const usuario = abrir();
        await usuario.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialogo = await screen.findByRole("dialog", { name: "Por quê do turno 1" });
        expect(within(dialogo).getByText("Resumo do turno")).toBeInTheDocument();

        act(() => setLocale("en-US"));

        expect(screen.getByRole("dialog", { name: "Why for turn 1" })).toBeInTheDocument();
        expect(screen.getByText("Turn summary")).toBeInTheDocument();
        expect(screen.getByText("Tools called")).toBeInTheDocument();
    });
});
