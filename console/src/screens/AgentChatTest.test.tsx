
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { GuardChangesProvider } from "../components/GuardChanges";
import { setToken } from "../api/client";
import { setLocale } from "../i18n";
import { AgentChatTest } from "./AgentChatTest";

const ORG = "org_1";
const AGENTE = "ag_1";
const BASE = `/admin/v1/orgs/${ORG}/agents/${AGENTE}/test-chat`;

type Order = {
    method: string;
    caminho: string;
    headers: Headers;
    body: unknown;
    signal?: AbortSignal | null;
};

type RouteEntry = {
    method: string;
    caminho: string;
    respond: (order: Order) => Response | Promise<Response>;
};

const codificador = new TextEncoder();
let orders: Order[] = [];
let routes: RouteEntry[] = [];

function json(status: number, body: unknown): Response {
    return new Response(body === undefined ? null : JSON.stringify(body), {
        status,
        headers: { "content-type": "application/json" },
    });
}

function error(status: number, code: string, message: string, details: unknown = null): Response {
    return json(status, { code, message, details });
}

function frame(evento: string, dados: unknown): string {
    return `event: ${evento}\ndata: ${JSON.stringify(dados)}\n\n`;
}

function turno(pieces: string[], options: { opened?: boolean; signal?: AbortSignal | null } = {}): Response {
    const body = new ReadableStream<Uint8Array>({
        start(control) {
            for (const piece of pieces) control.enqueue(codificador.encode(piece));
            if (!options.opened) {
                control.close();
                return;
            }
            options.signal?.addEventListener("abort", () => {
                control.error(new DOMException("cancelado pelo operador", "AbortError"));
            });
        },
    });
    return new Response(body, { status: 200, headers: { "content-type": "text/event-stream" } });
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

function mensagem(id: string, direction: "in" | "out", texto: string, conversationId: string) {
    return {
        id,
        conversation_id: conversationId,
        direction,
        role: direction === "in" ? "user" : "assistant",
        text: texto,
        attachments: [],
        turn_id: null,
        created_at: 1_700_000_000_000,
    };
}

function transcript(conv: ReturnType<typeof conversa>, messages: unknown[] = []) {
    return { conversation: { ...conv, message_count: messages.length }, messages, turns: [] };
}

const AGENT_JSON = {
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
};

function serverDefault() {
    const c1 = conversa("conv_1", { message_count: 2 });
    const c2 = conversa("conv_2", { status: "transferred", version: 3 });
    const historyC1 = [
        mensagem("m1", "in", "qual o prazo?", "conv_1"),
        mensagem("m2", "out", "Recebi: o prazo é de 5 dias úteis.", "conv_1"),
    ];
    routes = [
        { method: "GET", caminho: `/admin/v1/orgs/${ORG}`, respond: () => json(200, { id: ORG, name: "Clínica Teste" }) },
        { method: "GET", caminho: `/admin/v1/orgs/${ORG}/agents/${AGENTE}`, respond: () => json(200, AGENT_JSON) },
        {
            method: "GET",
            caminho: BASE,
            respond: () => json(200, { items: [c1, c2], next_cursor: null }),
        },
        {
            method: "GET",
            caminho: `${BASE}/conv_1`,
            respond: () => json(200, transcript(c1, historyC1)),
        },
        {
            method: "GET",
            caminho: `${BASE}/conv_2`,
            respond: () =>
                json(200, transcript(c2, [mensagem("m9", "in", "quero falar com alguém", "conv_2")])),
        },
        {
            method: "POST",
            caminho: `${BASE}/conv_1/messages`,
            respond: () =>
                turno([
                    frame("mensagem", { conversation_id: "conv_1", message_id: "m3", text: "", attachments: [] }),
                    frame("inicio", { turn_id: "t1", agent_id: AGENTE, conversation_id: "conv_1" }),
                    frame("texto", { delta: "Recebi: " }),
                    frame("texto", { delta: "o prazo é de 5 dias úteis." }),
                    frame("fim", {
                        turn_id: "t1",
                        state: "done",
                        reason: null,
                        text: "Recebi: o prazo é de 5 dias úteis.",
                        error: null,
                        input_tokens: 120,
                        output_tokens: 16,
                        timings: { total_ms: 83 },
                    }),
                ]),
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
            let body: unknown = init?.body;
            if (typeof init?.body === "string") {
                try {
                    body = JSON.parse(init.body);
                } catch {
                    body = init.body;
                }
            }
            const order: Order = {
                method,
                caminho: path,
                headers: new Headers(init?.headers),
                body,
                signal: init?.signal,
            };
            orders.push(order);
            const route = routes.find((candidate) => candidate.method === method && candidate.caminho === path);
            if (!route) throw new Error(`sem rota falsa para ${method} ${url}`);
            return route.respond(order);
        }),
    );
}

