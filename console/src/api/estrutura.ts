/**
 * Categorias de produto e departamentos da organização (HR-39b).
 *
 * Os dois recursos têm o mesmo formato (nome, descrição, situação, versão), então as chamadas
 * dividem um caminho só. O contrato é o OpenAPI (`schema.d.ts`); o `If-Match` da concorrência
 * otimista sai daqui, igual ao `orgs.ts`.
 *
 * Categoria com produtos ou agentes vinculados só sai com `force=true` (HR-39b): sem ele a API
 * responde 409 e nada é apagado. Por isso a tela conta os vínculos antes de abrir a confirmação.
 */

import { pedir } from "./cliente";
import type { components } from "./schema";
import { listarAgentes, type Agente } from "./agentes";

/** Agente, como a estrutura o lê (o tipo vive em `agentes.ts`). */
export type AgenteResumo = Agente;
export { listarAgentes };

export type Categoria = components["schemas"]["CategoriaJson"];
export type Departamento = components["schemas"]["DepartamentoJson"];
export type Registro = Categoria | Departamento;
export type TipoEstrutura = "categoria" | "departamento";

/** Trilho de cada recurso na API administrativa. */
const TRILHO: Record<TipoEstrutura, string> = {
    categoria: "categories",
    departamento: "departments",
};

/** Limite da listagem: a tela ainda não pagina (ver limites no relatório). */
const LIMITE_LISTA = 200;

function caminho(org: string, tipo: TipoEstrutura, id?: string): string {
    const base = `/admin/v1/orgs/${encodeURIComponent(org)}/${TRILHO[tipo]}`;
    return id === undefined ? base : `${base}/${encodeURIComponent(id)}`;
}

/** Valores que o formulário grava na criação. */
export type EntradaEstrutura = {
    name: string;
    description?: string;
};

/** Alteração: o servidor só muda o que vier. */
export type MudancaEstrutura = {
    name?: string;
    description?: string;
    status?: string;
};

/** Lista categorias ou departamentos da org (primeira página, até 200). */
export async function listarEstrutura(
    tipo: TipoEstrutura,
    org: string,
): Promise<Registro[]> {
    const resposta = await pedir<{ items: Registro[] }>(
        `${caminho(org, tipo)}?limit=${LIMITE_LISTA}`,
    );
    return resposta.items;
}

/** Lê um registro (usado para pegar a versão atual depois de um `412`). */
export async function obterEstrutura(
    tipo: TipoEstrutura,
    org: string,
    id: string,
): Promise<Registro> {
    return pedir<Registro>(caminho(org, tipo, id));
}

/** Cria categoria ou departamento. Descrição vazia não é enviada. */
export async function criarEstrutura(
    tipo: TipoEstrutura,
    org: string,
    entrada: EntradaEstrutura,
): Promise<Registro> {
    const corpo: Record<string, unknown> = { name: entrada.name };
    if (entrada.description && entrada.description.trim()) {
        corpo["description"] = entrada.description;
    }
    return pedir<Registro>(caminho(org, tipo), {
        method: "POST",
        body: JSON.stringify(corpo),
    });
}

/**
 * Altera com o `If-Match` da versão lida antes.
 *
 * `412` significa que outra pessoa salvou primeiro; quem chama relê e decide o que mostrar, sem
 * perder o que foi digitado.
 */
export async function atualizarEstrutura(
    tipo: TipoEstrutura,
    org: string,
    id: string,
    versao: number,
    mudanca: MudancaEstrutura,
): Promise<Registro> {
    const corpo: Record<string, unknown> = {};
    if (mudanca.name !== undefined) corpo["name"] = mudanca.name;
    if (mudanca.description !== undefined) corpo["description"] = mudanca.description;
    if (mudanca.status !== undefined) corpo["status"] = mudanca.status;
    return pedir<Registro>(caminho(org, tipo, id), {
        method: "PATCH",
        headers: { "if-match": String(versao) },
        body: JSON.stringify(corpo),
    });
}

