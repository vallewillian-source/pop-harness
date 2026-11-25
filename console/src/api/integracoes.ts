/**
 * Integrações do agente (HR-44, HR-44c, HR-44d e HR-44e): ciclo de vida pelo contrato real.
 *
 * O que a API administrativa entrega (medido em `crates/poph-server/src/api/{integracoes,whatsapp,
 * manifestos}.rs`):
 * - listar, mostrar, criar, alterar e excluir integração por `kind`;
 * - **manifesto do plugin** (`GET …/manifests/{kind}`): é a fonte do formulário — `configuracao` e
 *   `segredos` são JSON Schema. `404` = este `kind` não tem plugin declarado (logo, não tem
 *   formulário); `503 manifesto_indisponivel` = o plugin está declarado mas não respondeu;
 * - `config_json` e `secrets` aceitos na criação e na alteração quando o `kind` tem manifesto; sem
 *   manifesto a API continua recusando com `422 manifest_indisponivel`;
 * - na integração de `kind = "whatsapp"`, parear (`POST …/whatsapp/qrcode`), ler o estado ao vivo
 *   (`GET …/whatsapp/state`) e desconectar (`POST …/whatsapp/disconnect`, HR-44b).
 *
 * As duas escritas de WhatsApp exigem `If-Match` e **não sobem a versão**: `connection_state` é
 * estado de execução, não edição de configuração. Quem está com a tela aberta pode gerar QR de novo
 * com a mesma versão; se outra pessoa editar a integração no meio, a escrita devolve `412`.
 */

import { ErroApi, pedir } from "./cliente";
import type { EsquemaDeObjeto } from "../componentes/FormularioEsquema";
import { translate } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";

/**
 * Integração como a API a devolve. O OpenAPI publica a resposta como JSON genérico, então o tipo é
 * escrito aqui a partir do handler (`valor()` em `api/integracoes.rs`).
 */
export type Integracao = {
    id: string;
    kind: string;
    status: string;
    connection_state: string;
    version: number;
    /** `true` quando existe segredo gravado. O valor do segredo nunca sai da API. */
    tem_segredo: boolean;
    /**
     * Configuração pública gravada (o que o formulário do manifesto preencheu). Nunca traz segredo.
     * `null` quando a integração não tem configuração.
     */
    configuracao?: Record<string, unknown> | null;
    /**
     * Nomes dos segredos já preenchidos. `null` = há segredo gravado, mas a chave de cifra atual não
     * abre o blob: a tela diz "gravado, não legível" em vez de fingir que está vazio.
     */
    segredos_preenchidos?: string[] | null;
};

/** Situações aceitas pela API (`status_pedido`, `api/mod.rs`). */
export const SITUACOES_INTEGRACAO = [
    {
        valor: "active",
        get rotulo() {
            return translate("integrations.statusActive");
        },
    },
    {
        valor: "inactive",
        get rotulo() {
            return translate("integrations.statusInactive");
        },
    },
    {
        valor: "archived",
        get rotulo() {
            return translate("integrations.statusArchived");
        },
    },
] as const;

export type SituacaoIntegracao = (typeof SITUACOES_INTEGRACAO)[number]["valor"];

/** Estados de conexão que o webhook do canal grava (`docs/EVOLUTION.md`). */
const CONNECTION_LABEL_KEYS: Record<string, TranslationKey> = {
    connected: "integrations.connectionConnected",
    disconnected: "integrations.connectionDisconnected",
    awaiting_scan: "integrations.connectionAwaitingScan",
    connecting: "integrations.connectionConnecting",
};

/** Rótulo do estado de conexão; estado desconhecido aparece como veio. */
export function rotuloDaConexao(estado: string): string {
    const key = CONNECTION_LABEL_KEYS[estado];
    return key ? translate(key) : estado;
}

/** Tipo sugerido para a integração de WhatsApp (o mesmo `kind` do plugin de Evolution). */
export const KIND_WHATSAPP = "whatsapp";

/** Limite da listagem: a API pagina por cursor, e a tela lê a primeira página. */
export const LIMITE_LISTA_INTEGRACOES = 200;

/** Comprimento máximo do `kind` que a tela aceita (a API não impõe limite). */
export const LIMITE_KIND = 60;

function base(org: string, agente: string): string {
    return `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}/integrations`;
}

/** Lista as integrações do agente (primeira página). */
export async function listarIntegracoes(org: string, agente: string): Promise<Integracao[]> {
    const resposta = await pedir<{ items: Integracao[]; next_cursor?: string | null }>(
        `${base(org, agente)}?limit=${LIMITE_LISTA_INTEGRACOES}`,
    );
    return resposta.items;
}

/** Lê uma integração (usado para reler depois de um `412`). */
export async function obterIntegracao(
    org: string,
    agente: string,
    id: string,
): Promise<Integracao> {
    return pedir<Integracao>(`${base(org, agente)}/${encodeURIComponent(id)}`);
}

