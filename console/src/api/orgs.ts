
import { ApiError, request, type Org } from "./client";

const RETENTION_KEY = "retention_days";

export type OrgInput = {
    name: string;
    slug?: string;
    config?: Record<string, unknown>;
};

export type OrgChange = {
    name?: string;
    status?: string;
    config?: Record<string, unknown>;
};

export function retentionDays(org: Pick<Org, "config">): number | undefined {
    const config = org.config as Record<string, unknown> | undefined;
    const value = config?.[RETENTION_KEY];
    return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

export function withRetention(
    config: Record<string, unknown> | undefined,
    dias: number | null,
): Record<string, unknown> {
    const copy: Record<string, unknown> = { ...(config ?? {}) };
    if (dias === null) {
        delete copy[RETENTION_KEY];
    } else {
        copy[RETENTION_KEY] = dias;
    }
    return copy;
}

export async function createOrg(input: OrgInput): Promise<Org> {
    const body: Record<string, unknown> = { name: input.name };
    if (input.slug) body["slug"] = input.slug;
    if (input.config && Object.keys(input.config).length > 0) {
        body["config"] = input.config;
    }
    return request<Org>("/admin/v1/orgs", { method: "POST", body: JSON.stringify(body) });
}

export async function getOrg(id: string): Promise<Org> {
    return request<Org>(`/admin/v1/orgs/${encodeURIComponent(id)}`);
}

export async function updateOrg(
    id: string,
    versao: number,
    change: OrgChange,
): Promise<Org> {
    const body: Record<string, unknown> = {};
    if (change.name !== undefined) body["name"] = change.name;
    if (change.status !== undefined) body["status"] = change.status;
    if (change.config !== undefined) body["config"] = change.config;
    return request<Org>(`/admin/v1/orgs/${encodeURIComponent(id)}`, {
        method: "PATCH",
        headers: { "if-match": String(versao) },
        body: JSON.stringify(body),
    });
}

export async function deleteOrg(id: string, force: boolean): Promise<void> {
    const query = force ? "?force=true" : "";
    await request<void>(`/admin/v1/orgs/${encodeURIComponent(id)}${query}`, {
        method: "DELETE",
    });
}

export type VersionConflict = {
    "expected"?: number;
    "current"?: number;
};

export function versionConflict(error: unknown): VersionConflict | null {
    if (!(error instanceof ApiError) || error.status !== 412) return null;
    const details = error["details"] as Record<string, unknown> | null | undefined;
    const expected = details?.["expected_version"];
    const current = details?.["current_version"];
    return {
        "expected": typeof expected === "number" ? expected : undefined,
        "current": typeof current === "number" ? current : undefined,
    };
}
