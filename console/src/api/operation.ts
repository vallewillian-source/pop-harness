
import { translate } from "../i18n";
import { ApiError, request, type Health } from "./client";
import { getManifest, type PluginManifest } from "./integrations";

export const METRIC_NAMES = {
    "queueTurns": "poph_fila_turnos",
    "queueOutput": "poph_fila_saida",
    "activeConversations": "poph_conversas_ativas",
    "agentsLoaded": "poph_agentes_carregados",
    "processMemory": "poph_memoria_processo_bytes",
    "mainDatabase": "poph_harness_db_bytes",
    "databaseSearch": "poph_rag_db_bytes",
    "callsLlm": "poph_chamadas_llm_total",
    "errorsLlm": "poph_erros_llm_total",
    "callsJev": "poph_chamadas_jev_total",
    "errorsJev": "poph_erros_jev_total",
    "callsEvolution": "poph_chamadas_evolution_total",
    "errorsEvolution": "poph_erros_evolution_total",
    tokens: "poph_tokens_total",
} as const;

export type Measurements = Partial<Record<keyof typeof METRIC_NAMES, number>>;

export type InstanceHealth = {
    situacao: "ok" | "degradada";
    "health": Health;
};

async function searchText(path: string, accepts: string): Promise<{ status: number; texto: string }> {
    let response: Response;
    try {
        response = await fetch(path, { headers: { accept: accepts } });
    } catch {
        throw new ApiError(0, "rede", translate("operation.apiOffline"));
    }
    return { status: response.status, texto: await response.text() };
}

function readJson(texto: string): unknown {
    if (!texto) return undefined;
    try {
        return JSON.parse(texto) as unknown;
    } catch {
        return undefined;
    }
}

export async function fetchHealth(): Promise<InstanceHealth> {
    const { status, texto } = await searchText("/healthz", "application/json");
    const body = readJson(texto) as Partial<Health> | undefined;
    if (status === 200 || status === 503) {
        if (body && typeof body.version === "string") {
            return {
                situacao: status === 200 ? "ok" : "degradada",
                "health": body as Health,
            };
        }
        throw new ApiError(
            status,
            "healthz",
            translate("operation.apiHealthWithoutSummary", { status }),
        );
    }
    throw new ApiError(status, "healthz", translate("operation.apiHealthFailed", { status }));
}

export function parseMetrics(texto: string): Measurements {
    const byName = new Map<string, number>();
    for (const line of texto.split("\n")) {
        const cleared = line.trim();
        if (!cleared || cleared.startsWith("#")) continue;
        const parts = cleared.split(/\s+/);
        if (parts.length !== 2) continue;
        const nome = parts[0] ?? "";
        const valueText = parts[1] ?? "";
        if (nome.includes("{")) continue;
        const value = Number(valueText);
        if (!Number.isFinite(value)) continue;
        byName.set(nome, value);
    }
    const measurements: Measurements = {};
    for (const [chave, nome] of Object.entries(METRIC_NAMES) as [keyof Measurements, string][]) {
        const value = byName.get(nome);
        if (value !== undefined) measurements[chave] = value;
    }
    return measurements;
}

export async function fetchMetrics(): Promise<Measurements> {
    const { status, texto } = await searchText("/metrics", "text/plain");
    if (status !== 200) {
        throw new ApiError(status, "metrics", translate("operation.apiMetricsFailed", { status }));
    }
    return parseMetrics(texto);
}

export type PluginItem = {
    kind: string;
    situacao: "ok" | "indisponivel";
    nome?: string;
    versao?: string;
    mensagem?: string;
};

function defaultUnavailableMessage(): string {
    return translate("operation.apiPluginUnavailable");
}

export async function listPlugins(org: string): Promise<PluginItem[]> {
    const body = await request<{ items?: unknown } | undefined>(
        `/admin/v1/orgs/${encodeURIComponent(org)}/manifests`,
    );
    const rawItems: unknown[] = Array.isArray(body?.items) ? (body?.items as unknown[]) : [];
    const items: PluginItem[] = [];
    for (const raw of rawItems) {
        if (!raw || typeof raw !== "object") continue;
        const registro = raw as Record<string, unknown>;
        if (typeof registro.kind !== "string" || !registro.kind) continue;
        if (registro.situacao === "ok") {
            items.push({
                kind: registro.kind,
                situacao: "ok",
                nome: typeof registro.nome === "string" ? registro.nome : undefined,
                versao: typeof registro.versao === "string" ? registro.versao : undefined,
            });
        } else {
            items.push({
                kind: registro.kind,
                situacao: "indisponivel",
                mensagem:
                    typeof registro.mensagem === "string" && registro.mensagem
                        ? registro.mensagem
                        : defaultUnavailableMessage(),
            });
        }
    }
    return items;
}

