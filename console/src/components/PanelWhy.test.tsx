
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { TestTurn } from "../api/testChat";
import { setLocale } from "../i18n";
import { PanelWhy } from "../components/PanelWhy";

const TURN_COMPLETE: TestTurn = {
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

type Options = {
    turno?: TestTurn | null;
    error?: string | null;
    updating?: boolean;
    onTryNew?: () => void;
};

function Vitrine({ turno, error = null, updating = false, onTryNew = () => {} }: Options) {
    const [isOpen, setOpen] = useState(false);
    return (
        <>
            <button type="button" onClick={() => setOpen(true)}>
                Ver por quê (turno 1)
            </button>
            <PanelWhy
                aberto={isOpen}
                numero={1}
                turno={turno === undefined ? TURN_COMPLETE : turno}
                updating={updating}
                errorRead={error}
                onClose={() => setOpen(false)}
                onTryNew={onTryNew}
            />
        </>
    );
}

function open(options: Options = {}) {
    render(<Vitrine {...options} />);
    return userEvent.setup();
}

afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
});

describe("conteúdo do painel", () => {
    it("mostra decisões do Jev, base e produtos com a fonte, ferramentas, tempos e tokens", async () => {
        const user = open();
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));

        const dialog = await screen.findByRole("dialog", { name: "Por quê do turno 1" });

        expect(within(dialog).getByText("Resumo do turno")).toBeInTheDocument();
        expect(within(dialog).getByText(/136 tokens \(entrada 120 tokens, saída 16 tokens\)/)).toBeInTheDocument();
        expect(within(dialog).getByText("não calculado neste turno (o turno ainda não grava o custo)")).toBeInTheDocument();

        expect(within(dialog).getByRole("heading", { name: "G3 · Precisa de base, de produto ou de nada?" })).toBeInTheDocument();
        expect(within(dialog).getByText("Respondida pelo Jev")).toBeInTheDocument();
        expect(within(dialog).getByText("92%")).toBeInTheDocument();
        expect(within(dialog).getByText("61,0%")).toBeInTheDocument();
        expect(within(dialog).getByText("2 pergunta(s), 0 reaproveitada(s) do cache")).toBeInTheDocument();

        expect(within(dialog).getByText("Prazo de entrega")).toBeInTheDocument();
        expect(within(dialog).getByText(/Grupo: Logística/)).toBeInTheDocument();
        expect(within(dialog).getByText("Dedetização")).toBeInTheDocument();
        expect(within(dialog).getByText(/R\$ 450,00 por visita/)).toBeInTheDocument();

        expect(within(dialog).getByText("consultar_base")).toBeInTheDocument();
        expect(within(dialog).getByText("Resposta completa")).toBeInTheDocument();
        expect(within(dialog).getByRole("progressbar", { name: "Até o primeiro texto: 480 ms de 920 ms" })).toBeInTheDocument();
    });

    it("declara o que não foi gravado, em vez de inventar", async () => {
        const user = open();
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");

        expect(within(dialog).getByText(/o texto da pergunta enviada ao Jev e as probabilidades/)).toBeInTheDocument();
        expect(within(dialog).getByText(/link para a fonte oficial e indicação de corte/)).toBeInTheDocument();
        expect(within(dialog).getByText(/a estrutura completa do retorno de cada ferramenta/)).toBeInTheDocument();
        expect(within(dialog).getByText(/o tempo da busca na base separado/)).toBeInTheDocument();
    });

    it("turno sem Jev, sem recuperados e sem ferramentas mostra os vazios sem quebrar", async () => {
        const user = open({
            turno: { ...TURN_COMPLETE, jev: null, retrieved: [], tools: [], timings: null },
        });
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");

        expect(within(dialog).getByText("Nenhum gate decidiu neste turno.")).toBeInTheDocument();
        expect(within(dialog).getByText("Nenhum trecho da base nem produto entrou na resposta deste turno.")).toBeInTheDocument();
        expect(within(dialog).getByText("Nenhuma ferramenta foi chamada neste turno.")).toBeInTheDocument();
        expect(within(dialog).getByText("Os tempos não foram registrados neste turno.")).toBeInTheDocument();
    });

    it("turno falho mostra o estado e o erro gravado", async () => {
        const user = open({
            turno: {
                ...TURN_COMPLETE,
                state: "failed",
                error: "chamada ao LLM cancelada",
                output_tokens: null,
                finished_at: null,
                timings: null,
            },
        });
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");

        expect(within(dialog).getByText("Falhou")).toBeInTheDocument();
        expect(within(dialog).getByText("Erro registrado")).toBeInTheDocument();
        expect(within(dialog).getAllByText("chamada ao LLM cancelada").length).toBeGreaterThan(0);
        expect(within(dialog).getByText("não registrada")).toBeInTheDocument();
    });

    it("tokens zerados (turno que não usou o modelo) não aparecem como medição", async () => {
        const user = open({
            turno: { ...TURN_COMPLETE, state: "failed", input_tokens: 0, output_tokens: 0, error: "cancelado" },
        });
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");

        const tokens = within(dialog).getByText("Tokens do modelo").closest("div") as HTMLElement;
        expect(within(tokens).getByText("não registrado")).toBeInTheDocument();
        expect(tokens).not.toHaveTextContent(/\b0 tokens\b/);
    });

    it("o custo, quando a API manda, aparece com a unidade e sem conversão", async () => {
        const user = open({ turno: { ...TURN_COMPLETE, cost_micros: 1820 } });
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");

        expect(within(dialog).getByText("1.820 micros de moeda (a API não informa a moeda)")).toBeInTheDocument();
    });
});

