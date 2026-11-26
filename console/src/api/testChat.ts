
import { translate } from "../i18n";
import { ApiError, request, requestFlow } from "./client";

export const LIMIT_FILE_BYTES = 16 * 1024 * 1024;

export const LIMIT_CONVERSATIONS = 50;

export type TestConversationStatus = "open" | "transferred" | "closed" | string;

export type TestConversation = {
    id: string;
    org_id: string;
    agent_id: string;
    channel: string;
    status: TestConversationStatus;
    summary: string | null;
    version: number;
    message_count: number;
    created_at: number;
    updated_at: number;
    last_message_at: number | null;
};

export type TestAttachment = {
    id: string;
    name: string;
    mime: string;
    size_bytes: number;
    sha256: string;
    extraction_status: string;
    has_text: boolean;
};

export type TestMessage = {
    id: string;
    conversation_id: string;
    direction: "in" | "out" | string;
    role: string;
    text: string | null;
    attachments: TestAttachment[];
    turn_id: string | null;
    created_at: number;
};

export type TestSentFile = {
    id: string;
    turn_id: string;
    seq: number;
    "arquivo_id": string;
    name: string;
    mime: string;
    size_bytes: number;
    sha256: string;
    state: string;
    created_at: number;
};

export type TestTurn = {
    id: string;
    state: string;
    jev: unknown;
    retrieved: unknown;
    tools: unknown;
    tool_calls?: unknown;
    timings: unknown;
    input_tokens: number | null;
    output_tokens: number | null;
    cost_micros: number | null;
    error: string | null;
    files_out: TestSentFile[];
    started_at: number;
    finished_at: number | null;
};

export type TestHistory = {
    conversation: TestConversation;
    messages: TestMessage[];
    turns: TestTurn[];
};

export type TestConversationList = {
    items: TestConversation[];
    next_cursor: string | null;
};

export type TurnEventName =
    | "mensagem"
    | "inicio"
    | "texto"
    | "base"
    | "ferramenta"
    | "transferencia"
    | "fim"
    | "erro";

export type TurnEvent = {
    tipo: TurnEventName;
    id: string | null;
    dados: unknown;
};

export type ClientMessage = {
    texto: string;
    "arquivos": File[];
};

const EVENT_NAMES = new Set<string>([
    "mensagem",
    "inicio",
    "texto",
    "base",
    "ferramenta",
    "transferencia",
    "fim",
    "erro",
]);

function conversationsPath(org: string, agente: string): string {
    return `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}/test-chat`;
}

export async function listTestConversations(
    org: string,
    agente: string,
    cursor?: string | null,
): Promise<TestConversationList> {
    const after = cursor ? `&cursor=${encodeURIComponent(cursor)}` : "";
    return request<TestConversationList>(
        `${conversationsPath(org, agente)}?limit=${LIMIT_CONVERSATIONS}${after}`,
    );
}

export async function createTestConversation(org: string, agente: string): Promise<TestConversation> {
    return request<TestConversation>(conversationsPath(org, agente), { method: "POST" });
}

export async function getTestHistory(
    org: string,
    agente: string,
    conversa: string,
): Promise<TestHistory> {
    return request<TestHistory>(
        `${conversationsPath(org, agente)}/${encodeURIComponent(conversa)}`,
    );
}

export async function reopenTestConversation(
    org: string,
    agente: string,
    conversa: string,
    versao: number,
): Promise<TestConversation> {
    return request<TestConversation>(
        `${conversationsPath(org, agente)}/${encodeURIComponent(conversa)}/reopen`,
        { method: "POST", headers: { "if-match": String(versao) } },
    );
}

export async function sendTestMessage(
    org: string,
    agente: string,
    conversa: string,
    mensagem: ClientMessage,
    options: { "sinal"?: AbortSignal; "aoEvento": (evento: TurnEvent) => void },
): Promise<void> {
    const body = new FormData();
    const texto = mensagem.texto.trim();
    if (texto) {
        body.append("text", texto);
    }
    for (const file of mensagem["arquivos"]) {
        body.append("file", file, file.name);
    }

    const response = await requestFlow(
        `${conversationsPath(org, agente)}/${encodeURIComponent(conversa)}/messages`,
        { method: "POST", body: body, signal: options["sinal"] },
    );
    if (!response.body) {
        throw new ApiError(response.status, "sem_fluxo", translate("testChat.streamWithoutEvents"));
    }
    await readFlow(response.body, options["aoEvento"], options["sinal"]);
}

export function parseFrame(frame: string): TurnEvent | null {
    let nome = "message";
    let id: string | null = null;
    const dataLines: string[] = [];

    for (const line of frame.split("\n")) {
        if (line === "" || line.startsWith(":")) continue;
        const separador = line.indexOf(":");
        const campo = separador === -1 ? line : line.slice(0, separador);
        let value = separador === -1 ? "" : line.slice(separador + 1);
        if (value.startsWith(" ")) value = value.slice(1);
        if (campo === "event") nome = value;
        else if (campo === "id") id = value;
        else if (campo === "data") dataLines.push(value);
    }

    if (!EVENT_NAMES.has(nome)) return null;

    const raw = dataLines.join("\n");
    let dados: unknown = raw;
    if (raw) {
        try {
            dados = JSON.parse(raw) as unknown;
        } catch {
            dados = raw;
        }
    }
    return { tipo: nome as TurnEventName, id, dados };
}

export async function readFlow(
    body: ReadableStream<Uint8Array>,
    onEvent: (evento: TurnEvent) => void,
    signal?: AbortSignal,
): Promise<void> {
    const reader = body.getReader();
    const decodificador = new TextDecoder();
    let pending = "";

    const deliver = (frame: string) => {
        const evento = parseFrame(frame);
        if (evento) onEvent(evento);
    };

    for (;;) {
        if (signal?.aborted) {
            await reader.cancel().catch(() => undefined);
            throw new DOMException("turno cancelado pelo operador", "AbortError");
        }
        const { done, value } = await reader.read();
        if (done) break;
        pending += decodificador.decode(value, { stream: true }).replace(/\r\n?/g, "\n");
        let limit = pending.indexOf("\n\n");
        while (limit !== -1) {
            const frame = pending.slice(0, limit);
            pending = pending.slice(limit + 2);
            deliver(frame);
            limit = pending.indexOf("\n\n");
        }
    }

    pending += decodificador.decode().replace(/\r\n?/g, "\n");
    if (pending.trim()) deliver(pending);
}
