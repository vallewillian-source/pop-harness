
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { GuardChangesProvider } from "../components/GuardChanges";
import { setLocale } from "../i18n";
import { AgentChatTest } from "./AgentChatTest";

const ORG = "org_1";
const AGENTE = "ag_1";
const BASE = `/admin/v1/orgs/${ORG}/agents/${AGENTE}/test-chat`;

type RouteEntry = {
    method: string;
    path: string;
    respond: () => Response | Promise<Response>;
};

let routes: RouteEntry[] = [];
let readConversation1: () => Response = () => json(200, historyConversation1());
let readsConversation1 = 0;

function json(status: number, body: unknown): Response {
    return new Response(JSON.stringify(body), {
        status,
        headers: { "content-type": "application/json" },
    });
}

function conversa(id: string, partial: Record<string, unknown> = {}) {
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
        ...partial,
    };
}

function mensagem(
    id: string,
    direction: "in" | "out",
    texto: string,
    created: number,
    turno: string | null = null,
) {
    return {
        id,
        conversation_id: "conv_1",
        direction,
        role: direction === "in" ? "user" : "assistant",
        text: texto,
        attachments: [],
        turn_id: turno,
        created_at: created,
    };
}

const TURN_DONE = {
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

const TURN_FAILED = {
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

function historyConversation1() {
    return {
        conversation: conversa("conv_1", { message_count: 3 }),
        messages: [
            mensagem("m1", "in", "qual o prazo?", 1_000),
            mensagem("m2", "out", "Recebi: o prazo é de 5 dias úteis.", 1_800, "turno-1"),
            mensagem("m3", "in", "e para outra cidade?", 2_000),
        ],
        turns: [TURN_DONE, TURN_FAILED],
    };
}

function servidor() {
    routes = [
        { method: "GET", path: `/admin/v1/orgs/${ORG}`, respond: () => json(200, { id: ORG, name: "Clínica" }) },
        {
            method: "GET",
            path: `/admin/v1/orgs/${ORG}/agents/${AGENTE}`,
            respond: () =>
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
            method: "GET",
            path: BASE,
            respond: () =>
                json(200, {
                    items: [conversa("conv_1", { message_count: 3 }), conversa("conv_2", { status: "transferred" })],
                    next_cursor: null,
                }),
        },
        {
            method: "GET",
            path: `${BASE}/conv_1`,
            respond: () => {
                readsConversation1 += 1;
                return readConversation1();
            },
        },
        {
            method: "GET",
            path: `${BASE}/conv_2`,
            respond: () => json(200, { conversation: conversa("conv_2"), messages: [], turns: [] }),
        },
    ];
}

function installServer() {
    vi.stubGlobal(
        "fetch",
        vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
            const url = typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
            const path = url.split("?")[0] ?? url;
            const method = (init?.method ?? "GET").toUpperCase();
            const route = routes.find((candidate) => candidate.method === method && candidate.path === path);
            if (!route) throw new Error(`sem rota falsa para ${method} ${url}`);
            return route.respond();
        }),
    );
}

function build() {
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
    readConversation1 = () => json(200, historyConversation1());
    readsConversation1 = 0;
    servidor();
    installServer();
});

afterEach(() => {
    vi.unstubAllGlobals();
});

describe("abrir o por quê a partir da conversa", () => {
    it("o botão da resposta abre o painel com o turno gravado e relê a conversa", async () => {
        const user = userEvent.setup();
        build();
        const button = await screen.findByRole("button", { name: "Ver por quê (turno 1)" });
        const readsBefore = readsConversation1;

        await user.click(button);

        const dialog = await screen.findByRole("dialog", { name: "Por quê do turno 1" });
        expect(within(dialog).getByText("Prazo de entrega")).toBeInTheDocument();
        expect(within(dialog).getByText("Respondida pelo Jev")).toBeInTheDocument();
        expect(within(dialog).getByText("consultar_base")).toBeInTheDocument();
        await waitFor(() => expect(readsConversation1).toBeGreaterThan(readsBefore));
    });

    it("um turno que falhou aparece no registro com o erro e abre o painel dele", async () => {
        const user = userEvent.setup();
        build();
        await screen.findByText("e para outra cidade?");

        const row = screen.getByText("Turno #2 · Falhou").closest("div") as HTMLElement;
        expect(within(row).getByText("chamada ao LLM cancelada")).toBeInTheDocument();

        await user.click(within(row).getByRole("button", { name: "Ver por quê (turno 2)" }));

        const dialog = await screen.findByRole("dialog", { name: "Por quê do turno 2" });
        expect(within(dialog).getByText("Nenhum gate decidiu neste turno.")).toBeInTheDocument();
        expect(within(dialog).getAllByText("chamada ao LLM cancelada").length).toBeGreaterThan(0);
    });

    it("Esc fecha e o foco volta ao botão que abriu", async () => {
        const user = userEvent.setup();
        build();
        const button = await screen.findByRole("button", { name: "Ver por quê (turno 1)" });
        await user.click(button);
        await screen.findByRole("dialog");

        await user.keyboard("{Escape}");

        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
        await waitFor(() => expect(button).toHaveFocus());
    });
});

