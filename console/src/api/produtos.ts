/**
 * Catálogo de produtos da organização (HR-42).
 *
 * O contrato é o `docs/API.md` (linhas de Produtos). A resposta de produto é `Value` no OpenAPI,
 * então o tipo abaixo descreve os campos que a tela lê e grava, espelhando `produto_json` no servidor.
 *
 * Regras que a tela respeita:
 * - a listagem recusa `limit` com `400` (defeito já registrado), então pagina só com `cursor`;
 * - sem `all=true` a API devolve só os produtos ativos, então a tela pede todos e filtra no cliente;
 * - `PATCH` exige `If-Match` (428 sem ele, `412 version_conflict` com versão velha);
 * - `status` **não** é aceito na criação (a API responde 400 no campo `status`);
 * - a criação e a alteração devolvem `index_warning` quando o índice de busca não atualizou.
 */

import { pedir } from "./cliente";
import { listarEstrutura, type Categoria } from "./estrutura";
import { listarAgentes, obterAgente, type Agente } from "./agentes";

export type SituacaoProduto = "active" | "inactive" | "archived";

export type Produto = {
    id: string;
    org_id: string;
    category_id: string;
    name: string;
    slug: string;
    aliases: string[];
    description: string;
    price_text: string;
    price_cents: number;
    unit: string;
    includes: string;
    excludes: string;
    conditions: string;
    tags: string[];
    status: SituacaoProduto;
    version: number;
    created_at: number;
    updated_at: number;
    /** `null` ou ausente quando o índice atualizou; texto quando não atualizou. */
    index_warning?: string | null;
};

/** Valores que o formulário grava na criação (sem `status`, que a API recusa aqui). */
export type EntradaProduto = {
    category_id: string;
    name: string;
    slug?: string;
    aliases: string[];
    description: string;
    price_text: string;
    price_cents: number;
    unit: string;
    includes: string;
    excludes: string;
    conditions: string;
    tags: string[];
};

/** Alteração: o servidor só muda o que vier no corpo. */
export type MudancaProduto = Partial<Omit<EntradaProduto, "slug">> & {
    slug?: string;
    status?: SituacaoProduto;
};

/** Resposta da exclusão (`200`). Sem carrinho, sai sem `force`; com carrinho, exige `force=true` (HR-42f). */
export type ResultadoExclusaoProduto = {
    deleted: boolean;
    index_warning?: string | null;
};

function base(org: string): string {
    return `/admin/v1/orgs/${encodeURIComponent(org)}/products`;
}

/**
 * Lista todos os produtos (de qualquer situação), página a página pelo cursor.
 *
 * Sem `limit`: a rota recusa esse parâmetro. O tamanho da página é o padrão do servidor.
 */
export async function listarProdutos(org: string): Promise<Produto[]> {
    const itens: Produto[] = [];
    let cursor: string | null = null;
    do {
        const sufixo = cursor ? `&cursor=${encodeURIComponent(cursor)}` : "";
        const resposta: { items: Produto[]; next_cursor: string | null } = await pedir(
            `${base(org)}?all=true${sufixo}`,
        );
        itens.push(...resposta.items);
        cursor = resposta.next_cursor ?? null;
    } while (cursor);
    return itens;
}

/** Lê um produto (usado para pegar a versão atual depois de um `412`). */
export async function obterProduto(org: string, id: string): Promise<Produto> {
    return pedir<Produto>(`${base(org)}/${encodeURIComponent(id)}`);
}

/** Cria um produto. Slug vazio não é enviado: o servidor deriva do nome. */
export async function criarProduto(org: string, entrada: EntradaProduto): Promise<Produto> {
    const corpo: Record<string, unknown> = { ...entrada };
    if (!entrada.slug || !entrada.slug.trim()) {
        delete corpo["slug"];
    }
    return pedir<Produto>(base(org), {
        method: "POST",
        body: JSON.stringify(corpo),
    });
}

/** Altera com o `If-Match` da versão lida antes. Quem chama trata o `412`. */
export async function atualizarProduto(
    org: string,
    id: string,
    versao: number,
    mudanca: MudancaProduto,
): Promise<Produto> {
    return pedir<Produto>(`${base(org)}/${encodeURIComponent(id)}`, {
        method: "PATCH",
        headers: { "if-match": String(versao) },
        body: JSON.stringify(mudanca),
    });
}

/**
 * Exclui o produto. Sem `force`, a API recusa com `409` quando há itens de carrinho que citam o
 * produto (nada é apagado) e devolve a contagem em `detalhes`. Com `forcar`, manda `?force=true`.
 */
export async function excluirProduto(
    org: string,
    id: string,
    forcar = false,
): Promise<ResultadoExclusaoProduto> {
    const sufixo = forcar ? "?force=true" : "";
    return pedir<ResultadoExclusaoProduto>(`${base(org)}/${encodeURIComponent(id)}${sufixo}`, {
        method: "DELETE",
    });
}

/** Contagem que a recusa `409` do servidor traz quando o produto está em carrinhos. */
export type ContagemCarrinhos = { itens_de_carrinho: number; carrinhos: number };

/** Lê a contagem dos detalhes de um `409` de exclusão; `null` se o formato não for o esperado. */
export function contagemDaRecusa(detalhes: unknown): ContagemCarrinhos | null {
    if (!detalhes || typeof detalhes !== "object") return null;
    const { itens_de_carrinho, carrinhos } = detalhes as Record<string, unknown>;
    if (typeof itens_de_carrinho !== "number" || typeof carrinhos !== "number") return null;
    return { itens_de_carrinho, carrinhos };
}

/** Categorias da organização (mesma leitura da tela de estrutura). */
export async function listarCategorias(org: string): Promise<Categoria[]> {
    return (await listarEstrutura("categoria", org)) as Categoria[];
}

/**
 * Agentes com as categorias de produto que cada um enxerga.
 *
 * A lista não traz as categorias, então cada agente é lido (como na tela de estrutura). Os detalhes
 * são lidos em paralelo; a lista de agentes vem da primeira página (até 200).
 */
export async function carregarAgentesComCategorias(org: string): Promise<Agente[]> {
    const agentes = await listarAgentes(org);
    return Promise.all(agentes.map((agente) => obterAgente(org, agente.id)));
}

/**
 * Agentes que oferecem um produto pela busca.
 *
 * Regra do servidor (`visiveis_do_agente`): agente sem nenhuma categoria vinculada enxerga o
 * catálogo inteiro; com categorias, só enxerga produtos dessas categorias. Por isso o agente sem
 * vínculo entra na conta.
 */
export function agentesQueVeemProduto(agentes: Agente[], produto: Produto): Agente[] {
    return agentes.filter((agente) => {
        const categorias = agente.product_category_ids ?? [];
        return categorias.length === 0 || categorias.includes(produto.category_id);
    });
}