function calls(method: string, path: string): Order[] {
    return orders.filter((order) => order.method === method && order.caminho === path);
}

function build() {
    return render(
        <MemoryRouter initialEntries={[`/orgs/${ORG}/agents/${AGENTE}/test-chat`]}>
            <GuardChangesProvider>
                <Routes>
                    <Route path="/orgs/:orgId/agents/:agentId/test-chat" element={<AgentChatTest />} />
                    <Route path="/orgs/:orgId/agents" element={<div>Tela de agentes</div>} />
                </Routes>
            </GuardChangesProvider>
        </MemoryRouter>,
    );
}

function conversationItem(id: string): HTMLElement {
    return within(screen.getByRole("list", { name: "Conversas" })).getByText(id).closest("button") as HTMLElement;
}

function fieldMessage() {
    return screen.getByRole("textbox", { name: "Mensagem do cliente" });
}

beforeEach(() => {
    orders = [];
    window.sessionStorage.clear();
    serverDefault();
    installServer();
});

afterEach(() => {
    vi.unstubAllGlobals();
});

describe("lista e conversa", () => {
    it("lista as conversas, abre a mais recente e mostra o histórico", async () => {
        build();

        expect(await screen.findByText("qual o prazo?")).toBeInTheDocument();
        expect(screen.getByText("Recebi: o prazo é de 5 dias úteis.")).toBeInTheDocument();
        expect(screen.getByRole("list", { name: "Conversas" })).toBeInTheDocument();
        expect(screen.getAllByText("Ativa").length).toBeGreaterThan(0);
        expect(screen.getAllByText("Transferida").length).toBeGreaterThan(0);
    });

    it("nova conversa cria com memória zerada e abre a conversa vazia", async () => {
        const user = userEvent.setup();
        build();
        await screen.findByText("qual o prazo?");

        routes.push({
            method: "POST",
            caminho: BASE,
            respond: () => json(201, conversa("conv_novo")),
        });
        routes.push({
            method: "GET",
            caminho: `${BASE}/conv_novo`,
            respond: () => json(200, transcript(conversa("conv_novo"), [])),
        });

        await user.click(screen.getAllByRole("button", { name: "Nova conversa" })[0]!);

        expect(await screen.findByText("Esta conversa ainda não tem mensagens. Escreva a primeira abaixo.")).toBeInTheDocument();
        expect(calls("POST", BASE)).toHaveLength(1);
    });
});