/**
 * Cria a integração. `connection_state` é proibido na criação; `config_json`/`secrets` só vão
 * quando a tela tem o formulário do manifesto (sem ele a API recusa com `422`).
 */
export async function criarIntegracao(
    org: string,
    agente: string,
    kind: string,
    dados: DadosDaIntegracao = {},
): Promise<Integracao> {
    return pedir<Integracao>(base(org, agente), {
        method: "POST",
        body: JSON.stringify(corpoDaIntegracao(kind, dados)),
    });
}

/** O que a tela pode gravar numa integração, além do `kind` (fixo depois de criada). */
export type DadosDaIntegracao = {
    status?: SituacaoIntegracao;
    /** Configuração validada pelo esquema do manifesto. Ausente = não mexe. */
    configuracao?: Record<string, unknown>;
    /** Segredo por nome; `""` remove. Ausente = mantém os atuais. */
    segredos?: Record<string, string>;
};

/**
 * Monta o corpo da criação/da alteração. Só as chaves que existem de verdade entram: o `POST` do
 * contrato não usa `config_json` quando a integração não tem formulário, e o `PATCH` trata um objeto
 * vazio como "nada a alterar".
 */
function corpoDaIntegracao(
    kind: string | null,
    dados: DadosDaIntegracao,
): Record<string, unknown> {
    const corpo: Record<string, unknown> = {};
    if (kind !== null) corpo["kind"] = kind;
    if (dados.status !== undefined) corpo["status"] = dados.status;
    if (dados.configuracao !== undefined) corpo["config_json"] = dados.configuracao;
    if (dados.segredos !== undefined && Object.keys(dados.segredos).length > 0) {
        corpo["secrets"] = dados.segredos;
    }
    return corpo;
}

/**
 * Altera a integração com `If-Match` da versão lida: situação e, quando houver manifesto,
 * configuração e segredos. Sem o cabeçalho o servidor devolve `428`; versão velha devolve `412
 * version_conflict`.
 */
export async function alterarIntegracao(
    org: string,
    agente: string,
    id: string,
    versao: number,
    dados: DadosDaIntegracao,
): Promise<Integracao> {
    return pedir<Integracao>(`${base(org, agente)}/${encodeURIComponent(id)}`, {
        method: "PATCH",
        headers: { "if-match": String(versao) },
        body: JSON.stringify(corpoDaIntegracao(null, dados)),
    });
}

/** Exclui a integração. A API não pede `force` aqui. */
export async function excluirIntegracao(
    org: string,
    agente: string,
    id: string,
): Promise<void> {
    await pedir<{ deleted: boolean }>(`${base(org, agente)}/${encodeURIComponent(id)}`, {
        method: "DELETE",
    });
}

// ───────────────────────── Manifesto do plugin (HR-44c / HR-44e) ─────────────────────────

/**
 * Manifesto do plugin como a rota devolve (`GET …/manifests/{kind}`). Só os campos que a tela usa;
 * o resto (`capacidades`, `limites`, `protocolo`) fica de fora porque a tela não os desenha.
 */
export type ManifestoDoPlugin = {
    nome: string;
    kind: string;
    versao: string;
    /** JSON Schema do formulário de configuração (o Console desenha a partir dele). */
    configuracao: EsquemaDeObjeto;
    /** JSON Schema dos segredos. Ausente ou vazio = o canal não usa segredo. */
    segredos?: EsquemaDeObjeto | null;
};

/**
 * Lê o manifesto do `kind`. `404 not_found` = este tipo não tem plugin declarado, logo não tem
 * formulário (a tela segue sem configuração). `503 manifesto_indisponivel` = o plugin está
 * declarado e não respondeu ao handshake: aí a tela não deixa salvar, porque o servidor também
 * recusaria.
 */
export async function obterManifesto(org: string, kind: string): Promise<ManifestoDoPlugin> {
    return pedir<ManifestoDoPlugin>(
        `/admin/v1/orgs/${encodeURIComponent(org)}/manifests/${encodeURIComponent(kind)}`,
    );
}

/** `true` quando a falha quer dizer "este tipo não tem formulário" (e não "o plugin quebrou"). */
export function semManifestoParaEsteTipo(falha: unknown): boolean {
    return falha instanceof ErroApi && falha.status === 404;
}

// ───────────────────────── WhatsApp (HR-44b) ─────────────────────────

/** QR de pareamento devolvido pela Evolution (data URL em `base64`). */
export type QrDoWhatsapp = {
    /** `null` quando o aparelho já está conectado: não há QR a mostrar. */
    qrcode: {
        base64?: string | null;
        code?: string | null;
        count?: number | null;
    } | null;
    /** Código de 8 caracteres; só vem quando o pareamento é por número. */
    pairing_code?: string | null;
    /** Vocabulário da Evolution (`open`, `connecting`, `close`). */
    state: string;
    /** Nosso vocabulário (`connected`, `awaiting_scan`, `disconnected`, `desconhecido`). */
    connection_state: string;
    /** Nome da instância na Evolution (o mesmo do webhook). */
    instance?: string;
    /** Sempre `null`: a Evolution v2 não informa validade do QR. */
    expira_em?: string | null;
};

