/**
 * Chat de teste do agente (HR-46): conversas de teste, histórico e o turno em streaming.
 *
 * O contrato está em `docs/API.md` ("Chat de teste do Console (HR-22)"). No OpenAPI publicado as
 * respostas destas rotas são `unknown`, então os tipos abaixo descrevem só o que a tela lê; o resto
 * do JSON passa intacto. O turno chega como `text/event-stream` numa resposta de `POST` (por isso
 * não é `EventSource`): o leitor abaixo quebra o fluxo em eventos e entrega cada um na ordem.
 */

import { ErroApi, pedir, pedirFluxo } from "./cliente";

/** Teto por arquivo do recebimento (HR-23): 16 MB. Acima disso a API recusa; a tela avisa antes. */
export const LIMITE_ARQUIVO_BYTES = 16 * 1024 * 1024;

/** Tamanho da página da lista de conversas. */
export const LIMITE_CONVERSAS = 50;

/** Status da conversa no banco: `open` (a IA responde) ou `transferred` (a IA parou). */
export type StatusConversaTeste = "open" | "transferred" | "closed" | string;

export type ConversaTeste = {
    id: string;
    org_id: string;
    agent_id: string;
    channel: string;
    status: StatusConversaTeste;
    summary: string | null;
    /** Versão da conversa: é o `If-Match` do `reopen`. */
    version: number;
    message_count: number;
    created_at: number;
    updated_at: number;
    last_message_at: number | null;
};

export type AnexoTeste = {
    id: string;
    name: string;
    mime: string;
    size_bytes: number;
    sha256: string;
    /** Resultado da extração do texto (`ok`, `sem_texto`, `falhou`…); imagem não é extraída. */
    extraction_status: string;
    has_text: boolean;
};

export type MensagemTeste = {
    id: string;
    conversation_id: string;
    /** `in` = cliente, `out` = agente. */
    direction: "in" | "out" | string;
    /** `user`, `assistant`, `tool` ou `system`. */
    role: string;
    text: string | null;
    attachments: AnexoTeste[];
    turn_id: string | null;
    created_at: number;
};

/**
 * Um arquivo que o agente enviou no turno (`files_out`, HR-46d). `arquivo_id` é o que a rota de
 * conteúdo da biblioteca aceita. `state` é o estado real do item na fila de saída.
 */
export type ArquivoEnviadoTeste = {
    id: string;
    turn_id: string;
    seq: number;
    arquivo_id: string;
    name: string;
    mime: string;
    size_bytes: number;
    sha256: string;
    /** `pending`, `sending`, `sent` ou `failed`. */
    state: string;
    created_at: number;
};

export type TurnoTeste = {
    id: string;
    /** `running`, `done`, `failed` ou `skipped`. */
    state: string;
    jev: unknown;
    retrieved: unknown;
    tools: unknown;
    /**
     * Detalhe de cada chamada de ferramenta (HR-47b): `{nome, rodada, ordem, estado, motivo,
     * duracao_ms, argumentos, resultado}`. Ausente em turno gravado antes da HR-47b; a leitura trata
     * ausência como lista vazia.
     */
    tool_calls?: unknown;
    timings: unknown;
    input_tokens: number | null;
    output_tokens: number | null;
    cost_micros: number | null;
    error: string | null;
    /** Arquivos que o agente enviou neste turno; `[]` quando não enviou nenhum. */
    files_out: ArquivoEnviadoTeste[];
    started_at: number;
    finished_at: number | null;
};

export type HistoricoTeste = {
    conversation: ConversaTeste;
    messages: MensagemTeste[];
    turns: TurnoTeste[];
};

export type ListaConversasTeste = {
    items: ConversaTeste[];
    next_cursor: string | null;
};

/** Nomes dos eventos do turno, como estão em `docs/API.md`. */
export type NomeEventoDoTurno =
    | "mensagem"
    | "inicio"
    | "texto"
    | "base"
    | "ferramenta"
    | "transferencia"
    | "fim"
    | "erro";

/** Um evento do turno, já com o JSON lido (ou o texto cru, se não for JSON). */
export type EventoDoTurno = {
    tipo: NomeEventoDoTurno;
    id: string | null;
    dados: unknown;
};

/** O que o cliente manda numa mensagem: texto (opcional) e arquivos. */
export type MensagemDoCliente = {
    texto: string;
    arquivos: File[];
};

const NOMES_DE_EVENTO = new Set<string>([
    "mensagem",
    "inicio",
    "texto",
    "base",
    "ferramenta",
    "transferencia",
    "fim",
    "erro",
]);

function caminhoDasConversas(org: string, agente: string): string {
    return `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}/test-chat`;
}

/** Lista as conversas de teste do agente, mais recentes primeiro. */
export async function listarConversasTeste(
    org: string,
    agente: string,
    cursor?: string | null,
): Promise<ListaConversasTeste> {
    const apos = cursor ? `&cursor=${encodeURIComponent(cursor)}` : "";
    return pedir<ListaConversasTeste>(
        `${caminhoDasConversas(org, agente)}?limit=${LIMITE_CONVERSAS}${apos}`,
    );
}