describe("envio e streaming", () => {
    it("envia com Enter, mostra o texto que chega em ordem e fecha com o fim do turno", async () => {
        const user = userEvent.setup();
        build();
        await screen.findByText("qual o prazo?");

        await user.click(fieldMessage());
        await user.type(fieldMessage(), "qual o prazo de entrega?{Enter}");

        await waitFor(() => expect(screen.getByText("Turno concluído · entrada 120 tokens · saída 16 tokens · 83 ms")).toBeInTheDocument());
        const send = calls("POST", `${BASE}/conv_1/messages`)[0]!;
        expect(send.body).toBeInstanceOf(FormData);
        expect((send.body as FormData).get("text")).toBe("qual o prazo de entrega?");
        expect(fieldMessage()).toHaveValue("");
    });

    it("manda o arquivo no FormData como `file`, sem texto quando o campo está vazio", async () => {
        const user = userEvent.setup();
        build();
        await screen.findByText("qual o prazo?");

        const file = new File(["%PDF-1.4"], "procedimento.pdf", { type: "application/pdf" });
        const selector = document.getElementById("chat-teste-arquivos") as HTMLInputElement;
        await user.upload(selector, file);
        expect(screen.getByRole("button", { name: "Tirar procedimento.pdf da mensagem" })).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Enviar" }));

        await waitFor(() => expect(calls("POST", `${BASE}/conv_1/messages`)).toHaveLength(1));
        const body = calls("POST", `${BASE}/conv_1/messages`)[0]!.body as FormData;
        const sent = body.get("file") as File;
        expect(sent.name).toBe("procedimento.pdf");
        expect(body.get("text")).toBeNull();
    });

    it("recusa envio sem texto e sem arquivo, sem chamar a API", async () => {
        const user = userEvent.setup();
        build();
        await screen.findByText("qual o prazo?");

        await user.click(screen.getByRole("button", { name: "Enviar" }));

        expect(await screen.findByText("Escreva uma mensagem ou anexe um arquivo.")).toBeInTheDocument();
        expect(calls("POST", `${BASE}/conv_1/messages`)).toHaveLength(0);
    });

    it("arquivo acima de 16 MB é recusado no campo, sem chamar a API", async () => {
        const user = userEvent.setup();
        build();
        await screen.findByText("qual o prazo?");

        const large = new File(["x"], "gigante.pdf", { type: "application/pdf" });
        Object.defineProperty(large, "size", { value: 17 * 1024 * 1024 });
        const selector = document.getElementById("chat-teste-arquivos") as HTMLInputElement;
        await user.upload(selector, large);

        expect(await screen.findByText(/gigante\.pdf tem 17,0 MB: o limite é 16 MB/)).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Tirar gigante.pdf da mensagem" })).toBeNull();
        expect(calls("POST", `${BASE}/conv_1/messages`)).toHaveLength(0);
    });

    it("409 explica que já há turno em andamento e devolve o texto digitado", async () => {
        const user = userEvent.setup();
        build();
        await screen.findByText("qual o prazo?");
        routes.unshift({
            method: "POST",
            caminho: `${BASE}/conv_1/messages`,
            respond: () =>
                error(409, "turno_em_andamento", "esta conversa já tem um turno em andamento"),
        });

        await user.type(fieldMessage(), "segunda aba");
        await user.click(screen.getByRole("button", { name: "Enviar" }));

        expect(await screen.findByText("Esta conversa já tem uma resposta em andamento")).toBeInTheDocument();
        expect(fieldMessage()).toHaveValue("segunda aba");
    });

    it("503 explica que o turno não está ligado e não grava a mensagem", async () => {
        const user = userEvent.setup();
        build();
        await screen.findByText("qual o prazo?");
        routes.unshift({
            method: "POST",
            caminho: `${BASE}/conv_1/messages`,
            respond: () => error(503, "turno_indisponivel", "o turno não está ligado neste servidor"),
        });

        await user.type(fieldMessage(), "oi");
        await user.click(screen.getByRole("button", { name: "Enviar" }));

        expect(await screen.findByText("O turno não está ligado neste servidor")).toBeInTheDocument();
        expect(screen.getByText(/Sua mensagem não foi gravada/)).toBeInTheDocument();
        expect(fieldMessage()).toHaveValue("oi");
    });

    it("400 com details.field = file aparece no campo de arquivo", async () => {
        const user = userEvent.setup();
        build();
        await screen.findByText("qual o prazo?");
        routes.unshift({
            method: "POST",
            caminho: `${BASE}/conv_1/messages`,
            respond: () =>
                error(400, "validation_error", "o arquivo não é um tipo aceito", { field: "file" }),
        });

        const file = new File(["x"], "contrato.docx", {
            type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        });
        const selector = document.getElementById("chat-teste-arquivos") as HTMLInputElement;
        await user.upload(selector, file);
        await user.click(screen.getByRole("button", { name: "Enviar" }));

        expect(await screen.findByRole("alert")).toHaveTextContent("o arquivo não é um tipo aceito");
        expect(screen.getByRole("button", { name: "Tirar contrato.docx da mensagem" })).toBeInTheDocument();
    });
});