describe("leitura, foco e fechamento", () => {
    it("erro de leitura aparece no painel com Tentar de novo, e o que já estava na tela continua", async () => {
        const tryAction = vi.fn();
        const user = open({ error: "falha de conexão com o servidor", onTryNew: tryAction });
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");

        expect(within(dialog).getByText("Não consegui atualizar os dados deste turno")).toBeInTheDocument();
        expect(within(dialog).getByText("falha de conexão com o servidor")).toBeInTheDocument();
        expect(within(dialog).getByText("Prazo de entrega")).toBeInTheDocument();

        await user.click(within(dialog).getByRole("button", { name: "Tentar de novo" }));
        expect(tryAction).toHaveBeenCalledTimes(1);
    });

    it("sem o turno na leitura, diz para atualizar a conversa", async () => {
        const user = open({ turno: null });
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");

        expect(within(dialog).getByText(/não está na leitura atual da conversa/)).toBeInTheDocument();
    });

    it("Esc fecha e devolve o foco ao botão que abriu", async () => {
        const user = open();
        const button = screen.getByRole("button", { name: "Ver por quê (turno 1)" });
        await user.click(button);
        await screen.findByRole("dialog");

        await user.keyboard("{Escape}");

        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
        await waitFor(() => expect(button).toHaveFocus());
    });

    it("o foco fica preso no painel enquanto ele está aberto", async () => {
        const user = open();
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");
        await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));
        for (let i = 0; i < 5; i += 1) {
            await user.tab();
            await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));
        }
    });
});