/** Cria conversa nova com memória zerada. */
export async function criarConversaTeste(org: string, agente: string): Promise<ConversaTeste> {
    return pedir<ConversaTeste>(caminhoDasConversas(org, agente), { method: "POST" });
}

/** Lê a conversa com as mensagens e os turnos (a base do painel do porquê). */
export async function obterHistoricoTeste(
    org: string,
    agente: string,
    conversa: string,
): Promise<HistoricoTeste> {
    return pedir<HistoricoTeste>(
        `${caminhoDasConversas(org, agente)}/${encodeURIComponent(conversa)}`,
    );
}

/**
 * Devolve a conversa para `open` (a IA volta a responder). Exige `If-Match` com a versão lida: versão
 * velha devolve `412` com a versão atual.
 */
export async function reabrirConversaTeste(
    org: string,
    agente: string,
    conversa: string,
    versao: number,
): Promise<ConversaTeste> {
    return pedir<ConversaTeste>(
        `${caminhoDasConversas(org, agente)}/${encodeURIComponent(conversa)}/reopen`,
        { method: "POST", headers: { "if-match": String(versao) } },
    );
}

/**
 * Envia a mensagem do cliente e lê o turno. Cada evento chega a `aoEvento` na ordem do servidor.
 * Quando `sinal` é abortado, a leitura para com `AbortError` e o servidor cancela o turno (a queda
 * da conexão é o cancelamento, não há rota própria).
 *
 * Erro antes do turno começar (`409`, `503`, `400` com `details.field`) sai como `ErroApi`.
 */
export async function enviarMensagemTeste(
    org: string,
    agente: string,
    conversa: string,
    mensagem: MensagemDoCliente,
    opcoes: { sinal?: AbortSignal; aoEvento: (evento: EventoDoTurno) => void },
): Promise<void> {
    const corpo = new FormData();
    const texto = mensagem.texto.trim();
    if (texto) {
        corpo.append("text", texto);
    }
    for (const arquivo of mensagem.arquivos) {
        corpo.append("file", arquivo, arquivo.name);
    }

    const resposta = await pedirFluxo(
        `${caminhoDasConversas(org, agente)}/${encodeURIComponent(conversa)}/messages`,
        { method: "POST", body: corpo, signal: opcoes.sinal },
    );
    if (!resposta.body) {
        throw new ErroApi(resposta.status, "sem_fluxo", "a resposta do turno não trouxe eventos");
    }
    await lerFluxo(resposta.body, opcoes.aoEvento, opcoes.sinal);
}

/**
 * Interpreta um quadro do `text/event-stream` (linhas `event:`, `id:` e `data:`, separadas do
 * próximo quadro por linha em branco). Comentários e eventos que não são do turno (como o `ping`)
 * devolvem `null`.
 */
export function interpretarQuadro(quadro: string): EventoDoTurno | null {
    let nome = "message";
    let id: string | null = null;
    const linhasDeDados: string[] = [];

    for (const linha of quadro.split("\n")) {
        if (linha === "" || linha.startsWith(":")) continue;
        const separador = linha.indexOf(":");
        const campo = separador === -1 ? linha : linha.slice(0, separador);
        let valor = separador === -1 ? "" : linha.slice(separador + 1);
        if (valor.startsWith(" ")) valor = valor.slice(1);
        if (campo === "event") nome = valor;
        else if (campo === "id") id = valor;
        else if (campo === "data") linhasDeDados.push(valor);
    }

    if (!NOMES_DE_EVENTO.has(nome)) return null;

    const bruto = linhasDeDados.join("\n");
    let dados: unknown = bruto;
    if (bruto) {
        try {
            dados = JSON.parse(bruto) as unknown;
        } catch {
            dados = bruto;
        }
    }
    return { tipo: nome as NomeEventoDoTurno, id, dados };
}

/**
 * Lê um fluxo `text/event-stream` até o fim e entrega cada evento do turno, na ordem. Um quadro que
 * chega partido entre dois pedaços é esperado até a linha em branco que o fecha.
 */
export async function lerFluxo(
    corpo: ReadableStream<Uint8Array>,
    aoEvento: (evento: EventoDoTurno) => void,
    sinal?: AbortSignal,
): Promise<void> {
    const leitor = corpo.getReader();
    const decodificador = new TextDecoder();
    let pendente = "";

    const entregar = (quadro: string) => {
        const evento = interpretarQuadro(quadro);
        if (evento) aoEvento(evento);
    };

    for (;;) {
        if (sinal?.aborted) {
            await leitor.cancel().catch(() => undefined);
            throw new DOMException("turno cancelado pelo operador", "AbortError");
        }
        const { done, value } = await leitor.read();
        if (done) break;
        pendente += decodificador.decode(value, { stream: true }).replace(/\r\n?/g, "\n");
        let limite = pendente.indexOf("\n\n");
        while (limite !== -1) {
            const quadro = pendente.slice(0, limite);
            pendente = pendente.slice(limite + 2);
            entregar(quadro);
            limite = pendente.indexOf("\n\n");
        }
    }

    pendente += decodificador.decode().replace(/\r\n?/g, "\n");
    if (pendente.trim()) entregar(pendente);
}