/** Estado ao vivo da instância (a rota grava em `connection_state` e devolve o estado). */
export type EstadoDoWhatsapp = {
    state: string;
    connection_state: string;
    instance?: string;
};

/** Resultado do desconectar. */
export type DesconexaoDoWhatsapp = {
    connection_state: string;
    disconnected: boolean;
    instance?: string;
};

function baseDoWhatsapp(org: string, agente: string, id: string): string {
    return `${base(org, agente)}/${encodeURIComponent(id)}/whatsapp`;
}

/**
 * Pede o pareamento e devolve o QR. Exige `If-Match` da versão lida e **não** sobe a versão.
 * Devolve `qrcode: null` quando o aparelho já está conectado (`state: open`).
 */
export async function gerarQrDoWhatsapp(
    org: string,
    agente: string,
    id: string,
    versao: number,
    /** Só dígitos com DDI. Com o número, o WhatsApp também devolve o código de pareamento. */
    numero?: string,
): Promise<QrDoWhatsapp> {
    const corpo = numero ? JSON.stringify({ number: numero }) : undefined;
    return pedir<QrDoWhatsapp>(`${baseDoWhatsapp(org, agente, id)}/qrcode`, {
        method: "POST",
        headers: { "if-match": String(versao) },
        body: corpo,
    });
}

/** Lê o estado ao vivo da instância (grava em `connection_state`, sem subir a versão). */
export async function lerEstadoDoWhatsapp(
    org: string,
    agente: string,
    id: string,
): Promise<EstadoDoWhatsapp> {
    return pedir<EstadoDoWhatsapp>(`${baseDoWhatsapp(org, agente, id)}/state`);
}

/** Desconecta o aparelho. Exige `If-Match`; o efeito é `connection_state = disconnected`. */
export async function desconectarWhatsapp(
    org: string,
    agente: string,
    id: string,
    versao: number,
): Promise<DesconexaoDoWhatsapp> {
    return pedir<DesconexaoDoWhatsapp>(`${baseDoWhatsapp(org, agente, id)}/disconnect`, {
        method: "POST",
        headers: { "if-match": String(versao) },
    });
}

/** Transforma o `base64` do QR em fonte de `<img>` (a Evolution já manda data URL). */
export function fonteDoQr(base64: string): string {
    return base64.startsWith("data:") ? base64 : `data:image/png;base64,${base64}`;
}

/**
 * Explicação de uma falha do painel do WhatsApp: mensagem real da API, código e, quando o código é
 * conhecido, uma dica em linguagem de operação (o que fazer sem abrir o código do servidor).
 */
export type FalhaDoWhatsapp = { texto: string; codigo?: string; dica?: string };

const WHATSAPP_TIP_KEYS: Record<string, TranslationKey> = {
    whatsapp_nao_configurado: "integrations.whatsappTipNotConfigured",
    evolution_indisponivel: "integrations.whatsappTipEvolutionUnavailable",
    integracao_nao_e_whatsapp: "integrations.whatsappTipNotWhatsapp",
    evolution_recusou: "integrations.whatsappTipEvolutionRefused",
    evolution_sem_qr: "integrations.whatsappTipEvolutionNoQr",
    version_conflict: "integrations.whatsappTipVersionConflict",
    precondition_required: "integrations.whatsappTipPreconditionRequired",
};

export function explicarFalhaDoWhatsapp(falha: unknown): FalhaDoWhatsapp {
    if (falha instanceof ErroApi) {
        const tipKey = WHATSAPP_TIP_KEYS[falha.codigo];
        return {
            texto: falha.message,
            codigo: falha.codigo,
            dica: tipKey ? translate(tipKey) : undefined,
        };
    }
    if (falha instanceof Error) {
        return { texto: falha.message };
    }
    return { texto: translate("integrations.whatsappUnexpectedFailure") };
}

/**
 * Valida o formulário antes de chamar a API. Devolve mensagens por campo; vazio = pode gravar.
 * Chave `kind` e `situacao`.
 */
export function validarIntegracao(campos: {
    kind: string;
    situacao: string;
    editando: boolean;
}): Record<string, string> {
    const erros: Record<string, string> = {};
    if (!campos.editando) {
        const kind = campos.kind.trim();
        if (kind === "") {
            erros["kind"] = translate("integrations.validationKindRequired");
        } else if (kind.length > LIMITE_KIND) {
            erros["kind"] = translate("integrations.validationKindMaxLength", { max: LIMITE_KIND });
        }
    }
    if (!SITUACOES_INTEGRACAO.some((situacao) => situacao.valor === campos.situacao)) {
        erros["situacao"] = translate("integrations.validationStatusRequired");
    }
    return erros;
}