describe("leitura e troca de conversa", () => {
    it("falha ao reler a conversa mostra o erro no painel e Tentar de novo recarrega", async () => {
        const user = userEvent.setup();
        build();
        const button = await screen.findByRole("button", { name: "Ver por quê (turno 1)" });

        readConversation1 = () => json(500, { code: "internal", message: "falha ao ler o histórico", details: null });
        await user.click(button);

        const dialog = await screen.findByRole("dialog", { name: "Por quê do turno 1" });
        expect(await within(dialog).findByText("Não consegui atualizar os dados deste turno")).toBeInTheDocument();
        expect(within(dialog).getByText("Prazo de entrega")).toBeInTheDocument();

        readConversation1 = () => json(200, historyConversation1());
        await user.click(within(dialog).getByRole("button", { name: "Tentar de novo" }));

        await waitFor(() =>
            expect(within(screen.getByRole("dialog")).queryByText("Não consegui atualizar os dados deste turno")).not.toBeInTheDocument(),
        );
    });

    it("o botão Fechar do painel fecha e devolve o foco ao botão que abriu", async () => {
        const user = userEvent.setup();
        build();
        const button = await screen.findByRole("button", { name: "Ver por quê (turno 1)" });
        await user.click(button);
        const dialog = await screen.findByRole("dialog");

        await user.click(within(dialog).getByRole("button", { name: "Fechar o painel do por quê" }));

        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
        await waitFor(() => expect(button).toHaveFocus());
    });
});

describe("I18N-10: o por quê da conversa em en-US", () => {
    afterEach(() => {
        act(() => setLocale("pt-BR"));
    });

    it("o botão, o painel e o turno falho falam inglês", async () => {
        setLocale("en-US");
        const user = userEvent.setup();
        build();

        const button = await screen.findByRole("button", { name: "See why (turn 1)" });
        await user.click(button);

        const dialog = await screen.findByRole("dialog", { name: "Why for turn 1" });
        expect(within(dialog).getByText("Prazo de entrega")).toBeInTheDocument();
        expect(within(dialog).getByText("Answered by Jev")).toBeInTheDocument();
        expect(within(dialog).getByText("consultar_base")).toBeInTheDocument();
        expect(within(dialog).getByText("Turn summary")).toBeInTheDocument();

        await user.keyboard("{Escape}");
        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());

        const row = screen.getByText("Turn #2 · Failed").closest("div") as HTMLElement;
        expect(within(row).getByText("chamada ao LLM cancelada")).toBeInTheDocument();
        await user.click(within(row).getByRole("button", { name: "See why (turn 2)" }));

        const dialogTwo = await screen.findByRole("dialog", { name: "Why for turn 2" });
        expect(within(dialogTwo).getByText("No gate decided in this turn.")).toBeInTheDocument();
    });

    it("erro de leitura e Tentar de novo aparecem em inglês", async () => {
        setLocale("en-US");
        const user = userEvent.setup();
        build();
        const button = await screen.findByRole("button", { name: "See why (turn 1)" });

        readConversation1 = () => json(500, { code: "internal", message: "falha ao ler o histórico", details: null });
        await user.click(button);

        const dialog = await screen.findByRole("dialog", { name: "Why for turn 1" });
        expect(await within(dialog).findByText("Could not refresh this turn's data")).toBeInTheDocument();
        expect(within(dialog).getByRole("button", { name: "Try again" })).toBeInTheDocument();

        readConversation1 = () => json(200, historyConversation1());
        await user.click(within(dialog).getByRole("button", { name: "Try again" }));
        await waitFor(() =>
            expect(within(screen.getByRole("dialog")).queryByText("Could not refresh this turn's data")).not.toBeInTheDocument(),
        );

        await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Close the why panel" }));
        await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    });
});