describe("chamadas de ferramenta (HR-47c, tool_calls)", () => {
    const EXECUTED = {
        nome: "enviar_arquivo",
        rodada: 0,
        ordem: 1,
        estado: "executada",
        motivo: null,
        duracao_ms: 12,
        argumentos: '{"id":"arq-1"}',
        resultado: "Arquivo enfileirado para envio.",
    };

    const REFUSED = {
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
        const user = open({ turno: { ...TURN_COMPLETE, tools: ["enviar_arquivo"], tool_calls: [EXECUTED] } });
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");

        expect(within(dialog).getByText("enviar_arquivo")).toBeInTheDocument();
        expect(within(dialog).getByText("Executada")).toBeInTheDocument();
        expect(within(dialog).getByText("rodada 1, chamada 1")).toBeInTheDocument();
        expect(within(dialog).getByText("12 ms")).toBeInTheDocument();
        expect(within(dialog).getByLabelText("Argumentos de enviar_arquivo").textContent).toBe('{\n  "id": "arq-1"\n}');
        expect(within(dialog).getByLabelText("Resultado de enviar_arquivo")).toHaveTextContent("Arquivo enfileirado para envio.");
        expect(within(dialog).queryByText(/Motivo:/)).not.toBeInTheDocument();
    });

    it("argumentos JSON indentados mantêm cada valor como foi gravado (10.50 continua 10.50)", async () => {
        const args = '{"produto":{"nome":"Clareamento","preco":10.50},"itens":[1,2],"vazio":{},"lista":[]}';
        const user = open({ turno: { ...TURN_COMPLETE, tools: ["cart_add_item"], tool_calls: [{ ...EXECUTED, argumentos: args }] } });
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");

        expect(within(dialog).getByLabelText("Argumentos de enviar_arquivo").textContent).toBe(
            '{\n  "produto": {\n    "nome": "Clareamento",\n    "preco": 10.50\n  },\n  "itens": [\n    1,\n    2\n  ],\n  "vazio": {},\n  "lista": []\n}',
        );
    });

    it("argumento cortado (…[cortado]) e texto que não é JSON aparecem exatamente como vieram", async () => {
        const cut = `{"consulta":"${"x".repeat(40)}…[cortado]`;
        const textFree = "id=arq-1; sem formato de JSON";
        const user = open({
            turno: {
                ...TURN_COMPLETE,
                tools: ["consultar_base", "enviar_arquivo"],
                tool_calls: [
                    { ...EXECUTED, nome: "consultar_base", argumentos: cut, resultado: textFree },
                    { ...EXECUTED, ordem: 2, argumentos: "[sem fechar", resultado: `{"ok":true}…[cortado]` },
                ],
            },
        });
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");

        expect(within(dialog).getByLabelText("Argumentos de consultar_base").textContent).toBe(cut);
        expect(within(dialog).getByLabelText("Resultado de consultar_base").textContent).toBe(textFree);
        expect(within(dialog).getByLabelText("Argumentos de enviar_arquivo").textContent).toBe("[sem fechar");
        expect(within(dialog).getByLabelText("Resultado de enviar_arquivo").textContent).toBe('{"ok":true}…[cortado]');
    });

    it("chamada recusada mostra o motivo traduzido e não se confunde com executada", async () => {
        const user = open({ turno: { ...TURN_COMPLETE, tools: ["cart_add_item"], tool_calls: [REFUSED] } });
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");

        expect(within(dialog).getByText("Recusada, não rodou")).toBeInTheDocument();
        expect(within(dialog).queryByText("Executada")).not.toBeInTheDocument();
        expect(within(dialog).getByText(/Motivo: ferramenta não habilitada neste agente/)).toBeInTheDocument();
        expect(within(dialog).getByText("Erro devolvido ao modelo")).toBeInTheDocument();
        expect(within(dialog).getByLabelText("Argumentos de cart_add_item")).toHaveTextContent("[redigido]");
        expect(within(dialog).getByText("menos de 1 ms")).toBeInTheDocument();
        const card = within(dialog).getByText("cart_add_item").closest("li") as HTMLElement;
        expect(card).toHaveClass("poph-why__call--refused");
    });

    it("cada motivo de recusa aparece com o texto em português", async () => {
        const reasons: Array<[string, string]> = [
            ["nao_habilitada", "ferramenta não habilitada neste agente"],
            ["desconhecida", "ferramenta desconhecida (o nome não existe no sistema)"],
            ["argumento_invalido", "argumento inválido"],
            ["erro_de_execucao", "falha ao executar a ferramenta"],
            ["limite_de_chamadas", "passou do limite de chamadas de ferramenta nesta mensagem"],
            ["fora_do_turno", "fora do escopo deste turno"],
        ];
        const calls = reasons.map(([motivo], index) => ({
            ...REFUSED,
            nome: `ferramenta_${index}`,
            ordem: index + 1,
            motivo,
        }));
        const user = open({ turno: { ...TURN_COMPLETE, tool_calls: calls } });
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");

        for (const [, label] of reasons) {
            expect(within(dialog).getByText(new RegExp(`Motivo: ${label.replace(/[()]/g, "\\$&")}`))).toBeInTheDocument();
        }
        expect(within(dialog).queryByText(/nao_habilitada|limite_de_chamadas/)).not.toBeInTheDocument();
    });

    it("turno sem chamada e sem ferramenta mostra o vazio, sem lista", async () => {
        const user = open({ turno: { ...TURN_COMPLETE, tools: [], tool_calls: [] } });
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");

        expect(within(dialog).getByText("Nenhuma ferramenta foi chamada neste turno.")).toBeInTheDocument();
        expect(within(dialog).queryByRole("list", { name: "Chamadas de ferramenta, na ordem em que aconteceram" })).not.toBeInTheDocument();
    });

    it("turno antigo (tools preenchido, sem tool_calls) mostra os nomes e declara o que não foi gravado", async () => {
        const withoutField = { ...TURN_COMPLETE, tools: ["consultar_base", "consultar_base"] } as Partial<TestTurn>;
        delete withoutField.tool_calls;
        const user = open({ turno: withoutField as TestTurn });
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");

        expect(within(dialog).getByText("consultar_base")).toBeInTheDocument();
        expect(within(dialog).getByText("chamada 2 vezes")).toBeInTheDocument();
        expect(within(dialog).getByText(/Estado, motivo,\s+duração, argumentos e resultado não foram gravados/)).toBeInTheDocument();
    });

    it("tool_calls vazio com tools preenchido não quebra e não mostra lista de detalhes", async () => {
        const user = open({ turno: { ...TURN_COMPLETE, tools: ["buscar_produto"], tool_calls: [] } });
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");

        expect(within(dialog).getByText("buscar_produto")).toBeInTheDocument();
        expect(within(dialog).queryByText("Executada")).not.toBeInTheDocument();
        expect(within(dialog).queryByText("Recusada, não rodou")).not.toBeInTheDocument();
    });

    it("argumentos ou resultado ausentes aparecem como não registrados", async () => {
        const user = open({
            turno: { ...TURN_COMPLETE, tool_calls: [{ ...EXECUTED, argumentos: null, resultado: null, duracao_ms: null }] },
        });
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");

        expect(within(dialog).getByText("não registrada")).toBeInTheDocument();
        expect(within(dialog).getAllByText("não registrados").length).toBeGreaterThan(0);
        expect(within(dialog).getAllByText("não registrado").length).toBeGreaterThan(0);
    });
});

