/**
 * Base de conhecimento do agente (HR-41): grupos, itens, vínculo e reindexação.
 *
 * O contrato é o OpenAPI (`schema.d.ts`), mas as respostas da base são `Value` genérico — o
 * formato real está nos testes de paridade do servidor e é replicado nos tipos daqui.
 *
 * Regras que a tela precisa respeitar:
 * - `PATCH` de grupo e de item exige `If-Match` (428 sem ele, 412 `version_conflict` com versão
 *   velha);
 * - `DELETE` de grupo **sempre** exige `force=true` (sem ele a API responde 409, mesmo com o grupo
 *   vazio) e leva junto itens e vínculos;
 * - item tem título até 200 caracteres e conteúdo até 1000 (`LIMITE_CONTEUDO_ITEM` no servidor);
 * - `POST …/agents/{id}/knowledge/reindex` é **síncrono**: devolve o relatório do índice quando
 *   termina (e pode demorar, porque carrega o modelo de embedding).
 */

import { pedir } from "./cliente";

/** Limite do conteúdo de um item, em caracteres (o mesmo do servidor). */
export const LIMITE_CONTEUDO_ITEM = 1000;
/** Limite do título de um item, em caracteres. */
export const LIMITE_TITULO_ITEM = 200;
/** Limite do nome de um grupo, em caracteres. */
export const LIMITE_NOME_GRUPO = 120;

/** Grupo de conhecimento, como a API administrativa devolve. */
export type GrupoConhecimento = {
    id: string;
    org_id: string;
    name: string;
    description: string;
    status: string;
    version: number;
    created_at: number;
    updated_at: number;
    /** Só no `GET …/knowledge/{id}`: ids dos agentes que usam o grupo. */
    agents?: string[];
    /** `null` quando a sincronização do índice funcionou; texto quando atrasou. */
    index_warning?: string | null;
};

/** Item da base (trecho de conhecimento). */
export type ItemConhecimento = {
    id: string;
    org_id: string;
    group_id: string;
    title: string;
    content: string;
    tags: string[];
    position: number;
    status: string;
    version: number;
    created_at: number;
    updated_at: number;
    index_warning?: string | null;
};

/** Relatório que o `POST …/knowledge/reindex` devolve quando termina. */
export type RelatorioIndexacao = {
    itens_novos: number;
    itens_alterados: number;
    itens_inalterados: number;
    itens_removidos: number;
    embeddings_gerados: number;
    trechos_gravados: number;
};

export type ResultadoReindexacao = {
    agent: string;
    indexed: RelatorioIndexacao;
};

/** Valores que o formulário de grupo grava na criação. */
export type EntradaGrupo = {
    name: string;
    description?: string;
};

/** Alteração de grupo: o servidor só muda o que vier. */
export type MudancaGrupo = {
    name?: string;
    description?: string;
    status?: string;
};

/** Valores que o formulário de item grava na criação. */
export type EntradaItem = {
    title: string;
    content: string;
    tags?: string[];
    position?: number;
};

/** Alteração de item: o servidor só muda o que vier. */
export type MudancaItem = {
    title?: string;
    content?: string;
    tags?: string[];
    position?: number;
    status?: string;
};

/** Limite por página nas listagens que aceitam `limit`. */
const LIMITE_LISTA = 200;

function grupoBase(org: string): string {
    return `/admin/v1/orgs/${encodeURIComponent(org)}/knowledge`;
}

function itemBase(org: string, grupo: string): string {
    return `${grupoBase(org)}/${encodeURIComponent(grupo)}/items`;
}

/**
 * Lê todas as páginas de uma coleção (cursor), até o fim.
 *
 * As duas listagens da base aceitam `limit`; o cursor continua valendo para orgs com mais de 200
 * registros e evita que a tela esconda o excedente em silêncio.
 */
async function todasAsPaginas<T>(caminhoBase: string): Promise<T[]> {
    const itens: T[] = [];
    let cursor: string | null = null;
    do {
        const partes = [`limit=${LIMITE_LISTA}`];
        if (cursor) partes.push(`cursor=${encodeURIComponent(cursor)}`);
        const resposta = await pedir<{ items: T[]; next_cursor: string | null }>(
            `${caminhoBase}${caminhoBase.includes("?") ? "&" : "?"}${partes.join("&")}`,
        );
        itens.push(...resposta.items);
        cursor = resposta.next_cursor ?? null;
    } while (cursor);
    return itens;
}

/** Grupos de conhecimento da org (todas as páginas). */
export async function listarGrupos(org: string): Promise<GrupoConhecimento[]> {
    return todasAsPaginas<GrupoConhecimento>(grupoBase(org));
}

/** Um grupo, com os ids dos agentes que o usam. */
export async function obterGrupo(org: string, id: string): Promise<GrupoConhecimento> {
    return pedir<GrupoConhecimento>(`${grupoBase(org)}/${encodeURIComponent(id)}`);
}

/** Cria um grupo. Descrição vazia não é enviada. */
export async function criarGrupo(
    org: string,
    entrada: EntradaGrupo,
): Promise<GrupoConhecimento> {
    const corpo: Record<string, unknown> = { name: entrada.name };
    if (entrada.description && entrada.description.trim()) {
        corpo["description"] = entrada.description;
    }
    return pedir<GrupoConhecimento>(grupoBase(org), {
        method: "POST",
        body: JSON.stringify(corpo),
    });
}