describe("streaming, cancelamento e troca de conversa", () => {
    it("cancelar aborta o turno, avisa e volta a aceitar mensagem", async () => {
        const user = userEvent.setup();
        build();
        await screen.findByText("qual o prazo?");
        routes.unshift({
            method: "POST",
            caminho: `${BASE}/conv_1/messages`,
            respond: (order) =>
                turno(
                    [
                        frame("mensagem", { conversation_id: "conv_1", message_id: "m3", text: "segure", attachments: [] }),
                        frame("texto", { delta: "TRECHO-AO-VIVO" }),
                    ],
                    { opened: true, signal: order.signal },
                ),
        });

        await user.type(fieldMessage(), "segure{Enter}");
        expect(await screen.findByText(/TRECHO-AO-VIVO/)).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Cancelar turno em andamento" }));

        expect(await screen.findByText("Turno cancelado")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Enviar" })).toBeInTheDocument();
        expect(fieldMessage()).not.toBeDisabled();
    });

    it("trocar de conversa no meio do turno não mistura as duas", async () => {
        const user = userEvent.setup();
        build();
        await screen.findByText("qual o prazo?");
        routes.unshift({
            method: "POST",
            caminho: `${BASE}/conv_1/messages`,
            respond: (order) =>
                turno(
                    [
                        frame("mensagem", { conversation_id: "conv_1", message_id: "m3", text: "segure", attachments: [] }),
                        frame("texto", { delta: "TEXTO-DA-CONVERSA-UM" }),
                    ],
                    { opened: true, signal: order.signal },
                ),
        });

        await user.type(fieldMessage(), "segure{Enter}");
        expect(await screen.findByText(/TEXTO-DA-CONVERSA-UM/)).toBeInTheDocument();

        await user.click(conversationItem("conv_2"));

        expect(await screen.findByText("quero falar com alguém")).toBeInTheDocument();
        expect(screen.queryByText(/TEXTO-DA-CONVERSA-UM/)).toBeNull();
        expect(screen.queryByText("segure")).toBeNull();
        expect(calls("GET", `${BASE}/conv_2`).length).toBeGreaterThan(0);
    });
});

describe("conversa transferida", () => {
    it("bloqueia o envio e reabre com If-Match da versão lida", async () => {
        const user = userEvent.setup();
        build();
        await screen.findByText("qual o prazo?");

        await user.click(conversationItem("conv_2"));
        expect(await screen.findByText("Atendimento transferido para uma pessoa")).toBeInTheDocument();
        expect(fieldMessage()).toBeDisabled();

        routes.unshift({
            method: "POST",
            caminho: `${BASE}/conv_2/reopen`,
            respond: () => json(200, conversa("conv_2", { status: "open", version: 4 })),
        });
        routes.unshift({
            method: "GET",
            caminho: `${BASE}/conv_2`,
            respond: () => json(200, transcript(conversa("conv_2", { status: "open", version: 4 }), [])),
        });

        await user.click(screen.getByRole("button", { name: "Reabrir atendimento da IA nesta conversa" }));

        await waitFor(() => expect(screen.queryByText("Atendimento transferido para uma pessoa")).toBeNull());
        const reopen = calls("POST", `${BASE}/conv_2/reopen`)[0]!;
        expect(reopen.headers.get("if-match")).toBe("3");
        expect(fieldMessage()).not.toBeDisabled();
    });

    it("412 ao reabrir relê a conversa, mostra o aviso e usa a versão nova na próxima tentativa", async () => {
        const user = userEvent.setup();
        build();
        await screen.findByText("qual o prazo?");
        await user.click(conversationItem("conv_2"));
        await screen.findByText("Atendimento transferido para uma pessoa");

        let tentativas = 0;
        routes.unshift({
            method: "POST",
            caminho: `${BASE}/conv_2/reopen`,
            respond: () => {
                tentativas += 1;
                if (tentativas === 1) {
                    return error(412, "version_conflict", "versão desatualizada", {
                        expected_version: 3,
                        current_version: 5,
                    });
                }
                return json(200, conversa("conv_2", { status: "open", version: 6 }));
            },
        });
        routes.unshift({
            method: "GET",
            caminho: `${BASE}/conv_2`,
            respond: () =>
                json(200, transcript(conversa("conv_2", { status: "transferred", version: 5 }), [])),
        });

        await user.click(screen.getByRole("button", { name: "Reabrir atendimento da IA nesta conversa" }));
        expect(await screen.findByText("A conversa mudou em outro lugar")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Reabrir atendimento da IA nesta conversa" })).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Reabrir atendimento da IA nesta conversa" }));
        await waitFor(() => expect(calls("POST", `${BASE}/conv_2/reopen`)).toHaveLength(2));
        expect(calls("POST", `${BASE}/conv_2/reopen`)[1]!.headers.get("if-match")).toBe("5");
    });
});

describe("estados", () => {
    it("falha ao ler o agente mostra o erro com tentar de novo", async () => {
        routes = routes.map((route) =>
            route.caminho === `/admin/v1/orgs/${ORG}/agents/${AGENTE}`
                ? { ...route, respond: () => error(500, "internal", "o servidor não respondeu") }
                : route,
        );
        build();

        expect(await screen.findByText("Não consegui carregar o chat de teste")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
    });
});

const PNG = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]);
const PDF = new TextEncoder().encode("%PDF-1.4 teste");

function conteudo(bytes: Uint8Array, mime: string, layout: string): Response {
    return new Response(bytes.buffer as ArrayBuffer, {
        status: 200,
        headers: { "content-type": mime, "content-disposition": layout },
    });
}

const ATTACHMENT_IMAGE = {
    id: "anx_img",
    name: "laudo.png",
    mime: "image/png",
    size_bytes: PNG.length,
    sha256: "a".repeat(64),
    extraction_status: "ok",
    has_text: false,
};
const ATTACHMENT_PDF = {
    id: "anx_pdf",
    name: "procedimento.pdf",
    mime: "application/pdf",
    size_bytes: PDF.length,
    sha256: "b".repeat(64),
    extraction_status: "ok",
    has_text: true,
};
const ATTACHMENT_AGENT = {
    id: "anx_ag",
    name: "tabela.xlsx",
    mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    size_bytes: 2048,
    sha256: "c".repeat(64),
    extraction_status: "ok",
    has_text: false,
};

const REPORT_PATH = `${BASE}/conv_1/messages/m1/attachments/anx_img/content`;
const PDF_PATH = `${BASE}/conv_1/messages/m1/attachments/anx_pdf/content`;

