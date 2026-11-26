/**
 * Chat de teste com o "por quê" por turno (HR-47), com `fetch` falso.
 *
 * O que importa na tela: o botão aparece na resposta do agente com o número do turno; um turno que
 * falhou (sem mensagem de resposta) aparece no registro com o erro; abrir o painel relê a conversa;
 * erro dessa leitura aparece no painel com "Tentar de novo"; trocar de conversa fecha o painel.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { GuardChangesProvider } from "../components/GuardChanges";
import { setLocale } from "../i18n";
import { AgentChatTest } from "./AgenteChatTeste";

const ORG = "org_1";
const AGENTE = "ag_1";
const BASE = `/admin/v1/orgs/${ORG}/agents/${AGENTE}/test-chat`;

type Rota = {
    metodo: string;
    caminho: string;
    responder: () => Response | Promise<Response>;
};

let rotas: Rota[] = [];
/** Leitura da conversa 1: o teste troca o corpo ou faz falhar para simular a recarga. */
let leituraConversa1: () => Response = () => json(200, historicoConversa1());
let leiturasConversa1 = 0;

function json(status: number, corpo: unknown): Response {
    return new Response(JSON.stringify(corpo), {
        status,
        headers: { "content-type": "application/json" },
    });
}

function conversa(id: string, parcial: Record<string, unknown> = {}) {
    return {
        id,
        org_id: ORG,
        agent_id: AGENTE,
        channel: "test",
        status: "open",
        summary: null,
        version: 1,
        message_count: 0,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
        last_message_at: null,
        ...parcial,
    };
}

function mensagem(
    id: string,
    direcao: "in" | "out",
    texto: string,
    criadoEm: number,
    turno: string | null = null,
) {
    return {
        id,
        conversation_id: "conv_1",
        direction: direcao,
        role: direcao === "in" ? "user" : "assistant",
        text: texto,
        attachments: [],
        turn_id: turno,
        created_at: criadoEm,
    };
}