describe("I18N-10: painel em en-US", () => {
    afterEach(() => {
        act(() => setLocale("pt-BR"));
    });

    it("o resumo, as decisões do Jev e os tempos falam inglês", async () => {
        setLocale("en-US");
        const user = open();
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));

        const dialog = await screen.findByRole("dialog", { name: "Why for turn 1" });

        expect(within(dialog).getByText("Turn summary")).toBeInTheDocument();
        expect(
            within(dialog).getByText(/136 tokens \(input 120 tokens, output 16 tokens\)/),
        ).toBeInTheDocument();
        expect(
            within(dialog).getByText("not calculated in this turn (the turn does not record the cost yet)"),
        ).toBeInTheDocument();
        expect(within(dialog).getByText("Answered by Jev")).toBeInTheDocument();
        expect(within(dialog).getByText("92%")).toBeInTheDocument();
        expect(within(dialog).getByText("61.0%")).toBeInTheDocument();
        expect(within(dialog).getByText("2 question(s), 0 reused from cache")).toBeInTheDocument();
        expect(within(dialog).getByText(/Group: Logística · Similarity: 83% · Relevance \(ranking\): 0.0163/)).toBeInTheDocument();
        expect(within(dialog).getByText(/Price the customer saw:/)).toBeInTheDocument();
        expect(within(dialog).getByText("Complete response")).toBeInTheDocument();
        expect(
            within(dialog).getByRole("progressbar", { name: "Until the first text: 480 ms of 920 ms" }),
        ).toBeInTheDocument();
    });

    it("chamada recusada usa os rótulos em inglês", async () => {
        setLocale("en-US");
        const user = open({
            turno: {
                ...TURN_COMPLETE,
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
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog");

        expect(within(dialog).getByText("Rejected, did not run")).toBeInTheDocument();
        expect(within(dialog).getByText(/Reason: tool not enabled for this agent/)).toBeInTheDocument();
        expect(within(dialog).getByText("Error returned to the model")).toBeInTheDocument();
        expect(within(dialog).getByLabelText("Arguments of cart_add_item")).toHaveTextContent("[redigido]");
        expect(within(dialog).getByText("less than 1 ms")).toBeInTheDocument();
    });

    it("a troca para inglês reescreve o painel aberto sem recarregar", async () => {
        const user = open();
        await user.click(screen.getByRole("button", { name: "Ver por quê (turno 1)" }));
        const dialog = await screen.findByRole("dialog", { name: "Por quê do turno 1" });
        expect(within(dialog).getByText("Resumo do turno")).toBeInTheDocument();

        act(() => setLocale("en-US"));

        expect(screen.getByRole("dialog", { name: "Why for turn 1" })).toBeInTheDocument();
        expect(screen.getByText("Turn summary")).toBeInTheDocument();
        expect(screen.getByText("Tools called")).toBeInTheDocument();
    });
});