describe("arquivos do cliente: ver e baixar (HR-46c)", () => {
    let reportResponse: () => Response | Promise<Response>;
    let createdUrls: string[];
    let releasedUrls: string[];
    let clicks: { download: string; href: string }[];
    const createOriginal = URL.createObjectURL;
    const releaseOriginal = URL.revokeObjectURL;

    beforeEach(() => {
        createdUrls = [];
        releasedUrls = [];
        clicks = [];
        reportResponse = () => conteudo(PNG, "image/png", 'inline; filename="laudo.png"');

        let counter = 0;
        URL.createObjectURL = vi.fn(() => {
            counter += 1;
            const url = `blob:teste/${counter}`;
            createdUrls.push(url);
            return url;
        });
        URL.revokeObjectURL = vi.fn((url: string) => {
            releasedUrls.push(url);
        });
        vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (this: HTMLAnchorElement) {
            clicks.push({ download: this.download, href: this.href });
        });

        routes = routes.map((route) =>
            route.method === "GET" && route.caminho === `${BASE}/conv_1`
                ? {
                      ...route,
                      respond: () =>
                          json(
                              200,
                              transcript(conversa("conv_1", { message_count: 2 }), [
                                  {
                                      ...mensagem("m1", "in", "segue o laudo e o procedimento", "conv_1"),
                                      attachments: [ATTACHMENT_IMAGE, ATTACHMENT_PDF],
                                  },
                                  {
                                      ...mensagem("m2", "out", "Recebi os dois.", "conv_1"),
                                      attachments: [ATTACHMENT_AGENT],
                                  },
                              ]),
                          ),
                  }
                : route,
        );
        routes.push(
            {
                method: "GET",
                caminho: REPORT_PATH,
                respond: () => reportResponse(),
            },
            {
                method: "GET",
                caminho: PDF_PATH,
                respond: () =>
                    conteudo(
                        PDF,
                        "application/pdf",
                        "attachment; filename=\"procedimento.pdf\"; filename*=UTF-8''Procedimento%20de%20entrega%20(v2).pdf",
                    ),
            },
        );
    });

    afterEach(() => {
        setToken(null);
        vi.restoreAllMocks();
    });

    afterAll(() => {
        URL.createObjectURL = createOriginal;
        URL.revokeObjectURL = releaseOriginal;
    });

    it("Ver abre a imagem em data:, busca com o token da sessão e não oferece ação no anexo do agente", async () => {
        setToken("token_da_sessao");
        const user = userEvent.setup();
        build();

        await user.click(await screen.findByRole("button", { name: "Ver laudo.png" }));
        const image = await screen.findByRole("img", { name: "Imagem enviada pelo cliente: laudo.png" });
        expect(image.getAttribute("src")).toMatch(/^data:image\/png;base64,iVBORw0KGgo=$/);

        const order = calls("GET", REPORT_PATH)[0]!;
        expect(order.headers.get("authorization")).toBe("Bearer token_da_sessao");

        expect(screen.queryByRole("button", { name: "Ver procedimento.pdf" })).not.toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Baixar procedimento.pdf" })).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Baixar tabela.xlsx" })).not.toBeInTheDocument();
        expect(createdUrls).toHaveLength(0);
    });

    it("Baixar PDF usa o nome do Content-Disposition e solta a URL do download depois", async () => {
        const user = userEvent.setup();
        build();

        await user.click(await screen.findByRole("button", { name: "Baixar procedimento.pdf" }));
        await waitFor(() => expect(clicks).toHaveLength(1));
        expect(clicks[0]!.download).toBe("Procedimento de entrega (v2).pdf");
        expect(clicks[0]!.href).toBe("blob:teste/1");
        expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
        expect(releasedUrls).toHaveLength(0);
    });

    it("404 mostra o motivo com Tentar de novo, sem derrubar a conversa", async () => {
        reportResponse = () => error(404, "not_found", "o conteúdo deste arquivo não está disponível");
        const user = userEvent.setup();
        build();

        await user.click(await screen.findByRole("button", { name: "Baixar laudo.png" }));
        expect(await screen.findByText("o conteúdo deste arquivo não está disponível")).toBeInTheDocument();
        expect(screen.getByText("segue o laudo e o procedimento")).toBeInTheDocument();

        reportResponse = () => conteudo(PNG, "image/png", 'attachment; filename="laudo.png"');
        await user.click(screen.getByRole("button", { name: "Tentar de novo: baixar laudo.png" }));
        await waitFor(() => expect(clicks).toHaveLength(1));
        expect(clicks[0]!.download).toBe("laudo.png");
        expect(screen.queryByText("o conteúdo deste arquivo não está disponível")).not.toBeInTheDocument();
    });

    it("403 de outra org aparece na prévia com Tentar de novo", async () => {
        reportResponse = () => error(403, "forbidden", "este token é de outra org");
        const user = userEvent.setup();
        build();

        await user.click(await screen.findByRole("button", { name: "Ver laudo.png" }));
        expect(await screen.findByText("este token é de outra org")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
    });

    it("Esc fecha a prévia e devolve o foco ao botão Ver", async () => {
        const user = userEvent.setup();
        build();

        const buttonView = await screen.findByRole("button", { name: "Ver laudo.png" });
        await user.click(buttonView);
        await screen.findByRole("img", { name: "Imagem enviada pelo cliente: laudo.png" });

        await user.keyboard("{Escape}");
        await waitFor(() =>
            expect(screen.queryByRole("img", { name: "Imagem enviada pelo cliente: laudo.png" })).not.toBeInTheDocument(),
        );
        await waitFor(() => expect(buttonView).toHaveFocus());
    });

    it("a prévia abre como janela (diálogo) com o nome do arquivo e a imagem dentro dela", async () => {
        const user = userEvent.setup();
        build();

        await user.click(await screen.findByRole("button", { name: "Ver laudo.png" }));
        const dialog = await screen.findByRole("dialog", { name: "Imagem: laudo.png" });
        expect(within(dialog).getByRole("img", { name: "Imagem enviada pelo cliente: laudo.png" })).toBeInTheDocument();
    });

    it("resposta que chega depois de fechar a prévia não reabre a imagem", async () => {
        let liberar: () => void = () => undefined;
        reportResponse = () =>
            new Promise<Response>((resolver) => {
                liberar = () => resolver(conteudo(PNG, "image/png", 'inline; filename="laudo.png"'));
            });
        const user = userEvent.setup();
        build();

        await user.click(await screen.findByRole("button", { name: "Ver laudo.png" }));
        expect(await screen.findByText("Carregando a imagem…")).toBeInTheDocument();
        await user.keyboard("{Escape}");
        await waitFor(() => expect(screen.queryByText("Carregando a imagem…")).not.toBeInTheDocument());

        liberar();
        await new Promise((resolver) => setTimeout(resolver, 30));
        expect(screen.queryByRole("img", { name: "Imagem enviada pelo cliente: laudo.png" })).not.toBeInTheDocument();
    });

    it("trocar de conversa fecha a prévia e solta a URL de download pendente", async () => {
        const user = userEvent.setup();
        build();

        await user.click(await screen.findByRole("button", { name: "Baixar laudo.png" }));
        await waitFor(() => expect(clicks).toHaveLength(1));
        expect(releasedUrls).toHaveLength(0);

        await user.click(conversationItem("conv_2"));
        await waitFor(() => expect(releasedUrls).toContain("blob:teste/1"));
    });

    it("sair da tela solta as URLs de download pendentes", async () => {
        const user = userEvent.setup();
        const { unmount } = build();

        await user.click(await screen.findByRole("button", { name: "Baixar laudo.png" }));
        await waitFor(() => expect(clicks).toHaveLength(1));
        unmount();
        expect(releasedUrls).toContain("blob:teste/1");
    });
});