const TURNO_FEITO = {
    id: "turno-1",
    state: "done",
    jev: {
        chamada: { perguntas: 1, decididas_por_cache: 0, ms: 300, tokens: 200, usage: {}, falha: null },
        decisoes: [
            {
                gate: "g3",
                escolha: "base",
                escolha_do_jev: "base",
                origem: "jev",
                confianca: 0.9,
                margem: 0.5,
                motivo: null,
                ms: 300,
                tokens: 200,
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
            similaridade: 0.8,
            relevancia: 0.02,
        },
    ],
    tools: ["consultar_base"],
    timings: { fila_ms: 1, montagem_ms: 20, primeiro_texto_ms: 300, total_ms: 600, rodadas_ferramenta: 1, modelo: "modelo-teste", motivo_fim: "concluido" },
    input_tokens: 100,
    output_tokens: 20,
    cost_micros: null,
    error: null,
    started_at: 1_500,
    finished_at: 2_100,
};

const TURNO_FALHO = {
    id: "turno-2",
    state: "failed",
    jev: null,
    retrieved: [],
    tools: [],
    timings: null,
    input_tokens: null,
    output_tokens: null,
    cost_micros: null,
    error: "chamada ao LLM cancelada",
    started_at: 2_100,
    finished_at: 2_300,
};

function historicoConversa1() {
    return {
        conversation: conversa("conv_1", { message_count: 3 }),
        messages: [
            mensagem("m1", "in", "qual o prazo?", 1_000),
            mensagem("m2", "out", "Recebi: o prazo é de 5 dias úteis.", 1_800, "turno-1"),
            mensagem("m3", "in", "e para outra cidade?", 2_000),
        ],
        turns: [TURNO_FEITO, TURNO_FALHO],
    };
}

function servidor() {
    rotas = [
        { metodo: "GET", caminho: `/admin/v1/orgs/${ORG}`, responder: () => json(200, { id: ORG, name: "Clínica" }) },
        {
            metodo: "GET",
            caminho: `/admin/v1/orgs/${ORG}/agents/${AGENTE}`,
            responder: () =>
                json(200, {
                    id: AGENTE,
                    org_id: ORG,
                    name: "Beatriz",
                    slug: "beatriz",
                    description: "",
                    status: "active",
                    default_department_id: null,
                    human_handoff_enabled: false,
                    human_handoff_department_id: null,
                    version: 1,
                    created_at: 1_700_000_000_000,
                    updated_at: 1_700_000_000_000,
                }),
        },
        {
            metodo: "GET",
            caminho: BASE,
            responder: () =>
                json(200, {
                    items: [conversa("conv_1", { message_count: 3 }), conversa("conv_2", { status: "transferred" })],
                    next_cursor: null,
                }),
        },
        {
            metodo: "GET",
            caminho: `${BASE}/conv_1`,
            responder: () => {
                leiturasConversa1 += 1;
                return leituraConversa1();
            },
        },
        {
            metodo: "GET",
            caminho: `${BASE}/conv_2`,
            responder: () => json(200, { conversation: conversa("conv_2"), messages: [], turns: [] }),
        },
    ];
}

function instalarServidor() {
    vi.stubGlobal(
        "fetch",
        vi.fn(async (entrada: RequestInfo | URL, init?: RequestInit) => {
            const url = typeof entrada === "string" ? entrada : entrada instanceof URL ? entrada.toString() : entrada.url;
            const caminho = url.split("?")[0] ?? url;
            const metodo = (init?.method ?? "GET").toUpperCase();
            const rota = rotas.find((candidata) => candidata.metodo === metodo && candidata.caminho === caminho);
            if (!rota) throw new Error(`sem rota falsa para ${metodo} ${url}`);
            return rota.responder();
        }),
    );
}

function montar() {
    return render(
        <MemoryRouter initialEntries={[`/orgs/${ORG}/agents/${AGENTE}/test-chat`]}>
            <GuardChangesProvider>
                <Routes>
                    <Route path="/orgs/:orgId/agents/:agentId/test-chat" element={<AgentChatTest />} />
                </Routes>
            </GuardChangesProvider>
        </MemoryRouter>,
    );
}

beforeEach(() => {
    window.sessionStorage.clear();
    leituraConversa1 = () => json(200, historicoConversa1());
    leiturasConversa1 = 0;
    servidor();
    instalarServidor();
});

afterEach(() => {
    vi.unstubAllGlobals();
});

describe("abrir o por quê a partir da conversa", () => {
    it("o botão da resposta abre o painel com o turno gravado e relê a conversa", async () => {
        const usuario = userEvent.setup();
        montar();
        const botao = await screen.findByRole("button", { name: "Ver por quê (turno 1)" });
        const leiturasAntes = leiturasConversa1;

        await usuario.click(botao);

        const dialogo = await screen.findByRole("dialog", { name: "Por quê do turno 1" });
        expect(within(dialogo).getByText("Prazo de entrega")).toBeInTheDocument();
        expect(within(dialogo).getByText("Respondida pelo Jev")).toBeInTheDocument();
        expect(within(dialogo).getByText("consultar_base")).toBeInTheDocument();
        await waitFor(() => expect(leiturasConversa1).toBeGreaterThan(leiturasAntes));
    });

    it("um turno que falhou aparece no registro com o erro e abre o painel dele", async () => {
        const usuario = userEvent.setup();
        montar();
        await screen.findByText("e para outra cidade?");

        const linha = screen.getByText("Turno #2 · Falhou").closest("div") as HTMLElement;
        expect(within(linha).getByText("chamada ao LLM cancelada")).toBeInTheDocument();

        await usuario.click(within(linha).getByRole("button", { name: "Ver por quê (turno 2)" }));

        const dialogo = await screen.findByRole("dialog", { name: "Por quê do turno 2" });
        expect(within(dialogo).getByText("Nenhum gate decidiu neste turno.")).toBeInTheDocument();
        expect(within(dialogo).getAllByText("chamada ao LLM cancelada").length).toBeGreaterThan(0);
    });

    it("Esc fecha e o foco volta ao botão que abriu", async () => {
        const usuario = userEvent.setup();
        montar();
        const botao = await screen.findByRole("button", { name: "Ver por quê (turno 1)" });
        await usuario.click(botao);
        await screen.findByRole("dialog");

        await usuario.keyboard("{Escape}");

        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
        await waitFor(() => expect(botao).toHaveFocus());
    });
});

describe("leitura e troca de conversa", () => {
    it("falha ao reler a conversa mostra o erro no painel e Tentar de novo recarrega", async () => {
        const usuario = userEvent.setup();
        montar();
        const botao = await screen.findByRole("button", { name: "Ver por quê (turno 1)" });

        leituraConversa1 = () => json(500, { code: "internal", message: "falha ao ler o histórico", details: null });
        await usuario.click(botao);

        const dialogo = await screen.findByRole("dialog", { name: "Por quê do turno 1" });
        expect(await within(dialogo).findByText("Não consegui atualizar os dados deste turno")).toBeInTheDocument();
        expect(within(dialogo).getByText("Prazo de entrega")).toBeInTheDocument();

        leituraConversa1 = () => json(200, historicoConversa1());
        await usuario.click(within(dialogo).getByRole("button", { name: "Tentar de novo" }));

        await waitFor(() =>
            expect(within(screen.getByRole("dialog")).queryByText("Não consegui atualizar os dados deste turno")).not.toBeInTheDocument(),
        );
    });

    it("o botão Fechar do painel fecha e devolve o foco ao botão que abriu", async () => {
        const usuario = userEvent.setup();
        montar();
        const botao = await screen.findByRole("button", { name: "Ver por quê (turno 1)" });
        await usuario.click(botao);
        const dialogo = await screen.findByRole("dialog");

        // Com o painel aberto, o resto da página fica inerte (é um diálogo modal): a conversa segue
        // visível por trás, mas não recebe cliques até o painel fechar.
        await usuario.click(within(dialogo).getByRole("button", { name: "Fechar o painel do por quê" }));

        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
        await waitFor(() => expect(botao).toHaveFocus());
    });
});

describe("I18N-10: o por quê da conversa em en-US", () => {
    afterEach(() => {
        act(() => setLocale("pt-BR"));
    });

    it("o botão, o painel e o turno falho falam inglês", async () => {
        setLocale("en-US");
        const usuario = userEvent.setup();
        montar();

        const botao = await screen.findByRole("button", { name: "See why (turn 1)" });
        await usuario.click(botao);

        const dialogo = await screen.findByRole("dialog", { name: "Why for turn 1" });
        expect(within(dialogo).getByText("Prazo de entrega")).toBeInTheDocument();
        expect(within(dialogo).getByText("Answered by Jev")).toBeInTheDocument();
        expect(within(dialogo).getByText("consultar_base")).toBeInTheDocument();
        expect(within(dialogo).getByText("Turn summary")).toBeInTheDocument();

        await usuario.keyboard("{Escape}");
        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());

        const linha = screen.getByText("Turn #2 · Failed").closest("div") as HTMLElement;
        expect(within(linha).getByText("chamada ao LLM cancelada")).toBeInTheDocument();
        await usuario.click(within(linha).getByRole("button", { name: "See why (turn 2)" }));

        const dialogoDois = await screen.findByRole("dialog", { name: "Why for turn 2" });
        expect(within(dialogoDois).getByText("No gate decided in this turn.")).toBeInTheDocument();
    });

    it("erro de leitura e Tentar de novo aparecem em inglês", async () => {
        setLocale("en-US");
        const usuario = userEvent.setup();
        montar();
        const botao = await screen.findByRole("button", { name: "See why (turn 1)" });

        leituraConversa1 = () => json(500, { code: "internal", message: "falha ao ler o histórico", details: null });
        await usuario.click(botao);

        const dialogo = await screen.findByRole("dialog", { name: "Why for turn 1" });
        expect(await within(dialogo).findByText("Could not refresh this turn's data")).toBeInTheDocument();
        expect(within(dialogo).getByRole("button", { name: "Try again" })).toBeInTheDocument();

        leituraConversa1 = () => json(200, historicoConversa1());
        await usuario.click(within(dialogo).getByRole("button", { name: "Try again" }));
        await waitFor(() =>
            expect(within(screen.getByRole("dialog")).queryByText("Could not refresh this turn's data")).not.toBeInTheDocument(),
        );

        await usuario.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Close the why panel" }));
        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    });
});
