/**
 * Testes da aba Chat de teste (HR-46), com `fetch` falso que entrega o turno em pedaços.
 *
 * O que importa: lista e nova conversa com memória zerada; texto que cresce em ordem e fecha no
 * `fim`; anexo indo no `FormData`; `409` e `503` explicados sem perder o que foi digitado; a conversa
 * transferida bloqueia o envio e a reabertura manda `If-Match`; `412` relê e não perde a tela;
 * cancelar aborta e a tela volta a aceitar mensagem; trocar de conversa no meio do turno não mistura
 * as duas; arquivo acima de 16 MB é recusado sem chamar a API.
 */

import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { GuardChangesProvider } from "../components/GuardChanges";
import { setToken } from "../api/client";
import { setLocale } from "../i18n";
import { AgentChatTest } from "./AgenteChatTeste";

const ORG = "org_1";
const AGENTE = "ag_1";
const BASE = `/admin/v1/orgs/${ORG}/agents/${AGENTE}/test-chat`;

type Pedido = {
    metodo: string;
    caminho: string;
    cabecalhos: Headers;
    corpo: unknown;
    sinal?: AbortSignal | null;
};

type Rota = {
    metodo: string;
    caminho: string;
    responder: (pedido: Pedido) => Response | Promise<Response>;
};

const codificador = new TextEncoder();
let pedidos: Pedido[] = [];
let rotas: Rota[] = [];

function json(status: number, corpo: unknown): Response {
    return new Response(corpo === undefined ? null : JSON.stringify(corpo), {
        status,
        headers: { "content-type": "application/json" },
    });
}

function erro(status: number, code: string, message: string, details: unknown = null): Response {
    return json(status, { code, message, details });
}

/** Um quadro do `text/event-stream`, como o servidor escreve. */
function quadro(evento: string, dados: unknown): string {
    return `event: ${evento}\ndata: ${JSON.stringify(dados)}\n\n`;
}

/**
 * Turno em streaming. `pedacos` vão primeiro; com `aberto`, o fluxo fica no ar até o aborto (o sinal
 * do pedido), que é o que o servidor vê como queda da conexão.
 */