describe("arquivos que o agente enviou no turno (HR-46e)", () => {
    const AGENT_PDF_PATH = `/admin/v1/orgs/${ORG}/agents/${AGENTE}/files/arq_pdf/content`;
    const AGENT_PNG_PATH = `/admin/v1/orgs/${ORG}/agents/${AGENTE}/files/arq_png/content`;
    const PATH_ONLY_AGENT = `/admin/v1/orgs/${ORG}/agents/${AGENTE}/files/arq_so/content`;

    const send = (id: string, file: string, nome: string, mime: string, estado: string, turno: string) => ({
        id,
        turn_id: turno,
        seq: 1,
        arquivo_id: file,
        name: nome,
        mime,
        size_bytes: 568,
        sha256: "c2b07337",
        state: estado,
        created_at: 1_700_000_000_000,
    });

    const SEND_PDF = send("env_pdf", "arq_pdf", "Catalogo.pdf", "application/pdf", "sent", "t_ag");
    const SEND_PNG = send("env_png", "arq_png", "tabela.png", "image/png", "sent", "t_ag");
    const SEND_ONLY = send("env_so", "arq_so", "Relatorio.pdf", "application/pdf", "failed", "t_sem");

    const turno = (id: string, files: unknown[], partial: Record<string, unknown> = {}) => ({
        id,
        state: "done",
        jev: null,
        retrieved: [],
        tools: files.length > 0 ? ["enviar_arquivo"] : [],
        timings: { total_ms: 80 },
        input_tokens: 120,
        output_tokens: 16,
        cost_micros: 0,
        error: null,
        files_out: files,
        started_at: 1_699_999_999_000,
        finished_at: 1_699_999_999_100,
        ...partial,
    });

    let pdfResponse: () => Response | Promise<Response>;
    let createdUrls: string[];
    let releasedUrls: string[];
    let clicks: { download: string; href: string }[];
    const createOriginal = URL.createObjectURL;
    const releaseOriginal = URL.revokeObjectURL;

    function respondHistory(turns: unknown[], messages: unknown[]) {
        routes = routes.map((route) =>
            route.method === "GET" && route.caminho === `${BASE}/conv_1`
                ? {
                      ...route,
                      respond: () =>
                          json(200, {
                              conversation: conversa("conv_1", { message_count: messages.length }),
                              messages,
                              turns,
                          }),
                  }
                : route,
        );
    }

    const messagesDefault = () => [
        mensagem("m1", "in", "manda o catálogo e a tabela", "conv_1"),
        { ...mensagem("m2", "out", "Enviei os arquivos.", "conv_1"), turn_id: "t_ag" },
    ];

    beforeEach(() => {
        createdUrls = [];
        releasedUrls = [];
        clicks = [];
        pdfResponse = () =>
            conteudo(
                PDF,
                "application/pdf",
                "attachment; filename=\"Catalogo.pdf\"; filename*=UTF-8''Cat%C3%A1logo%20de%20servi%C3%A7os.pdf",
            );

        let counter = 0;
        URL.createObjectURL = vi.fn(() => {
            counter += 1;
            const url = `blob:teste/${counter}`;
            createdUrls.push(url);
            return url;
        });
        URL.revokeObjectURL = vi.fn((url: string) => {
            releasedUrls.push(url);
        });
        vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (this: HTMLAnchorElement) {
            clicks.push({ download: this.download, href: this.href });
        });

        respondHistory(
            [turno("t_ag", [SEND_PDF, SEND_PNG]), turno("t_sem", [SEND_ONLY], { started_at: 1_700_000_001_000 })],
            messagesDefault(),
        );
        routes.push(
            { method: "GET", caminho: AGENT_PDF_PATH, respond: () => pdfResponse() },
            {
                method: "GET",
                caminho: AGENT_PNG_PATH,
                respond: () => conteudo(PNG, "image/png", 'inline; filename="tabela.png"'),
            },
            {
                method: "GET",
                caminho: PATH_ONLY_AGENT,
                respond: () => conteudo(PDF, "application/pdf", 'attachment; filename="Relatorio.pdf"'),
            },
        );
    });

    afterEach(() => {
        setToken(null);
        vi.restoreAllMocks();
    });

    afterAll(() => {
        URL.createObjectURL = createOriginal;
        URL.revokeObjectURL = releaseOriginal;
    });

    it("mostra os arquivos no turno do agente e baixa pela biblioteca, com o token e o nome do cabeçalho", async () => {
        setToken("token_da_sessao");
        const user = userEvent.setup();
        build();

        const lists = await screen.findAllByRole("list", { name: "Arquivos enviados pelo agente" });
        expect(lists).toHaveLength(2);
        expect(within(lists[0]!).getAllByText("enviado")).toHaveLength(2);
        expect(within(lists[1]!).getByText("falhou ao enviar")).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Baixar Catalogo.pdf" }));
        await waitFor(() => expect(clicks).toHaveLength(1));
        expect(clicks[0]!.download).toBe("Catálogo de serviços.pdf");

        const order = calls("GET", AGENT_PDF_PATH)[0]!;
        expect(order.headers.get("authorization")).toBe("Bearer token_da_sessao");
        expect(screen.queryByRole("button", { name: "Ver Catalogo.pdf" })).not.toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Ver tabela.png" })).toBeInTheDocument();
    });

    it("imagem enviada pelo agente abre a prévia com o texto de quem enviou", async () => {
        const user = userEvent.setup();
        build();

        await user.click(await screen.findByRole("button", { name: "Ver tabela.png" }));
        const image = await screen.findByRole("img", { name: "Imagem enviada pelo agente: tabela.png" });
        expect(image.getAttribute("src")).toMatch(/^data:image\/png;base64,iVBORw0KGgo=$/);
        expect(calls("GET", AGENT_PNG_PATH)).toHaveLength(1);
    });

    it("envio que falhou aparece rotulado, sem esconder o download, num turno sem texto do agente", async () => {
        const user = userEvent.setup();
        build();

        expect(await screen.findByText("falhou ao enviar")).toBeInTheDocument();
        expect(screen.getByText("O turno terminou sem texto de resposta.")).toBeInTheDocument();
        const download = screen.getByRole("button", { name: "Baixar Relatorio.pdf" });
        expect(download).toBeEnabled();

        await user.click(download);
        await waitFor(() => expect(clicks).toHaveLength(1));
        expect(clicks[0]!.download).toBe("Relatorio.pdf");
    });

    it("turno sem arquivos não mostra lista de arquivos", async () => {
        respondHistory([turno("t_ag", [])], messagesDefault());
        build();

        expect(await screen.findByText("Enviei os arquivos.")).toBeInTheDocument();
        expect(screen.queryByRole("list", { name: "Arquivos enviados pelo agente" })).not.toBeInTheDocument();
    });

    it("404 ao baixar o arquivo do agente mostra o motivo com Tentar de novo, sem derrubar a conversa", async () => {
        pdfResponse = () => error(404, "not_found", "o conteúdo deste arquivo não está disponível");
        const user = userEvent.setup();
        build();

        await user.click(await screen.findByRole("button", { name: "Baixar Catalogo.pdf" }));
        expect(await screen.findByText("o conteúdo deste arquivo não está disponível")).toBeInTheDocument();
        expect(screen.getByText("Enviei os arquivos.")).toBeInTheDocument();

        pdfResponse = () =>
            conteudo(PDF, "application/pdf", 'attachment; filename="Catalogo.pdf"');
        await user.click(screen.getByRole("button", { name: "Tentar de novo: baixar Catalogo.pdf" }));
        await waitFor(() => expect(clicks).toHaveLength(1));
        expect(screen.queryByText("o conteúdo deste arquivo não está disponível")).not.toBeInTheDocument();
    });

    it("trocar de conversa solta a URL do download do arquivo do agente", async () => {
        const user = userEvent.setup();
        build();

        await user.click(await screen.findByRole("button", { name: "Baixar Catalogo.pdf" }));
        await waitFor(() => expect(clicks).toHaveLength(1));
        expect(releasedUrls).toHaveLength(0);

        await user.click(conversationItem("conv_2"));
        await waitFor(() => expect(releasedUrls).toContain("blob:teste/1"));
    });
});

