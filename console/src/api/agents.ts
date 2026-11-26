
import { request } from "./client";

export const LIMIT_AGENTS = 200;

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
    temperature?: number | null;
    max_output_tokens?: number | null;
    max_context_tokens?: number;
    grouping_window_ms?: number;
    max_tool_rounds?: number;
    max_tool_calls_per_message?: number;
    rag_top_k?: number;
    rag_min_score?: number;
    product_category_ids?: string[];
    jev?: Record<string, unknown>;
    version: number;
    created_at: number;
    updated_at: number;
};

export async function listAgents(org: string): Promise<Agente[]> {
    const response = await request<{ items: Agente[]; next_cursor?: string | null }>(
        `/admin/v1/orgs/${encodeURIComponent(org)}/agents?limit=${LIMIT_AGENTS}`,
    );
    return response.items;
}

export async function getAgent(org: string, id: string): Promise<Agente> {
    return request<Agente>(
        `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(id)}`,
    );
}
