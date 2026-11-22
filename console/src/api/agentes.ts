/**
 * Agentes de atendimento da organização (HR-40).
 *
 * Esta rodada só **lê** agente: a lista (`GET …/agents`) e um agente (`GET …/agents/{id}`). O CRUD
 * de agente é tarefa seguinte; a tela de Instruções abre o agente selecionado e trabalha sobre ele.
 *
 * O contrato é o OpenAPI (`schema.d.ts`), mas a listagem e o detalhe devolvem `Value` (campos
 * livres, montados em `json_agente` no servidor), então o tipo abaixo descreve só os campos que a
 * tela lê — o resto do JSON passa intacto.
 */

import { pedir } from "./cliente";

/** Limite da listagem: a tela ainda não pagina (ver limites no relatório). */
export const LIMITE_AGENTES = 200;

/** Agente como o servidor devolve em `json_agente`. */
export type Agente = {
    id: string;
    org_id: string;
    name: string;
    slug: string;
    description: string;
    status: string;
    agent_type?: string;
    category?: string;
    default_department_id: string | null;
    human_handoff_enabled?: boolean;
    human_handoff_department_id: string | null;
    provider?: string;
    model?: string;
    /** `null` = herda o padrão do servidor (a API não aceita voltar a `null` depois de definido). */
    temperature?: number | null;
    max_output_tokens?: number | null;
    max_context_tokens?: number;
    grouping_window_ms?: number;
    max_tool_rounds?: number;
    max_tool_calls_per_message?: number;
    rag_top_k?: number;
    rag_min_score?: number;
    /** Categorias de produto habilitadas para o agente (detalhe). */
    product_category_ids?: string[];
    /**
     * Flags e políticas do agente (`jev_json`, já lido como objeto). A gravação substitui o JSON
     * inteiro, então quem altera uma chave precisa mandar as outras de volta (ver `roteamento.ts`).
     */
    jev?: Record<string, unknown>;
    version: number;
    created_at: number;
    updated_at: number;
};

/** Lista os agentes da org (primeira página, até 200). */
export async function listarAgentes(org: string): Promise<Agente[]> {
    const resposta = await pedir<{ items: Agente[]; next_cursor?: string | null }>(
        `/admin/v1/orgs/${encodeURIComponent(org)}/agents?limit=${LIMITE_AGENTES}`,
    );
    return resposta.items;
}

/** Lê um agente (nome, slug, situação, versão e vínculos). */
export async function obterAgente(org: string, id: string): Promise<Agente> {
    return pedir<Agente>(
        `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(id)}`,
    );
}