function turno(pedacos: string[], opcoes: { aberto?: boolean; sinal?: AbortSignal | null } = {}): Response {
    const corpo = new ReadableStream<Uint8Array>({
        start(controle) {
            for (const pedaco of pedacos) controle.enqueue(codificador.encode(pedaco));
            if (!opcoes.aberto) {
                controle.close();
                return;
            }
            opcoes.sinal?.addEventListener("abort", () => {
                controle.error(new DOMException("cancelado pelo operador", "AbortError"));
            });
        },
    });
    return new Response(corpo, { status: 200, headers: { "content-type": "text/event-stream" } });
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

function mensagem(id: string, direcao: "in" | "out", texto: string, conversaId: string) {
    return {
        id,
        conversation_id: conversaId,
        direction: direcao,
        role: direcao === "in" ? "user" : "assistant",
        text: texto,
        attachments: [],
        turn_id: null,
        created_at: 1_700_000_000_000,
    };
}

function historico(conv: ReturnType<typeof conversa>, mensagens: unknown[] = []) {
    return { conversation: { ...conv, message_count: mensagens.length }, messages: mensagens, turns: [] };
}

const AGENTE_JSON = {
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

/** Estado base do servidor: duas conversas (uma ativa, uma transferida) e o histórico de cada. */
function servidorPadrao() {
    const c1 = conversa("conv_1", { message_count: 2 });
    const c2 = conversa("conv_2", { status: "transferred", version: 3 });
    const historicoC1 = [
        mensagem("m1", "in", "qual o prazo?", "conv_1"),
        mensagem("m2", "out", "Recebi: o prazo é de 5 dias úteis.", "conv_1"),
    ];
    rotas = [
        { metodo: "GET", caminho: `/admin/v1/orgs/${ORG}`, responder: () => json(200, { id: ORG, name: "Clínica Teste" }) },
        { metodo: "GET", caminho: `/admin/v1/orgs/${ORG}/agents/${AGENTE}`, responder: () => json(200, AGENTE_JSON) },
        {
            metodo: "GET",
            caminho: BASE,
            responder: () => json(200, { items: [c1, c2], next_cursor: null }),
        },
        {
            metodo: "GET",
            caminho: `${BASE}/conv_1`,
            responder: () => json(200, historico(c1, historicoC1)),
        },
        {
            metodo: "GET",
            caminho: `${BASE}/conv_2`,
            responder: () =>
                json(200, historico(c2, [mensagem("m9", "in", "quero falar com alguém", "conv_2")])),
        },
        {
            metodo: "POST",
            caminho: `${BASE}/conv_1/messages`,
            responder: () =>
                turno([
                    quadro("mensagem", { conversation_id: "conv_1", message_id: "m3", text: "", attachments: [] }),
                    quadro("inicio", { turn_id: "t1", agent_id: AGENTE, conversation_id: "conv_1" }),
                    quadro("texto", { delta: "Recebi: " }),
                    quadro("texto", { delta: "o prazo é de 5 dias úteis." }),
                    quadro("fim", {
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

/** Instala o `fetch` falso: cada pedido casa com a primeira rota de mesmo método e caminho. */
function instalarServidor() {
    vi.stubGlobal(
        "fetch",
        vi.fn(async (entrada: RequestInfo | URL, init?: RequestInit) => {
            const url = typeof entrada === "string" ? entrada : entrada instanceof URL ? entrada.toString() : entrada.url;
            const caminho = url.split("?")[0] ?? url;
            const metodo = (init?.method ?? "GET").toUpperCase();
            let corpo: unknown = init?.body;
            if (typeof init?.body === "string") {
                try {
                    corpo = JSON.parse(init.body);
                } catch {
                    corpo = init.body;
                }
            }
            const pedido: Pedido = {
                metodo,
                caminho,
                cabecalhos: new Headers(init?.headers),
                corpo,
                sinal: init?.signal,
            };
            pedidos.push(pedido);
            const rota = rotas.find((candidata) => candidata.metodo === metodo && candidata.caminho === caminho);
            if (!rota) throw new Error(`sem rota falsa para ${metodo} ${url}`);
            return rota.responder(pedido);
        }),
    );
}

function chamadas(metodo: string, caminho: string): Pedido[] {
    return pedidos.filter((pedido) => pedido.metodo === metodo && pedido.caminho === caminho);
}

function montar() {
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

/** Botão da conversa na lista lateral, pelo id curto que aparece nela. */
function itemDaConversa(id: string): HTMLElement {
    return within(screen.getByRole("list", { name: "Conversas" })).getByText(id).closest("button") as HTMLElement;
}

/** Campo de mensagem e botão de envio, pela forma como o operador os encontra. */
function campoDeMensagem() {
    return screen.getByRole("textbox", { name: "Mensagem do cliente" });
}

beforeEach(() => {
    pedidos = [];
    window.sessionStorage.clear();
    servidorPadrao();
    instalarServidor();
});

afterEach(() => {
    vi.unstubAllGlobals();
});

describe("lista e conversa", () => {
    it("lista as conversas, abre a mais recente e mostra o histórico", async () => {
        montar();

        expect(await screen.findByText("qual o prazo?")).toBeInTheDocument();
        expect(screen.getByText("Recebi: o prazo é de 5 dias úteis.")).toBeInTheDocument();
        expect(screen.getByRole("list", { name: "Conversas" })).toBeInTheDocument();
        expect(screen.getAllByText("Ativa").length).toBeGreaterThan(0);
        expect(screen.getAllByText("Transferida").length).toBeGreaterThan(0);
    });

    it("nova conversa cria com memória zerada e abre a conversa vazia", async () => {
        const usuario = userEvent.setup();
        montar();
        await screen.findByText("qual o prazo?");

        rotas.push({
            metodo: "POST",
            caminho: BASE,
            responder: () => json(201, conversa("conv_novo")),
        });
        rotas.push({
            metodo: "GET",
            caminho: `${BASE}/conv_novo`,
            responder: () => json(200, historico(conversa("conv_novo"), [])),
        });

        await usuario.click(screen.getAllByRole("button", { name: "Nova conversa" })[0]!);

        expect(await screen.findByText("Esta conversa ainda não tem mensagens. Escreva a primeira abaixo.")).toBeInTheDocument();
        expect(chamadas("POST", BASE)).toHaveLength(1);
    });
});

describe("envio e streaming", () => {
    it("envia com Enter, mostra o texto que chega em ordem e fecha com o fim do turno", async () => {
        const usuario = userEvent.setup();
        montar();
        await screen.findByText("qual o prazo?");

        await usuario.click(campoDeMensagem());
        await usuario.type(campoDeMensagem(), "qual o prazo de entrega?{Enter}");

        // O histórico é relido no fim do turno: a resposta gravada aparece e o rodapé confirma os números.
        await waitFor(() => expect(screen.getByText("Turno concluído · entrada 120 tokens · saída 16 tokens · 83 ms")).toBeInTheDocument());
        const envio = chamadas("POST", `${BASE}/conv_1/messages`)[0]!;
        expect(envio.corpo).toBeInstanceOf(FormData);
        expect((envio.corpo as FormData).get("text")).toBe("qual o prazo de entrega?");
        expect(campoDeMensagem()).toHaveValue("");
    });

    it("manda o arquivo no FormData como `file`, sem texto quando o campo está vazio", async () => {
        const usuario = userEvent.setup();
        montar();
        await screen.findByText("qual o prazo?");

        const arquivo = new File(["%PDF-1.4"], "procedimento.pdf", { type: "application/pdf" });
        const seletor = document.getElementById("chat-teste-arquivos") as HTMLInputElement;
        await usuario.upload(seletor, arquivo);
        expect(screen.getByRole("button", { name: "Tirar procedimento.pdf da mensagem" })).toBeInTheDocument();

        await usuario.click(screen.getByRole("button", { name: "Enviar" }));

        await waitFor(() => expect(chamadas("POST", `${BASE}/conv_1/messages`)).toHaveLength(1));
        const corpo = chamadas("POST", `${BASE}/conv_1/messages`)[0]!.corpo as FormData;
        const enviado = corpo.get("file") as File;
        expect(enviado.name).toBe("procedimento.pdf");
        expect(corpo.get("text")).toBeNull();
    });

    it("recusa envio sem texto e sem arquivo, sem chamar a API", async () => {
        const usuario = userEvent.setup();
        montar();
        await screen.findByText("qual o prazo?");

        await usuario.click(screen.getByRole("button", { name: "Enviar" }));

        expect(await screen.findByText("Escreva uma mensagem ou anexe um arquivo.")).toBeInTheDocument();
        expect(chamadas("POST", `${BASE}/conv_1/messages`)).toHaveLength(0);
    });

    it("arquivo acima de 16 MB é recusado no campo, sem chamar a API", async () => {
        const usuario = userEvent.setup();
        montar();
        await screen.findByText("qual o prazo?");

        const grande = new File(["x"], "gigante.pdf", { type: "application/pdf" });
        Object.defineProperty(grande, "size", { value: 17 * 1024 * 1024 });
        const seletor = document.getElementById("chat-teste-arquivos") as HTMLInputElement;
        await usuario.upload(seletor, grande);

        expect(await screen.findByText(/gigante\.pdf tem 17,0 MB: o limite é 16 MB/)).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Tirar gigante.pdf da mensagem" })).toBeNull();
        expect(chamadas("POST", `${BASE}/conv_1/messages`)).toHaveLength(0);
    });

    it("409 explica que já há turno em andamento e devolve o texto digitado", async () => {
        const usuario = userEvent.setup();
        montar();
        await screen.findByText("qual o prazo?");
        rotas.unshift({
            metodo: "POST",
            caminho: `${BASE}/conv_1/messages`,
            responder: () =>
                erro(409, "turno_em_andamento", "esta conversa já tem um turno em andamento"),
        });

        await usuario.type(campoDeMensagem(), "segunda aba");
        await usuario.click(screen.getByRole("button", { name: "Enviar" }));

        expect(await screen.findByText("Esta conversa já tem uma resposta em andamento")).toBeInTheDocument();
        expect(campoDeMensagem()).toHaveValue("segunda aba");
    });

    it("503 explica que o turno não está ligado e não grava a mensagem", async () => {
        const usuario = userEvent.setup();
        montar();
        await screen.findByText("qual o prazo?");
        rotas.unshift({
            metodo: "POST",
            caminho: `${BASE}/conv_1/messages`,
            responder: () => erro(503, "turno_indisponivel", "o turno não está ligado neste servidor"),
        });

        await usuario.type(campoDeMensagem(), "oi");
        await usuario.click(screen.getByRole("button", { name: "Enviar" }));

        expect(await screen.findByText("O turno não está ligado neste servidor")).toBeInTheDocument();
        expect(screen.getByText(/Sua mensagem não foi gravada/)).toBeInTheDocument();
        expect(campoDeMensagem()).toHaveValue("oi");
    });

    it("400 com details.field = file aparece no campo de arquivo", async () => {
        const usuario = userEvent.setup();
        montar();
        await screen.findByText("qual o prazo?");
        rotas.unshift({
            metodo: "POST",
            caminho: `${BASE}/conv_1/messages`,
            responder: () =>
                erro(400, "validation_error", "o arquivo não é um tipo aceito", { field: "file" }),
        });

        const arquivo = new File(["x"], "contrato.docx", {
            type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        });
        const seletor = document.getElementById("chat-teste-arquivos") as HTMLInputElement;
        await usuario.upload(seletor, arquivo);
        await usuario.click(screen.getByRole("button", { name: "Enviar" }));

        expect(await screen.findByRole("alert")).toHaveTextContent("o arquivo não é um tipo aceito");
        expect(screen.getByRole("button", { name: "Tirar contrato.docx da mensagem" })).toBeInTheDocument();
    });
});

describe("streaming, cancelamento e troca de conversa", () => {
    it("cancelar aborta o turno, avisa e volta a aceitar mensagem", async () => {
        const usuario = userEvent.setup();
        montar();
        await screen.findByText("qual o prazo?");
        rotas.unshift({
            metodo: "POST",
            caminho: `${BASE}/conv_1/messages`,
            responder: (pedido) =>
                turno(
                    [
                        quadro("mensagem", { conversation_id: "conv_1", message_id: "m3", text: "segure", attachments: [] }),
                        quadro("texto", { delta: "TRECHO-AO-VIVO" }),
                    ],
                    { aberto: true, sinal: pedido.sinal },
                ),
        });

        await usuario.type(campoDeMensagem(), "segure{Enter}");
        expect(await screen.findByText(/TRECHO-AO-VIVO/)).toBeInTheDocument();

        await usuario.click(screen.getByRole("button", { name: "Cancelar turno em andamento" }));

        expect(await screen.findByText("Turno cancelado")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Enviar" })).toBeInTheDocument();
        expect(campoDeMensagem()).not.toBeDisabled();
    });

    it("trocar de conversa no meio do turno não mistura as duas", async () => {
        const usuario = userEvent.setup();
        montar();
        await screen.findByText("qual o prazo?");
        rotas.unshift({
            metodo: "POST",
            caminho: `${BASE}/conv_1/messages`,
            responder: (pedido) =>
                turno(
                    [
                        quadro("mensagem", { conversation_id: "conv_1", message_id: "m3", text: "segure", attachments: [] }),
                        quadro("texto", { delta: "TEXTO-DA-CONVERSA-UM" }),
                    ],
                    { aberto: true, sinal: pedido.sinal },
                ),
        });

        await usuario.type(campoDeMensagem(), "segure{Enter}");
        expect(await screen.findByText(/TEXTO-DA-CONVERSA-UM/)).toBeInTheDocument();

        // Troca de conversa: o turno da um é abortado e nada dele aparece na dois.
        await usuario.click(itemDaConversa("conv_2"));

        expect(await screen.findByText("quero falar com alguém")).toBeInTheDocument();
        expect(screen.queryByText(/TEXTO-DA-CONVERSA-UM/)).toBeNull();
        expect(screen.queryByText("segure")).toBeNull();
        expect(chamadas("GET", `${BASE}/conv_2`).length).toBeGreaterThan(0);
    });
});

describe("conversa transferida", () => {
    it("bloqueia o envio e reabre com If-Match da versão lida", async () => {
        const usuario = userEvent.setup();
        montar();
        await screen.findByText("qual o prazo?");

        await usuario.click(itemDaConversa("conv_2"));
        expect(await screen.findByText("Atendimento transferido para uma pessoa")).toBeInTheDocument();
        expect(campoDeMensagem()).toBeDisabled();

        rotas.unshift({
            metodo: "POST",
            caminho: `${BASE}/conv_2/reopen`,
            responder: () => json(200, conversa("conv_2", { status: "open", version: 4 })),
        });
        rotas.unshift({
            metodo: "GET",
            caminho: `${BASE}/conv_2`,
            responder: () => json(200, historico(conversa("conv_2", { status: "open", version: 4 }), [])),
        });

        await usuario.click(screen.getByRole("button", { name: "Reabrir atendimento da IA nesta conversa" }));

        await waitFor(() => expect(screen.queryByText("Atendimento transferido para uma pessoa")).toBeNull());
        const reabrir = chamadas("POST", `${BASE}/conv_2/reopen`)[0]!;
        expect(reabrir.cabecalhos.get("if-match")).toBe("3");
        expect(campoDeMensagem()).not.toBeDisabled();
    });

    it("412 ao reabrir relê a conversa, mostra o aviso e usa a versão nova na próxima tentativa", async () => {
        const usuario = userEvent.setup();
        montar();
        await screen.findByText("qual o prazo?");
        await usuario.click(itemDaConversa("conv_2"));
        await screen.findByText("Atendimento transferido para uma pessoa");

        let tentativas = 0;
        rotas.unshift({
            metodo: "POST",
            caminho: `${BASE}/conv_2/reopen`,
            responder: () => {
                tentativas += 1;
                if (tentativas === 1) {
                    return erro(412, "version_conflict", "versão desatualizada", {
                        expected_version: 3,
                        current_version: 5,
                    });
                }
                return json(200, conversa("conv_2", { status: "open", version: 6 }));
            },
        });
        rotas.unshift({
            metodo: "GET",
            caminho: `${BASE}/conv_2`,
            responder: () =>
                json(200, historico(conversa("conv_2", { status: "transferred", version: 5 }), [])),
        });

        await usuario.click(screen.getByRole("button", { name: "Reabrir atendimento da IA nesta conversa" }));
        expect(await screen.findByText("A conversa mudou em outro lugar")).toBeInTheDocument();
        // A tela continua usável: a conversa segue transferida, com o botão de reabrir.
        expect(screen.getByRole("button", { name: "Reabrir atendimento da IA nesta conversa" })).toBeInTheDocument();

        await usuario.click(screen.getByRole("button", { name: "Reabrir atendimento da IA nesta conversa" }));
        await waitFor(() => expect(chamadas("POST", `${BASE}/conv_2/reopen`)).toHaveLength(2));
        expect(chamadas("POST", `${BASE}/conv_2/reopen`)[1]!.cabecalhos.get("if-match")).toBe("5");
    });
});

describe("estados", () => {
    it("falha ao ler o agente mostra o erro com tentar de novo", async () => {
        rotas = rotas.map((rota) =>
            rota.caminho === `/admin/v1/orgs/${ORG}/agents/${AGENTE}`
                ? { ...rota, responder: () => erro(500, "internal", "o servidor não respondeu") }
                : rota,
        );
        montar();

        expect(await screen.findByText("Não consegui carregar o chat de teste")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
    });
});

const PNG = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]);
const PDF = new TextEncoder().encode("%PDF-1.4 teste");

/** Resposta de conteúdo de arquivo: bytes crus com os cabeçalhos que a API manda. */
function conteudo(bytes: Uint8Array, mime: string, disposicao: string): Response {
    return new Response(bytes.buffer as ArrayBuffer, {
        status: 200,
        headers: { "content-type": mime, "content-disposition": disposicao },
    });
}

const ANEXO_IMAGEM = {
    id: "anx_img",
    name: "laudo.png",
    mime: "image/png",
    size_bytes: PNG.length,
    sha256: "a".repeat(64),
    extraction_status: "ok",
    has_text: false,
};
const ANEXO_PDF = {
    id: "anx_pdf",
    name: "procedimento.pdf",
    mime: "application/pdf",
    size_bytes: PDF.length,
    sha256: "b".repeat(64),
    extraction_status: "ok",
    has_text: true,
};
const ANEXO_DO_AGENTE = {
    id: "anx_ag",
    name: "tabela.xlsx",
    mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    size_bytes: 2048,
    sha256: "c".repeat(64),
    extraction_status: "ok",
    has_text: false,
};

const CAMINHO_DO_LAUDO = `${BASE}/conv_1/messages/m1/attachments/anx_img/content`;
const CAMINHO_DO_PDF = `${BASE}/conv_1/messages/m1/attachments/anx_pdf/content`;

describe("arquivos do cliente: ver e baixar (HR-46c)", () => {
    let respostaDoLaudo: () => Response | Promise<Response>;
    let urlsCriadas: string[];
    let urlsLiberadas: string[];
    let cliques: { download: string; href: string }[];
    const criarOriginal = URL.createObjectURL;
    const liberarOriginal = URL.revokeObjectURL;

    beforeEach(() => {
        urlsCriadas = [];
        urlsLiberadas = [];
        cliques = [];
        respostaDoLaudo = () => conteudo(PNG, "image/png", 'inline; filename="laudo.png"');

        let contador = 0;
        URL.createObjectURL = vi.fn(() => {
            contador += 1;
            const url = `blob:teste/${contador}`;
            urlsCriadas.push(url);
            return url;
        });
        URL.revokeObjectURL = vi.fn((url: string) => {
            urlsLiberadas.push(url);
        });
        vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (this: HTMLAnchorElement) {
            cliques.push({ download: this.download, href: this.href });
        });

        // O histórico de conv_1 traz um anexo do cliente (imagem e PDF) e um do agente (sem ações).
        rotas = rotas.map((rota) =>
            rota.metodo === "GET" && rota.caminho === `${BASE}/conv_1`
                ? {
                      ...rota,
                      responder: () =>
                          json(
                              200,
                              historico(conversa("conv_1", { message_count: 2 }), [
                                  {
                                      ...mensagem("m1", "in", "segue o laudo e o procedimento", "conv_1"),
                                      attachments: [ANEXO_IMAGEM, ANEXO_PDF],
                                  },
                                  {
                                      ...mensagem("m2", "out", "Recebi os dois.", "conv_1"),
                                      attachments: [ANEXO_DO_AGENTE],
                                  },
                              ]),
                          ),
                  }
                : rota,
        );
        rotas.push(
            {
                metodo: "GET",
                caminho: CAMINHO_DO_LAUDO,
                responder: () => respostaDoLaudo(),
            },
            {
                metodo: "GET",
                caminho: CAMINHO_DO_PDF,
                responder: () =>
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

    // Só no fim do arquivo: o desmonte de cada teste (RTL) ainda chama `revokeObjectURL` depois do
    // `afterEach` deste bloco, e o jsdom não tem essas funções de origem.
    afterAll(() => {
        URL.createObjectURL = criarOriginal;
        URL.revokeObjectURL = liberarOriginal;
    });

    it("Ver abre a imagem em data:, busca com o token da sessão e não oferece ação no anexo do agente", async () => {
        setToken("token_da_sessao");
        const usuario = userEvent.setup();
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Ver laudo.png" }));
        const imagem = await screen.findByRole("img", { name: "Imagem enviada pelo cliente: laudo.png" });
        expect(imagem.getAttribute("src")).toMatch(/^data:image\/png;base64,iVBORw0KGgo=$/);

        const pedido = chamadas("GET", CAMINHO_DO_LAUDO)[0]!;
        expect(pedido.cabecalhos.get("authorization")).toBe("Bearer token_da_sessao");

        // PDF não tem "Ver", só "Baixar"; o anexo do agente não tem ação nenhuma.
        expect(screen.queryByRole("button", { name: "Ver procedimento.pdf" })).not.toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Baixar procedimento.pdf" })).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Baixar tabela.xlsx" })).not.toBeInTheDocument();
        expect(urlsCriadas).toHaveLength(0);
    });

    it("Baixar PDF usa o nome do Content-Disposition e solta a URL do download depois", async () => {
        const usuario = userEvent.setup();
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Baixar procedimento.pdf" }));
        await waitFor(() => expect(cliques).toHaveLength(1));
        expect(cliques[0]!.download).toBe("Procedimento de entrega (v2).pdf");
        expect(cliques[0]!.href).toBe("blob:teste/1");
        // Baixar não abre prévia.
        expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
        expect(urlsLiberadas).toHaveLength(0);
    });

    it("404 mostra o motivo com Tentar de novo, sem derrubar a conversa", async () => {
        respostaDoLaudo = () => erro(404, "not_found", "o conteúdo deste arquivo não está disponível");
        const usuario = userEvent.setup();
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Baixar laudo.png" }));
        expect(await screen.findByText("o conteúdo deste arquivo não está disponível")).toBeInTheDocument();
        expect(screen.getByText("segue o laudo e o procedimento")).toBeInTheDocument();

        respostaDoLaudo = () => conteudo(PNG, "image/png", 'attachment; filename="laudo.png"');
        await usuario.click(screen.getByRole("button", { name: "Tentar de novo: baixar laudo.png" }));
        await waitFor(() => expect(cliques).toHaveLength(1));
        expect(cliques[0]!.download).toBe("laudo.png");
        expect(screen.queryByText("o conteúdo deste arquivo não está disponível")).not.toBeInTheDocument();
    });

    it("403 de outra org aparece na prévia com Tentar de novo", async () => {
        respostaDoLaudo = () => erro(403, "forbidden", "este token é de outra org");
        const usuario = userEvent.setup();
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Ver laudo.png" }));
        expect(await screen.findByText("este token é de outra org")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
    });

    it("Esc fecha a prévia e devolve o foco ao botão Ver", async () => {
        const usuario = userEvent.setup();
        montar();

        const botaoVer = await screen.findByRole("button", { name: "Ver laudo.png" });
        await usuario.click(botaoVer);
        await screen.findByRole("img", { name: "Imagem enviada pelo cliente: laudo.png" });

        await usuario.keyboard("{Escape}");
        await waitFor(() =>
            expect(screen.queryByRole("img", { name: "Imagem enviada pelo cliente: laudo.png" })).not.toBeInTheDocument(),
        );
        await waitFor(() => expect(botaoVer).toHaveFocus());
    });

    it("a prévia abre como janela (diálogo) com o nome do arquivo e a imagem dentro dela", async () => {
        const usuario = userEvent.setup();
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Ver laudo.png" }));
        const dialogo = await screen.findByRole("dialog", { name: "Imagem: laudo.png" });
        expect(within(dialogo).getByRole("img", { name: "Imagem enviada pelo cliente: laudo.png" })).toBeInTheDocument();
    });

    it("resposta que chega depois de fechar a prévia não reabre a imagem", async () => {
        let liberar: () => void = () => undefined;
        respostaDoLaudo = () =>
            new Promise<Response>((resolver) => {
                liberar = () => resolver(conteudo(PNG, "image/png", 'inline; filename="laudo.png"'));
            });
        const usuario = userEvent.setup();
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Ver laudo.png" }));
        expect(await screen.findByText("Carregando a imagem…")).toBeInTheDocument();
        await usuario.keyboard("{Escape}");
        await waitFor(() => expect(screen.queryByText("Carregando a imagem…")).not.toBeInTheDocument());

        liberar();
        await new Promise((resolver) => setTimeout(resolver, 30));
        expect(screen.queryByRole("img", { name: "Imagem enviada pelo cliente: laudo.png" })).not.toBeInTheDocument();
    });

    it("trocar de conversa fecha a prévia e solta a URL de download pendente", async () => {
        const usuario = userEvent.setup();
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Baixar laudo.png" }));
        await waitFor(() => expect(cliques).toHaveLength(1));
        expect(urlsLiberadas).toHaveLength(0);

        await usuario.click(itemDaConversa("conv_2"));
        await waitFor(() => expect(urlsLiberadas).toContain("blob:teste/1"));
    });

    it("sair da tela solta as URLs de download pendentes", async () => {
        const usuario = userEvent.setup();
        const { unmount } = montar();

        await usuario.click(await screen.findByRole("button", { name: "Baixar laudo.png" }));
        await waitFor(() => expect(cliques).toHaveLength(1));
        unmount();
        expect(urlsLiberadas).toContain("blob:teste/1");
    });
});

describe("arquivos que o agente enviou no turno (HR-46e)", () => {
    const CAMINHO_PDF_DO_AGENTE = `/admin/v1/orgs/${ORG}/agents/${AGENTE}/files/arq_pdf/content`;
    const CAMINHO_PNG_DO_AGENTE = `/admin/v1/orgs/${ORG}/agents/${AGENTE}/files/arq_png/content`;
    const CAMINHO_SO_DO_AGENTE = `/admin/v1/orgs/${ORG}/agents/${AGENTE}/files/arq_so/content`;

    /** Item de `files_out`, como o histórico devolve (HR-46d). */
    const envio = (id: string, arquivo: string, nome: string, mime: string, estado: string, turno: string) => ({
        id,
        turn_id: turno,
        seq: 1,
        arquivo_id: arquivo,
        name: nome,
        mime,
        size_bytes: 568,
        sha256: "c2b07337",
        state: estado,
        created_at: 1_700_000_000_000,
    });

    const ENVIO_PDF = envio("env_pdf", "arq_pdf", "Catalogo.pdf", "application/pdf", "sent", "t_ag");
    const ENVIO_PNG = envio("env_png", "arq_png", "tabela.png", "image/png", "sent", "t_ag");
    const ENVIO_SO = envio("env_so", "arq_so", "Relatorio.pdf", "application/pdf", "failed", "t_sem");

    const turno = (id: string, arquivos: unknown[], parcial: Record<string, unknown> = {}) => ({
        id,
        state: "done",
        jev: null,
        retrieved: [],
        tools: arquivos.length > 0 ? ["enviar_arquivo"] : [],
        timings: { total_ms: 80 },
        input_tokens: 120,
        output_tokens: 16,
        cost_micros: 0,
        error: null,
        files_out: arquivos,
        started_at: 1_699_999_999_000,
        finished_at: 1_699_999_999_100,
        ...parcial,
    });

    let respostaDoPdf: () => Response | Promise<Response>;
    let urlsCriadas: string[];
    let urlsLiberadas: string[];
    let cliques: { download: string; href: string }[];
    const criarOriginal = URL.createObjectURL;
    const liberarOriginal = URL.revokeObjectURL;

    /** Troca a leitura da conversa 1 por esta: turnos e mensagens que o servidor devolveria. */
    function responderHistorico(turnos: unknown[], mensagens: unknown[]) {
        rotas = rotas.map((rota) =>
            rota.metodo === "GET" && rota.caminho === `${BASE}/conv_1`
                ? {
                      ...rota,
                      responder: () =>
                          json(200, {
                              conversation: conversa("conv_1", { message_count: mensagens.length }),
                              messages: mensagens,
                              turns: turnos,
                          }),
                  }
                : rota,
        );
    }

    const mensagensPadrao = () => [
        mensagem("m1", "in", "manda o catálogo e a tabela", "conv_1"),
        { ...mensagem("m2", "out", "Enviei os arquivos.", "conv_1"), turn_id: "t_ag" },
    ];

    beforeEach(() => {
        urlsCriadas = [];
        urlsLiberadas = [];
        cliques = [];
        respostaDoPdf = () =>
            conteudo(
                PDF,
                "application/pdf",
                "attachment; filename=\"Catalogo.pdf\"; filename*=UTF-8''Cat%C3%A1logo%20de%20servi%C3%A7os.pdf",
            );

        let contador = 0;
        URL.createObjectURL = vi.fn(() => {
            contador += 1;
            const url = `blob:teste/${contador}`;
            urlsCriadas.push(url);
            return url;
        });
        URL.revokeObjectURL = vi.fn((url: string) => {
            urlsLiberadas.push(url);
        });
        vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (this: HTMLAnchorElement) {
            cliques.push({ download: this.download, href: this.href });
        });

        responderHistorico(
            [turno("t_ag", [ENVIO_PDF, ENVIO_PNG]), turno("t_sem", [ENVIO_SO], { started_at: 1_700_000_001_000 })],
            mensagensPadrao(),
        );
        rotas.push(
            { metodo: "GET", caminho: CAMINHO_PDF_DO_AGENTE, responder: () => respostaDoPdf() },
            {
                metodo: "GET",
                caminho: CAMINHO_PNG_DO_AGENTE,
                responder: () => conteudo(PNG, "image/png", 'inline; filename="tabela.png"'),
            },
            {
                metodo: "GET",
                caminho: CAMINHO_SO_DO_AGENTE,
                responder: () => conteudo(PDF, "application/pdf", 'attachment; filename="Relatorio.pdf"'),
            },
        );
    });

    afterEach(() => {
        setToken(null);
        vi.restoreAllMocks();
    });

    afterAll(() => {
        URL.createObjectURL = criarOriginal;
        URL.revokeObjectURL = liberarOriginal;
    });

    it("mostra os arquivos no turno do agente e baixa pela biblioteca, com o token e o nome do cabeçalho", async () => {
        setToken("token_da_sessao");
        const usuario = userEvent.setup();
        montar();

        // Dois turnos do agente com arquivo: a lista aparece uma vez por turno.
        const listas = await screen.findAllByRole("list", { name: "Arquivos enviados pelo agente" });
        expect(listas).toHaveLength(2);
        expect(within(listas[0]!).getAllByText("enviado")).toHaveLength(2);
        expect(within(listas[1]!).getByText("falhou ao enviar")).toBeInTheDocument();

        await usuario.click(screen.getByRole("button", { name: "Baixar Catalogo.pdf" }));
        await waitFor(() => expect(cliques).toHaveLength(1));
        expect(cliques[0]!.download).toBe("Catálogo de serviços.pdf");

        const pedido = chamadas("GET", CAMINHO_PDF_DO_AGENTE)[0]!;
        expect(pedido.cabecalhos.get("authorization")).toBe("Bearer token_da_sessao");
        // PDF não ganha "Ver"; a imagem do agente ganha.
        expect(screen.queryByRole("button", { name: "Ver Catalogo.pdf" })).not.toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Ver tabela.png" })).toBeInTheDocument();
    });

    it("imagem enviada pelo agente abre a prévia com o texto de quem enviou", async () => {
        const usuario = userEvent.setup();
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Ver tabela.png" }));
        const imagem = await screen.findByRole("img", { name: "Imagem enviada pelo agente: tabela.png" });
        expect(imagem.getAttribute("src")).toMatch(/^data:image\/png;base64,iVBORw0KGgo=$/);
        expect(chamadas("GET", CAMINHO_PNG_DO_AGENTE)).toHaveLength(1);
    });

    it("envio que falhou aparece rotulado, sem esconder o download, num turno sem texto do agente", async () => {
        const usuario = userEvent.setup();
        montar();

        expect(await screen.findByText("falhou ao enviar")).toBeInTheDocument();
        expect(screen.getByText("O turno terminou sem texto de resposta.")).toBeInTheDocument();
        const baixar = screen.getByRole("button", { name: "Baixar Relatorio.pdf" });
        expect(baixar).toBeEnabled();

        await usuario.click(baixar);
        await waitFor(() => expect(cliques).toHaveLength(1));
        expect(cliques[0]!.download).toBe("Relatorio.pdf");
    });

    it("turno sem arquivos não mostra lista de arquivos", async () => {
        responderHistorico([turno("t_ag", [])], mensagensPadrao());
        montar();

        expect(await screen.findByText("Enviei os arquivos.")).toBeInTheDocument();
        expect(screen.queryByRole("list", { name: "Arquivos enviados pelo agente" })).not.toBeInTheDocument();
    });

    it("404 ao baixar o arquivo do agente mostra o motivo com Tentar de novo, sem derrubar a conversa", async () => {
        respostaDoPdf = () => erro(404, "not_found", "o conteúdo deste arquivo não está disponível");
        const usuario = userEvent.setup();
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Baixar Catalogo.pdf" }));
        expect(await screen.findByText("o conteúdo deste arquivo não está disponível")).toBeInTheDocument();
        expect(screen.getByText("Enviei os arquivos.")).toBeInTheDocument();

        respostaDoPdf = () =>
            conteudo(PDF, "application/pdf", 'attachment; filename="Catalogo.pdf"');
        await usuario.click(screen.getByRole("button", { name: "Tentar de novo: baixar Catalogo.pdf" }));
        await waitFor(() => expect(cliques).toHaveLength(1));
        expect(screen.queryByText("o conteúdo deste arquivo não está disponível")).not.toBeInTheDocument();
    });

    it("trocar de conversa solta a URL do download do arquivo do agente", async () => {
        const usuario = userEvent.setup();
        montar();

        await usuario.click(await screen.findByRole("button", { name: "Baixar Catalogo.pdf" }));
        await waitFor(() => expect(cliques).toHaveLength(1));
        expect(urlsLiberadas).toHaveLength(0);

        await usuario.click(itemDaConversa("conv_2"));
        await waitFor(() => expect(urlsLiberadas).toContain("blob:teste/1"));
    });
});

describe("I18N-10: chat de teste em en-US", () => {
    afterEach(() => {
        act(() => setLocale("pt-BR"));
    });

    it("a lista, a conversa e o envio falam inglês", async () => {
        setLocale("en-US");
        const usuario = userEvent.setup();
        montar();

        expect(await screen.findByText("qual o prazo?")).toBeInTheDocument();
        expect(screen.getByRole("list", { name: "Conversations" })).toBeInTheDocument();
        expect(screen.getAllByText("Active").length).toBeGreaterThan(0);
        expect(screen.getAllByText("Transferred").length).toBeGreaterThan(0);
        expect(screen.getByRole("button", { name: "New conversation" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Refresh" })).toBeInTheDocument();
        expect(screen.getByRole("textbox", { name: "Customer message" })).toBeInTheDocument();
        expect(screen.getByText("Attach files")).toBeInTheDocument();

        await usuario.type(screen.getByRole("textbox", { name: "Customer message" }), "prazo?{Enter}");

        await waitFor(() =>
            expect(
                screen.getByText("Turn completed · input 120 tokens · output 16 tokens · 83 ms"),
            ).toBeInTheDocument(),
        );
    });

    it("a validação local e o arquivo grande respondem em inglês", async () => {
        setLocale("en-US");
        const usuario = userEvent.setup();
        montar();
        await screen.findByText("qual o prazo?");

        await usuario.click(screen.getByRole("button", { name: "Send" }));
        expect(await screen.findByText("Write a message or attach a file.")).toBeInTheDocument();

        const grande = new File(["x"], "huge.pdf", { type: "application/pdf" });
        Object.defineProperty(grande, "size", { value: 17 * 1024 * 1024 });
        const seletor = document.getElementById("chat-teste-arquivos") as HTMLInputElement;
        await usuario.upload(seletor, grande);

        expect(await screen.findByText(/huge\.pdf is 17\.0 MB: the limit is 16 MB/)).toBeInTheDocument();
    });

    it("a conversa transferida e o estado vazio aparecem em inglês", async () => {
        setLocale("en-US");
        const usuario = userEvent.setup();
        montar();
        await screen.findByText("qual o prazo?");

        await usuario.click(within(screen.getByRole("list", { name: "Conversations" })).getByText("conv_2").closest("button") as HTMLElement);

        expect(await screen.findByText("Conversation transferred to a person")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Reopen AI service in this conversation" })).toBeInTheDocument();
    });

    it("a troca para inglês reescreve a conversa aberta sem recarregar", async () => {
        montar();
        await screen.findByText("qual o prazo?");
        expect(screen.getByRole("button", { name: "Nova conversa" })).toBeInTheDocument();
        expect(screen.getByRole("textbox", { name: "Mensagem do cliente" })).toBeInTheDocument();

        act(() => setLocale("en-US"));

        expect(screen.getByRole("button", { name: "New conversation" })).toBeInTheDocument();
        expect(screen.getByRole("textbox", { name: "Customer message" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Send" })).toBeInTheDocument();
    });
});