describe("I18N-10: chat de teste em en-US", () => {
    afterEach(() => {
        act(() => setLocale("pt-BR"));
    });

    it("a lista, a conversa e o envio falam inglês", async () => {
        setLocale("en-US");
        const user = userEvent.setup();
        build();

        expect(await screen.findByText("qual o prazo?")).toBeInTheDocument();
        expect(screen.getByRole("list", { name: "Conversations" })).toBeInTheDocument();
        expect(screen.getAllByText("Active").length).toBeGreaterThan(0);
        expect(screen.getAllByText("Transferred").length).toBeGreaterThan(0);
        expect(screen.getByRole("button", { name: "New conversation" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Refresh" })).toBeInTheDocument();
        expect(screen.getByRole("textbox", { name: "Customer message" })).toBeInTheDocument();
        expect(screen.getByText("Attach files")).toBeInTheDocument();

        await user.type(screen.getByRole("textbox", { name: "Customer message" }), "prazo?{Enter}");

        await waitFor(() =>
            expect(
                screen.getByText("Turn completed · input 120 tokens · output 16 tokens · 83 ms"),
            ).toBeInTheDocument(),
        );
    });

    it("a validação local e o arquivo grande respondem em inglês", async () => {
        setLocale("en-US");
        const user = userEvent.setup();
        build();
        await screen.findByText("qual o prazo?");

        await user.click(screen.getByRole("button", { name: "Send" }));
        expect(await screen.findByText("Write a message or attach a file.")).toBeInTheDocument();

        const large = new File(["x"], "huge.pdf", { type: "application/pdf" });
        Object.defineProperty(large, "size", { value: 17 * 1024 * 1024 });
        const selector = document.getElementById("chat-teste-arquivos") as HTMLInputElement;
        await user.upload(selector, large);

        expect(await screen.findByText(/huge\.pdf is 17\.0 MB: the limit is 16 MB/)).toBeInTheDocument();
    });

    it("a conversa transferida e o estado vazio aparecem em inglês", async () => {
        setLocale("en-US");
        const user = userEvent.setup();
        build();
        await screen.findByText("qual o prazo?");

        await user.click(within(screen.getByRole("list", { name: "Conversations" })).getByText("conv_2").closest("button") as HTMLElement);

        expect(await screen.findByText("Conversation transferred to a person")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Reopen AI service in this conversation" })).toBeInTheDocument();
    });

    it("a troca para inglês reescreve a conversa aberta sem recarregar", async () => {
        build();
        await screen.findByText("qual o prazo?");
        expect(screen.getByRole("button", { name: "Nova conversa" })).toBeInTheDocument();
        expect(screen.getByRole("textbox", { name: "Mensagem do cliente" })).toBeInTheDocument();

        act(() => setLocale("en-US"));

        expect(screen.getByRole("button", { name: "New conversation" })).toBeInTheDocument();
        expect(screen.getByRole("textbox", { name: "Customer message" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Send" })).toBeInTheDocument();
    });
});
