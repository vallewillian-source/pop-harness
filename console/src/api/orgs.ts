/**
 * Operações de organização na API administrativa (HR-39a).
 *
 * O contrato continua sendo o OpenAPI (`schema.d.ts`), consumido por `cliente.ts`; aqui ficam as
 * quatro chamadas de escrita/leitura que as telas usam, com o `If-Match` da concorrência otimista
 * resolvido num só lugar (PRD §8 / docs/API.md: `PATCH` sem `If-Match` = 428, versão velha = 412).
 */

import { ErroApi, pedir, type Org } from "./cliente";

/** Retenção do histórico mora em `config`, que é JSON livre na API. */
const CHAVE_RETENCAO = "retention_days";

/** Valores que o formulário grava. `slug` e `config` só são enviados quando fazem sentido. */
export type EntradaOrg = {
    name: string;
    slug?: string;
    config?: Record<string, unknown>;
};

/** Alteração de org: o servidor só muda o que vier. */
export type MudancaOrg = {
    name?: string;
    status?: string;
    config?: Record<string, unknown>;
};

/** Lê a retenção em dias da configuração da org (`undefined` = não definida). */
export function retencaoEmDias(org: Pick<Org, "config">): number | undefined {
    const config = org.config as Record<string, unknown> | undefined;
    const valor = config?.[CHAVE_RETENCAO];
    return typeof valor === "number" && Number.isFinite(valor) ? valor : undefined;
}

/** Devolve a configuração atual com a retenção trocada, preservando as demais chaves. */
export function comRetencao(
    config: Record<string, unknown> | undefined,
    dias: number | null,
): Record<string, unknown> {
    const nova: Record<string, unknown> = { ...(config ?? {}) };
    if (dias === null) {
        delete nova[CHAVE_RETENCAO];
    } else {
        nova[CHAVE_RETENCAO] = dias;
    }
    return nova;
}

/** Cria a org (`POST /admin/v1/orgs`, só token global). */
export async function criarOrg(entrada: EntradaOrg): Promise<Org> {
    const corpo: Record<string, unknown> = { name: entrada.name };
    if (entrada.slug) corpo["slug"] = entrada.slug;
    if (entrada.config && Object.keys(entrada.config).length > 0) {
        corpo["config"] = entrada.config;
    }
    return pedir<Org>("/admin/v1/orgs", { method: "POST", body: JSON.stringify(corpo) });
}

/** Lê uma org (`GET /admin/v1/orgs/{id}`). */
export async function obterOrg(id: string): Promise<Org> {
    return pedir<Org>(`/admin/v1/orgs/${encodeURIComponent(id)}`);
}

/**
 * Altera a org com o `If-Match` da versão lida antes.
 *
 * Um `412` significa que outra pessoa salvou primeiro: a chamada falha e quem chama decide o que
 * mostrar, sem perder o que o operador digitou.
 */
export async function atualizarOrg(
    id: string,
    versao: number,
    mudanca: MudancaOrg,
): Promise<Org> {
    const corpo: Record<string, unknown> = {};
    if (mudanca.name !== undefined) corpo["name"] = mudanca.name;
    if (mudanca.status !== undefined) corpo["status"] = mudanca.status;
    if (mudanca.config !== undefined) corpo["config"] = mudanca.config;
    return pedir<Org>(`/admin/v1/orgs/${encodeURIComponent(id)}`, {
        method: "PATCH",
        headers: { "if-match": String(versao) },
        body: JSON.stringify(corpo),
    });
}

/**
 * Exclui a org (`DELETE /admin/v1/orgs/{id}`).
 *
 * `force` confirma a exclusão de uma org com conteúdo (agentes, categorias, departamentos, tokens);
 * sem ele a API responde `409 conflict`, que a tela mostra como veio.
 */
export async function excluirOrg(id: string, force: boolean): Promise<void> {
    const consulta = force ? "?force=true" : "";
    await pedir<void>(`/admin/v1/orgs/${encodeURIComponent(id)}${consulta}`, {
        method: "DELETE",
    });
}

/** Versões de uma disputa de concorrência (`details` do `412 version_conflict`). */
export type ConflitoDeVersao = {
    esperada?: number;
    atual?: number;
};

/** Extrai `details.expected_version`/`details.current_version` de um `412`, quando vierem. */
export function conflitoDeVersao(erro: unknown): ConflitoDeVersao | null {
    if (!(erro instanceof ErroApi) || erro.status !== 412) return null;
    const detalhes = erro.detalhes as Record<string, unknown> | null | undefined;
    const esperada = detalhes?.["expected_version"];
    const atual = detalhes?.["current_version"];
    return {
        esperada: typeof esperada === "number" ? esperada : undefined,
        atual: typeof atual === "number" ? atual : undefined,
    };
}
