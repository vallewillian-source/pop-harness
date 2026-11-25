/**
 * Cliente HTTP da API administrativa (`/admin/v1`).
 *
 * O contrato não é escrito à mão: os tipos vêm de `schema.d.ts`, gerado do OpenAPI do servidor por
 * `console/gen-api.sh` (HR-20/HR-38). Aqui ficam só o transporte, o token e o formato de erro
 * `{ code, message, details }` do PRD §8.1.
 */

import type { components } from "./schema";
import { translate } from "../i18n";

export type Org = components["schemas"]["OrgJson"];
export type ListaOrgs = components["schemas"]["ListaJson_OrgJson"];
export type RespostaErro = components["schemas"]["RespostaErro"];

/**
 * Estado de uma instância na listagem com `?cluster=true` (campo `instancias`). `situacao` é
 * `local` (esta instância), `ok` (respondeu) ou `inacessivel`.
 */
export type InstanciaDaLista = {
    id: string;
    situacao: string;
    orgs: number;
    mensagem?: string | null;
};

/**
 * Org na listagem da tela. `agents_active_count` é a contagem de agentes ativos (contrato novo,
 * CF-03a): fica opcional até o tipo gerado trazer o campo, e a tela mostra "—" enquanto não vier.
 */
export type OrgNaLista = Org & { agents_active_count?: number };

/** Listagem de organizações da tela: com `?cluster=true` traz também o estado das instâncias. */
export type ListaOrgsDoCluster = {
    items: OrgNaLista[];
    next_cursor?: string | null;
    instancias?: InstanciaDaLista[];
};

/** Falha de uma chamada à API, já traduzida do formato de erro do servidor. */
export class ErroApi extends Error {
    /** Status HTTP; `0` quando a requisição nem chegou ao servidor. */
    readonly status: number;
    /** Código estável do servidor (`invalid_token`, `not_found`, `rede`…). */
    readonly codigo: string;
    /** Detalhe técnico para colar num ticket, quando o servidor manda. */
    readonly detalhes?: unknown;

    constructor(status: number, codigo: string, mensagem: string, detalhes?: unknown) {
        super(mensagem);
        this.name = "ErroApi";
        this.status = status;
        this.codigo = codigo;
        this.detalhes = detalhes;
    }

    /** `true` quando o servidor recusou o token (sessão inválida, revogada ou expirada). */
    get naoAutorizado(): boolean {
        return this.status === 401;
    }

    /** `true` quando a falha é de transporte (servidor fora do ar, rede caindo). */
    get semServidor(): boolean {
        return this.status === 0;
    }

    /** `true` quando o token é válido mas o escopo dele não alcança o recurso. */
    get semPermissao(): boolean {
        return this.status === 403;
    }
}

type OuvinteExpiracao = () => void;

let tokenAtual: string | null = null;
let aoExpirar: OuvinteExpiracao | null = null;

/** Define o token enviado em toda chamada (`null` desliga). */
export function definirToken(token: string | null): void {
    tokenAtual = token;
}

/** Registra o que fazer quando o servidor recusar o token de uma sessão já aberta. */
export function quandoExpirar(ouvinte: OuvinteExpiracao | null): void {
    aoExpirar = ouvinte;
}

/** Base da API. Vazio = mesma origem (o Console é servido pelo próprio `poph serve`). */
const BASE = "";

function mensagemDeRede(erro: unknown): string {
    const motivo = erro instanceof Error ? erro.message : String(erro);
    return translate("client.networkError", { reason: motivo });
}

/** Ajustes de transporte de uma chamada. */
export type OpcoesPedido = {
    /**
     * `true` (padrão): um `401` significa que a sessão em uso caiu, e o Console volta ao login.
     * `false` para chamadas que estão **validando** um token (o login precisa do erro na tela, não
     * de uma expiração de sessão).
     */
    encerraSessao?: boolean;
};

/**
 * Faz uma chamada e devolve o corpo já tipado.
 *
 * Erro sempre vira `ErroApi`; um `401` numa sessão aberta dispara o ouvinte de expiração uma vez.
 */
export async function pedir<T>(
    caminho: string,
    init: RequestInit = {},
    opcoes: OpcoesPedido = {},
): Promise<T> {
    const cabecalhos = new Headers(init.headers);
    if (init.body !== undefined && !cabecalhos.has("content-type")) {
        cabecalhos.set("content-type", "application/json");
    }
    if (tokenAtual) {
        cabecalhos.set("authorization", `Bearer ${tokenAtual}`);
    }
    cabecalhos.set("accept", "application/json");

    let resposta: Response;
    try {
        resposta = await fetch(`${BASE}${caminho}`, { ...init, headers: cabecalhos });
    } catch (erro) {
        throw new ErroApi(0, "rede", mensagemDeRede(erro));
    }

    if (resposta.status === 204) {
        return undefined as T;
    }

    const corpo = lerCorpo(await resposta.text());
    if (!resposta.ok) {
        throw erroDaResposta(resposta, corpo, opcoes.encerraSessao ?? true);
    }

    return corpo as T;
}