/** Exclui um departamento. Não há `force`: a API desfaz o vínculo dos agentes (ver a tela). */
export async function excluirDepartamento(org: string, id: string): Promise<void> {
    await pedir<void>(caminho(org, "departamento", id), { method: "DELETE" });
}

/**
 * Exclui uma categoria. Sem `forcar`, a API recusa com 409 se houver produtos ou agentes
 * vinculados (o motivo vem na mensagem); com `forcar`, apaga os produtos e tira a categoria dos
 * agentes.
 */
export async function excluirCategoria(org: string, id: string, forcar: boolean): Promise<void> {
    const sufixo = forcar ? "?force=true" : "";
    await pedir<void>(`${caminho(org, "categoria", id)}${sufixo}`, { method: "DELETE" });
}

/**
 * Lê todas as páginas de uma coleção (cursor), até o fim.
 *
 * `comLimite` controla se o `limit` vai na URL. A listagem de produtos recusa `limit` (400
 * "parâmetros de paginação inválidos"), então ela pagina só com `cursor`, no tamanho padrão do
 * servidor; as outras listagens usam `limit=200`.
 */
async function todasAsPaginas<T>(caminhoBase: string, comLimite = true): Promise<T[]> {
    const itens: T[] = [];
    let cursor: string | null = null;
    do {
        const partes: string[] = [];
        if (comLimite) partes.push(`limit=${LIMITE_LISTA}`);
        if (cursor) partes.push(`cursor=${encodeURIComponent(cursor)}`);
        const sufixo = partes.length > 0 ? `${caminhoBase.includes("?") ? "&" : "?"}${partes.join("&")}` : "";
        const resposta: { items: T[]; next_cursor: string | null } = await pedir(
            `${caminhoBase}${sufixo}`,
        );
        itens.push(...resposta.items);
        cursor = resposta.next_cursor ?? null;
    } while (cursor);
    return itens;
}

/** Produto que a categoria tem, de qualquer situação (ativo, inativo ou arquivado). */
export type ProdutoDaCategoria = { id: string; name: string };

/**
 * Produtos da categoria. A listagem traz todas as situações com `all=true`, e pagina só com
 * `cursor` (sem `limit`, que o servidor recusa nessa rota).
 */
export async function produtosDaCategoria(org: string, categoriaId: string): Promise<ProdutoDaCategoria[]> {
    const todos = await todasAsPaginas<{ id: string; name: string; category_id: string | null }>(
        `/admin/v1/orgs/${encodeURIComponent(org)}/products?all=true`,
        false,
    );
    return todos
        .filter((produto) => produto.category_id === categoriaId)
        .map((produto) => ({ id: produto.id, name: produto.name }));
}

/** Agentes que usam a categoria. A lista não traz as categorias, então cada agente é lido. */
export async function agentesQueUsamCategoria(org: string, categoriaId: string): Promise<AgenteResumo[]> {
    const agentes = await todasAsPaginas<AgenteResumo>(
        `/admin/v1/orgs/${encodeURIComponent(org)}/agents`,
    );
    const detalhes = await Promise.all(
        agentes.map((agente) =>
            pedir<AgenteResumo>(
                `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente.id)}`,
            ),
        ),
    );
    return detalhes.filter((agente) => agente.product_category_ids?.includes(categoriaId));
}

/**
 * Parte do agente que a estrutura usa: os vínculos com departamento e categoria.
 *
 * A leitura de agente mora em `agentes.ts` (HR-40); aqui só se reaproveita o tipo, para a tela de
 * estrutura não conhecer dois formatos de agente.
 */

/** Agentes que usam o departamento como padrão ou como destino de transferência. */
export function agentesQueUsam(agentes: AgenteResumo[], departamentoId: string): AgenteResumo[] {
    return agentes.filter(
        (agente) =>
            agente.default_department_id === departamentoId ||
            agente.human_handoff_department_id === departamentoId,
    );
}