export async function fetchPlugin(org: string, kind: string): Promise<PluginManifest> {
    return getManifest(org, kind);
}

export function manifestSummary(manifesto: PluginManifest): {
    configuracao: number;
    segredos: number;
} {
    return {
        configuracao: Object.keys(manifesto.configuracao?.properties ?? {}).length,
        segredos: Object.keys(manifesto.segredos?.properties ?? {}).length,
    };
}


export type ClusterMapSummary = {
    "modo": "cluster" | "instancia_unica";
    "localInstance": string;
    "instanceDefault": string | null;
    "secretConfigured": boolean;
    "instancias": { id: string; local: boolean }[];
    "quantityOrgs": number;
};

export type OrgSwitch = {
    org: string;
    de: string | null;
    para: string | null;
};

export type ReloadReport = {
    "mudou": boolean;
    "printBefore": string;
    "printAfter": string;
    "antes": ClusterMapSummary;
    "depois": ClusterMapSummary;
    "orgsChanged": OrgSwitch[];
    "instancesAdded": string[];
    "instancesRemoved": string[];
};

function asObject(value: unknown): Record<string, unknown> | null {
    return value !== null && typeof value === "object" && !Array.isArray(value)
        ? (value as Record<string, unknown>)
        : null;
}

function textOrNull(value: unknown): string | null {
    return typeof value === "string" && value !== "" ? value : null;
}

function textList(value: unknown): string[] {
    return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

function readSummary(raw: unknown): ClusterMapSummary {
    const registro = asObject(raw) ?? {};
    const instances = Array.isArray(registro["instancias"]) ? registro["instancias"] : [];
    const orgs = asObject(registro.orgs);
    return {
        "modo": registro["modo"] === "cluster" ? "cluster" : "instancia_unica",
        "localInstance": typeof registro.instance_id === "string" ? registro.instance_id : "",
        "instanceDefault": textOrNull(registro.default_instance),
        "secretConfigured": registro["segredo_configurado"] === true,
        "instancias": instances
            .map((item) => asObject(item))
            .filter((item): item is Record<string, unknown> => item !== null)
            .map((item) => ({
                id: typeof item.id === "string" ? item.id : "",
                local: item.local === true,
            })),
        "quantityOrgs": orgs ? Object.keys(orgs).length : 0,
    };
}

export function parseReport(body: unknown): ReloadReport {
    const registro = asObject(body);
    if (!registro || typeof registro["mudou"] !== "boolean" || !asObject(registro["antes"]) || !asObject(registro["depois"])) {
        throw new ApiError(200, "formato", translate("operation.apiClusterWithoutSummary"));
    }
    const trocas = Array.isArray(registro["orgs_que_trocaram"]) ? registro["orgs_que_trocaram"] : [];
    return {
        "mudou": registro["mudou"],
        "printBefore": typeof registro["fingerprint_antes"] === "string" ? registro["fingerprint_antes"] : "",
        "printAfter": typeof registro["fingerprint_depois"] === "string" ? registro["fingerprint_depois"] : "",
        "antes": readSummary(registro["antes"]),
        "depois": readSummary(registro["depois"]),
        "orgsChanged": trocas
            .map((item) => asObject(item))
            .filter((item): item is Record<string, unknown> => item !== null)
            .map((item) => ({
                org: typeof item.org === "string" ? item.org : "",
                de: textOrNull(item.de),
                para: textOrNull(item.para),
            })),
        "instancesAdded": textList(registro["instancias_adicionadas"]),
        "instancesRemoved": textList(registro["instancias_removidas"]),
    };
}

export async function reloadCluster(): Promise<ReloadReport> {
    const body = await request<unknown>("/admin/v1/cluster/reload", { method: "POST" });
    return parseReport(body);
}