/** Lê o corpo JSON; texto que não é JSON vira `undefined` (o erro segue com o status). */
function lerCorpo(texto: string): unknown {
    if (!texto) return undefined;
    try {
        return JSON.parse(texto) as unknown;
    } catch {
        return undefined;
    }
}

/** Traduz uma resposta de erro no formato `{ code, message, details }` para `ErroApi`. */
function erroDaResposta(resposta: Response, corpo: unknown, encerraSessao: boolean): ErroApi {
    const erro = corpo as Partial<RespostaErro> | undefined;
    const codigo = typeof erro?.code === "string" ? erro.code : `http_${resposta.status}`;
    const status = resposta.status;
    const mensagem =
        typeof erro?.message === "string" && erro.message
            ? erro.message
            : translate("client.httpError", { status });
    if (status === 401 && encerraSessao && tokenAtual !== null) {
        aoExpirar?.();
    }
    return new ErroApi(status, codigo, mensagem, erro?.details);
}

/**
 * Chamada que devolve a resposta **crua** quando ela é um fluxo de eventos (`text/event-stream`, o
 * turno do chat de teste). Mesmo token e mesmo formato de erro de `pedir`; quem lê o corpo é quem
 * chama. Aborto do `AbortSignal` sai como `AbortError`, sem virar erro de rede.
 */
export async function pedirFluxo(
    caminho: string,
    init: RequestInit = {},
    opcoes: OpcoesPedido = {},
): Promise<Response> {
    const cabecalhos = new Headers(init.headers);
    if (tokenAtual) {
        cabecalhos.set("authorization", `Bearer ${tokenAtual}`);
    }
    cabecalhos.set("accept", "text/event-stream");

    let resposta: Response;
    try {
        resposta = await fetch(`${BASE}${caminho}`, { ...init, headers: cabecalhos });
    } catch (erro) {
        if (erro instanceof DOMException && erro.name === "AbortError") throw erro;
        throw new ErroApi(0, "rede", mensagemDeRede(erro));
    }

    if (resposta.ok) {
        return resposta;
    }
    const corpo = lerCorpo(await resposta.text());
    throw erroDaResposta(resposta, corpo, opcoes.encerraSessao ?? true);
}

/** Resposta de um download: os bytes e os cabeçalhos (nome e tipo vêm deles). */
export type RespostaBinaria = {
    blob: Blob;
    cabecalhos: Headers;
};

/**
 * Baixa bytes de um arquivo (conteúdo de anexo ou da biblioteca) com o mesmo token das outras
 * chamadas. Não dá para usar `<img src>` nem `<a href>` direto: a rota exige o cabeçalho
 * `Authorization`, e o navegador não manda esse cabeçalho nessas tags. Erro vira `ErroApi`, com o
 * mesmo formato de `pedir`; nada do corpo do arquivo entra em mensagem de erro.
 */
export async function pedirBytes(
    caminho: string,
    opcoes: OpcoesPedido = {},
): Promise<RespostaBinaria> {
    const cabecalhos = new Headers();
    if (tokenAtual) {
        cabecalhos.set("authorization", `Bearer ${tokenAtual}`);
    }

    let resposta: Response;
    try {
        resposta = await fetch(`${BASE}${caminho}`, { method: "GET", headers: cabecalhos });
    } catch (erro) {
        throw new ErroApi(0, "rede", mensagemDeRede(erro));
    }

    if (!resposta.ok) {
        const corpo = lerCorpo(await resposta.text());
        throw erroDaResposta(resposta, corpo, opcoes.encerraSessao ?? true);
    }

    return { blob: await resposta.blob(), cabecalhos: resposta.headers };
}

/** Situação de saúde da instância (`GET /healthz`, público). */
export type Saude = {
    status: string;
    version: string;
    uptime_seconds: number;
    databases: { harness: number; rag: number };
};

/** Lê a saúde da instância sem token. */
export async function saude(): Promise<Saude> {
    let resposta: Response;
    try {
        resposta = await fetch(`${BASE}/healthz`, { headers: { accept: "application/json" } });
    } catch (erro) {
        throw new ErroApi(0, "rede", mensagemDeRede(erro));
    }
    if (!resposta.ok) {
        throw new ErroApi(resposta.status, "healthz", translate("client.healthError", { status: resposta.status }));
    }
    return (await resposta.json()) as Saude;
}
