
import { request } from "./client";
import type { components } from "./schema";
import { listAgents, type Agente } from "./agents";

export type AgentSummary = Agente;
export { listAgents };

export type Category = components["schemas"]["CategoriaJson"];
export type Departamento = components["schemas"]["DepartamentoJson"];
export type Registro = Category | Departamento;
export type StructureType = "categoria" | "departamento";

const TRILHO: Record<StructureType, string> = {
    "categoria": "categories",
    departamento: "departments",
};

const LIMIT_LIST = 200;

function path(org: string, tipo: StructureType, id?: string): string {
    const base = `/admin/v1/orgs/${encodeURIComponent(org)}/${TRILHO[tipo]}`;
    return id === undefined ? base : `${base}/${encodeURIComponent(id)}`;
}

export type StructureInput = {
    name: string;
    description?: string;
};

export type StructureChange = {
    name?: string;
    description?: string;
    status?: string;
};

export async function listStructure(
    tipo: StructureType,
    org: string,
): Promise<Registro[]> {
    const response = await request<{ items: Registro[] }>(
        `${path(org, tipo)}?limit=${LIMIT_LIST}`,
    );
    return response.items;
}

export async function getStructure(
    tipo: StructureType,
    org: string,
    id: string,
): Promise<Registro> {
    return request<Registro>(path(org, tipo, id));
}

export async function createStructure(
    tipo: StructureType,
    org: string,
    input: StructureInput,
): Promise<Registro> {
    const body: Record<string, unknown> = { name: input.name };
    if (input.description && input.description.trim()) {
        body["description"] = input.description;
    }
    return request<Registro>(path(org, tipo), {
        method: "POST",
        body: JSON.stringify(body),
    });
}

export async function updateStructure(
    tipo: StructureType,
    org: string,
    id: string,
    versao: number,
    change: StructureChange,
): Promise<Registro> {
    const body: Record<string, unknown> = {};
    if (change.name !== undefined) body["name"] = change.name;
    if (change.description !== undefined) body["description"] = change.description;
    if (change.status !== undefined) body["status"] = change.status;
    return request<Registro>(path(org, tipo, id), {
        method: "PATCH",
        headers: { "if-match": String(versao) },
        body: JSON.stringify(body),
    });
}

export async function deleteDepartment(org: string, id: string): Promise<void> {
    await request<void>(path(org, "departamento", id), { method: "DELETE" });
}

export async function deleteCategory(org: string, id: string, forcar: boolean): Promise<void> {
    const suffix = forcar ? "?force=true" : "";
    await request<void>(`${path(org, "categoria", id)}${suffix}`, { method: "DELETE" });
}

async function allPages<T>(basePath: string, withLimit = true): Promise<T[]> {
    const items: T[] = [];
    let cursor: string | null = null;
    do {
        const parts: string[] = [];
        if (withLimit) parts.push(`limit=${LIMIT_LIST}`);
        if (cursor) parts.push(`cursor=${encodeURIComponent(cursor)}`);
        const suffix = parts.length > 0 ? `${basePath.includes("?") ? "&" : "?"}${parts.join("&")}` : "";
        const response: { items: T[]; next_cursor: string | null } = await request(
            `${basePath}${suffix}`,
        );
        items.push(...response.items);
        cursor = response.next_cursor ?? null;
    } while (cursor);
    return items;
}

export type CategoryProduct = { id: string; name: string };

export async function categoryProducts(org: string, categoryId: string): Promise<CategoryProduct[]> {
    const all = await allPages<{ id: string; name: string; category_id: string | null }>(
        `/admin/v1/orgs/${encodeURIComponent(org)}/products?all=true`,
        false,
    );
    return all
        .filter((product) => product.category_id === categoryId)
        .map((product) => ({ id: product.id, name: product.name }));
}

export async function agentsUsingCategory(org: string, categoryId: string): Promise<AgentSummary[]> {
    const agents = await allPages<AgentSummary>(
        `/admin/v1/orgs/${encodeURIComponent(org)}/agents`,
    );
    const details = await Promise.all(
        agents.map((agente) =>
            request<AgentSummary>(
                `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente.id)}`,
            ),
        ),
    );
    return details.filter((agente) => agente.product_category_ids?.includes(categoryId));
}


export function agentsUsing(agents: AgentSummary[], departmentId: string): AgentSummary[] {
    return agents.filter(
        (agente) =>
            agente.default_department_id === departmentId ||
            agente.human_handoff_department_id === departmentId,
    );
}
