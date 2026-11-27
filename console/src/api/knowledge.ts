
import { request } from "./client";

export const LIMIT_ITEM_CONTENT = 1000;
export const LIMIT_ITEM_TITLE = 200;
export const LIMIT_GROUP_NAME = 120;

export type KnowledgeGroup = {
    id: string;
    org_id: string;
    name: string;
    description: string;
    status: string;
    version: number;
    created_at: number;
    updated_at: number;
    agents?: string[];
    index_warning?: string | null;
};

export type KnowledgeItem = {
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

export type IndexReport = {
    newItems: number;
    changedItems: number;
    unchangedItems: number;
    removedItems: number;
    embeddingsGenerated: number;
    passagesWritten: number;
};

export type ReindexResult = {
    agent: string;
    indexed: IndexReport;
};

export type GroupInput = {
    name: string;
    description?: string;
};

export type GroupChange = {
    name?: string;
    description?: string;
    status?: string;
};

export type ItemInput = {
    title: string;
    content: string;
    tags?: string[];
    position?: number;
};

export type ItemChange = {
    title?: string;
    content?: string;
    tags?: string[];
    position?: number;
    status?: string;
};

const LIMIT_LIST = 200;

function baseGroup(org: string): string {
    return `/admin/v1/orgs/${encodeURIComponent(org)}/knowledge`;
}

function itemBase(org: string, group: string): string {
    return `${baseGroup(org)}/${encodeURIComponent(group)}/items`;
}

async function allPages<T>(basePath: string): Promise<T[]> {
    const items: T[] = [];
    let cursor: string | null = null;
    do {
        const parts = [`limit=${LIMIT_LIST}`];
        if (cursor) parts.push(`cursor=${encodeURIComponent(cursor)}`);
        const response = await request<{ items: T[]; next_cursor: string | null }>(
            `${basePath}${basePath.includes("?") ? "&" : "?"}${parts.join("&")}`,
        );
        items.push(...response.items);
        cursor = response.next_cursor ?? null;
    } while (cursor);
    return items;
}

export async function listGroups(org: string): Promise<KnowledgeGroup[]> {
    return allPages<KnowledgeGroup>(baseGroup(org));
}

export async function getGroup(org: string, id: string): Promise<KnowledgeGroup> {
    return request<KnowledgeGroup>(`${baseGroup(org)}/${encodeURIComponent(id)}`);
}

export async function createGroup(
    org: string,
    input: GroupInput,
): Promise<KnowledgeGroup> {
    const body: Record<string, unknown> = { name: input.name };
    if (input.description && input.description.trim()) {
        body["description"] = input.description;
    }
    return request<KnowledgeGroup>(baseGroup(org), {
        method: "POST",
        body: JSON.stringify(body),
    });
}

export async function updateGroup(
    org: string,
    id: string,
    versao: number,
    change: GroupChange,
): Promise<KnowledgeGroup> {
    const body: Record<string, unknown> = {};
    if (change.name !== undefined) body["name"] = change.name;
    if (change.description !== undefined) body["description"] = change.description;
    if (change.status !== undefined) body["status"] = change.status;
    return request<KnowledgeGroup>(`${baseGroup(org)}/${encodeURIComponent(id)}`, {
        method: "PATCH",
        headers: { "if-match": String(versao) },
        body: JSON.stringify(body),
    });
}

export async function deleteGroup(org: string, id: string): Promise<void> {
    await request<void>(`${baseGroup(org)}/${encodeURIComponent(id)}?force=true`, {
        method: "DELETE",
    });
}

export async function listItems(
    org: string,
    group: string,
): Promise<KnowledgeItem[]> {
    return allPages<KnowledgeItem>(itemBase(org, group));
}

export async function createItem(
    org: string,
    group: string,
    input: ItemInput,
): Promise<KnowledgeItem> {
    const body: Record<string, unknown> = {
        title: input.title,
        content: input.content,
    };
    if (input.tags && input.tags.length > 0) body["tags"] = input.tags;
    if (input.position !== undefined) body["position"] = input.position;
    return request<KnowledgeItem>(itemBase(org, group), {
        method: "POST",
        body: JSON.stringify(body),
    });
}

export async function updateItem(
    org: string,
    group: string,
    id: string,
    versao: number,
    change: ItemChange,
): Promise<KnowledgeItem> {
    const body: Record<string, unknown> = {};
    if (change.title !== undefined) body["title"] = change.title;
    if (change.content !== undefined) body["content"] = change.content;
    if (change.tags !== undefined) body["tags"] = change.tags;
    if (change.position !== undefined) body["position"] = change.position;
    if (change.status !== undefined) body["status"] = change.status;
    return request<KnowledgeItem>(`${itemBase(org, group)}/${encodeURIComponent(id)}`, {
        method: "PATCH",
        headers: { "if-match": String(versao) },
        body: JSON.stringify(body),
    });
}

export async function deleteItem(
    org: string,
    group: string,
    id: string,
): Promise<void> {
    await request<void>(`${itemBase(org, group)}/${encodeURIComponent(id)}`, {
        method: "DELETE",
    });
}

function baseLink(org: string, agente: string, group: string): string {
    return `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}/knowledge/${encodeURIComponent(group)}`;
}

export async function linkGroup(
    org: string,
    agente: string,
    group: string,
): Promise<void> {
    await request<unknown>(baseLink(org, agente, group), { method: "POST" });
}

export async function unlinkGroup(
    org: string,
    agente: string,
    group: string,
): Promise<void> {
    await request<unknown>(baseLink(org, agente, group), { method: "DELETE" });
}

function countOrZero(value: unknown): number {
    return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function readIndexReport(raw: unknown): IndexReport {
    const body = (raw ?? {}) as Record<string, unknown>;
    return {
        newItems: countOrZero(body["itens_novos"]),
        changedItems: countOrZero(body["itens_alterados"]),
        unchangedItems: countOrZero(body["itens_inalterados"]),
        removedItems: countOrZero(body["itens_removidos"]),
        embeddingsGenerated: countOrZero(body["embeddings_gerados"]),
        passagesWritten: countOrZero(body["trechos_gravados"]),
    };
}

export async function reindexAgent(
    org: string,
    agente: string,
): Promise<ReindexResult> {
    const raw = await request<Record<string, unknown>>(
        `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}/knowledge/reindex`,
        { method: "POST" },
    );
    const agent = raw["agent"];
    return {
        agent: typeof agent === "string" ? agent : "",
        indexed: readIndexReport(raw["indexed"]),
    };
}

export function readTags(texto: string): string[] {
    const seen = new Set<string>();
    for (const part of texto.split(",")) {
        const tag = part.trim();
        if (tag) seen.add(tag);
    }
    return [...seen];
}

export function writeTags(tags: string[]): string {
    return tags.join(", ");
}
