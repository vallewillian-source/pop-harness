
import { request } from "./client";
import type { components } from "./schema";
import { formatNumber } from "../i18n/format";

export type LibraryFile = components["schemas"]["ArquivoJson"];

export const LIMIT_FILE_BYTES = 16 * 1024 * 1024;

export const LIMIT_FILE_NAME = 200;

export const LIMIT_FILE_DESCRIPTION = 500;

export const LIMIT_FILE_LIST = 200;

export function sizeInMb(bytes: number): string {
    const megabytes = formatNumber(bytes / (1024 * 1024), {
        minimumFractionDigits: 1,
        maximumFractionDigits: 1,
    });
    return `${megabytes} MB`;
}

function base(org: string, agente: string): string {
    return `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}/files`;
}

export async function listFiles(org: string, agente: string): Promise<LibraryFile[]> {
    const response = await request<{ items: LibraryFile[]; next_cursor?: string | null }>(
        `${base(org, agente)}?limit=${LIMIT_FILE_LIST}`,
    );
    return response.items;
}

export async function sendFile(
    org: string,
    agente: string,
    file: File,
    nome: string,
    descricao: string,
): Promise<LibraryFile> {
    const parameters = new URLSearchParams({ filename: file.name });
    if (nome.trim()) parameters.set("name", nome.trim());
    if (descricao.trim()) parameters.set("description", descricao.trim());
    return request<LibraryFile>(`${base(org, agente)}?${parameters.toString()}`, {
        method: "POST",
        headers: { "content-type": "application/octet-stream" },
        body: file,
    });
}

export async function deleteFile(org: string, agente: string, id: string): Promise<void> {
    await request<unknown>(`${base(org, agente)}/${encodeURIComponent(id)}`, { method: "DELETE" });
}