/**
 * Altera um grupo com o `If-Match` da versão lida.
 *
 * `412` significa que outra pessoa salvou primeiro; quem chama relê e decide o que mostrar, sem
 * perder o que foi digitado.
 */
export async function atualizarGrupo(
    org: string,
    id: string,
    versao: number,
    mudanca: MudancaGrupo,
): Promise<GrupoConhecimento> {
    const corpo: Record<string, unknown> = {};
    if (mudanca.name !== undefined) corpo["name"] = mudanca.name;
    if (mudanca.description !== undefined) corpo["description"] = mudanca.description;
    if (mudanca.status !== undefined) corpo["status"] = mudanca.status;
    return pedir<GrupoConhecimento>(`${grupoBase(org)}/${encodeURIComponent(id)}`, {
        method: "PATCH",
        headers: { "if-match": String(versao) },
        body: JSON.stringify(corpo),
    });
}

/**
 * Exclui um grupo, os itens e os vínculos.
 *
 * A API exige `force=true` sempre (sem ele responde 409 mesmo com o grupo vazio); a confirmação
 * na tela é o que substitui esse parâmetro — ela lista o que será apagado antes.
 */
export async function excluirGrupo(org: string, id: string): Promise<void> {
    await pedir<void>(`${grupoBase(org)}/${encodeURIComponent(id)}?force=true`, {
        method: "DELETE",
    });
}

/** Itens de um grupo (todas as páginas). */
export async function listarItens(
    org: string,
    grupo: string,
): Promise<ItemConhecimento[]> {
    return todasAsPaginas<ItemConhecimento>(itemBase(org, grupo));
}

/** Cria um item no grupo. */
export async function criarItem(
    org: string,
    grupo: string,
    entrada: EntradaItem,
): Promise<ItemConhecimento> {
    const corpo: Record<string, unknown> = {
        title: entrada.title,
        content: entrada.content,
    };
    if (entrada.tags && entrada.tags.length > 0) corpo["tags"] = entrada.tags;
    if (entrada.position !== undefined) corpo["position"] = entrada.position;
    return pedir<ItemConhecimento>(itemBase(org, grupo), {
        method: "POST",
        body: JSON.stringify(corpo),
    });
}

/** Altera um item com o `If-Match` da versão lida. */
export async function atualizarItem(
    org: string,
    grupo: string,
    id: string,
    versao: number,
    mudanca: MudancaItem,
): Promise<ItemConhecimento> {
    const corpo: Record<string, unknown> = {};
    if (mudanca.title !== undefined) corpo["title"] = mudanca.title;
    if (mudanca.content !== undefined) corpo["content"] = mudanca.content;
    if (mudanca.tags !== undefined) corpo["tags"] = mudanca.tags;
    if (mudanca.position !== undefined) corpo["position"] = mudanca.position;
    if (mudanca.status !== undefined) corpo["status"] = mudanca.status;
    return pedir<ItemConhecimento>(`${itemBase(org, grupo)}/${encodeURIComponent(id)}`, {
        method: "PATCH",
        headers: { "if-match": String(versao) },
        body: JSON.stringify(corpo),
    });
}

/** Exclui um item. */
export async function excluirItem(
    org: string,
    grupo: string,
    id: string,
): Promise<void> {
    await pedir<void>(`${itemBase(org, grupo)}/${encodeURIComponent(id)}`, {
        method: "DELETE",
    });
}

function vinculoBase(org: string, agente: string, grupo: string): string {
    return `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}/knowledge/${encodeURIComponent(grupo)}`;
}

/** Liga o grupo ao agente (o agente passa a consultar os itens dele). */
export async function vincularGrupo(
    org: string,
    agente: string,
    grupo: string,
): Promise<void> {
    await pedir<unknown>(vinculoBase(org, agente, grupo), { method: "POST" });
}

/** Desliga o grupo do agente. */
export async function desvincularGrupo(
    org: string,
    agente: string,
    grupo: string,
): Promise<void> {
    await pedir<unknown>(vinculoBase(org, agente, grupo), { method: "DELETE" });
}

/**
 * Reconstrói o índice do agente.
 *
 * É **síncrono**: a chamada só volta quando a sincronização termina. Sem o modelo de embedding
 * instalado, a API responde 503 `embedding_unavailable` e nada muda.
 */
export async function reindexarAgente(
    org: string,
    agente: string,
): Promise<ResultadoReindexacao> {
    return pedir<ResultadoReindexacao>(
        `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}/knowledge/reindex`,
        { method: "POST" },
    );
}

/** Tags digitadas como texto ("a, b, c") viram lista, sem vazios e sem repetição. */
export function lerTags(texto: string): string[] {
    const vistas = new Set<string>();
    for (const parte of texto.split(",")) {
        const tag = parte.trim();
        if (tag) vistas.add(tag);
    }
    return [...vistas];
}

/** Lista de tags de volta para o campo de texto. */
export function escreverTags(tags: string[]): string {
    return tags.join(", ");
}
