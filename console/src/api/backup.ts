
import { ApiError, request, requestBytes } from "./client";
import { headerName } from "./testFile";
import { sizeInMb } from "./files";
import { translate } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";

export { sizeInMb };

export const LIMIT_IMPORT_BYTES = 64 * 1024 * 1024;

export const SHOWN_CONFLICTS = 5;

export function exportPath(org: string, withConversations: boolean): string {
    const base = `/admin/v1/orgs/${encodeURIComponent(org)}/backup`;
    return withConversations ? `${base}?com_conversas=true` : base;
}

export function importPath(org: string, sobrescrever = false): string {
    const base = `/admin/v1/orgs/${encodeURIComponent(org)}/backup/import`;
    return sobrescrever ? `${base}?sobrescrever=true` : base;
}

export type ExportedFile = { blob: Blob; nome: string };

export async function exportOrg(org: string, withConversations: boolean): Promise<ExportedFile> {
    const { blob, headers } = await requestBytes(exportPath(org, withConversations));
    const nome = headerName(headers.get("content-disposition")) ?? `organizacao-${org}-backup.json`;
    return { blob, nome };
}

export type ImportReport = {
    org: { id: string; created: boolean };
    withConversations: boolean;
    tables: Record<string, { inserted: number; updated: number }>;
    total: number;
    secretsToRedo: number;
    index: {
        knowledgeBase: { agents: number; passages: number };
        products: { passages: number };
    };
    notice: string | null;
};

function countOrZero(value: unknown): number {
    return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function readImportReport(raw: unknown): ImportReport {
    const body = (raw ?? {}) as Record<string, unknown>;
    const org = (body["org"] ?? {}) as Record<string, unknown>;
    const tables = (body["tabelas"] ?? {}) as Record<string, unknown>;
    const index = (body["indice"] ?? {}) as Record<string, unknown>;
    const knowledge = (index["base_conhecimento"] ?? {}) as Record<string, unknown>;
    const products = (index["produtos"] ?? {}) as Record<string, unknown>;
    const counts: ImportReport["tables"] = {};
    for (const [table, value] of Object.entries(tables)) {
        const row = (value ?? {}) as Record<string, unknown>;
        counts[table] = {
            inserted: countOrZero(row["inseridas"]),
            updated: countOrZero(row["atualizadas"]),
        };
    }
    return {
        org: {
            id: typeof org.id === "string" ? org.id : "",
            created: org["criada"] === true,
        },
        withConversations: body["com_conversas"] === true,
        tables: counts,
        total: countOrZero(body["total"]),
        secretsToRedo: countOrZero(body["segredos_a_redigitar"]),
        index: {
            knowledgeBase: {
                agents: countOrZero(knowledge["agentes"]),
                passages: countOrZero(knowledge["trechos"]),
            },
            products: { passages: countOrZero(products["trechos"]) },
        },
        notice: typeof body["aviso"] === "string" ? body["aviso"] : null,
    };
}

export async function importOrg(
    org: string,
    texto: string,
    sobrescrever = false,
): Promise<ImportReport> {
    const raw = await request<unknown>(importPath(org, sobrescrever), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: texto,
    });
    return readImportReport(raw);
}

export function readFileText(file: Blob): Promise<string> {
    return new Promise((resolver, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolver(typeof reader.result === "string" ? reader.result : "");
        reader.onerror = () =>
            reject(reader.error ?? new Error(translate("orgs.backupReadFileError")));
        reader.readAsText(file);
    });
}

export function downloadFile(file: ExportedFile): void {
    const url = URL.createObjectURL(file.blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = file.nome;
    link.rel = "noopener";
    document.body.appendChild(link);
    link.click();
    link.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 1_000);
}

export function exceedsLimit(bytes: number): boolean {
    return bytes > LIMIT_IMPORT_BYTES;
}

export const TABLE_NAMES: Record<string, TranslationKey> = {
    agents: "orgs.tableAgents",
    agent_instructions: "orgs.tableAgentInstructions",
    departments: "orgs.tableDepartments",
    product_categories: "orgs.tableProductCategories",
    agent_product_categories: "orgs.tableAgentProductCategories",
    products: "orgs.tableProducts",
    knowledge_groups: "orgs.tableKnowledgeGroups",
    agent_knowledge_groups: "orgs.tableAgentKnowledgeGroups",
    knowledge_items: "orgs.tableKnowledgeItems",
    file_library: "orgs.tableFileLibrary",
    integrations: "orgs.tableIntegrations",
    conversations: "orgs.tableConversations",
    messages: "orgs.tableMessages",
    turns: "orgs.tableTurns",
    attachments: "orgs.tableAttachments",
    outbox: "orgs.tableOutbox",
    inbox: "orgs.tableInbox",
    carts: "orgs.tableCarts",
    cart_items: "orgs.tableCartItems",
};

export function tableName(table: string): string {
    const key = Object.prototype.hasOwnProperty.call(TABLE_NAMES, table)
        ? TABLE_NAMES[table]
        : undefined;
    return key === undefined ? table : translate(key);
}

export function conflictText(item: string): string {
    const clean = item.replace(/`/g, "").trim();
    const match = /^([a-z_]+)\s+(.*)$/.exec(clean);
    if (!match) return clean;
    const [, table = "", rest = ""] = match;
    if (!Object.prototype.hasOwnProperty.call(TABLE_NAMES, table)) return clean;
    return `${tableName(table)}: ${rest}`;
}

export type BackupFailure = {
    "title": string;
    texto: string;
    "conflicts"?: { "items": string[]; "more": number };
    "versions"?: { "file": number; "installation": number };
    "canRetry": boolean;
};

function detailNumber(details: unknown, campo: string): number | undefined {
    if (!details || typeof details !== "object") return undefined;
    const value = (details as Record<string, unknown>)[campo];
    return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function detailList(details: unknown, campo: string): string[] {
    if (!details || typeof details !== "object") return [];
    const value = (details as Record<string, unknown>)[campo];
    return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

export type OverwriteRefusal = {
    tabelas: Record<string, number>;
    "lines": number;
    "examples": string[];
};

export const SHOWN_EXAMPLES = 5;

export function overwriteRefusal(error: unknown): OverwriteRefusal | null {
    if (!(error instanceof ApiError)) return null;
    if (error.status !== 409 || error["code"] !== "importacao_sobrescreve") return null;

    const details = error["details"] && typeof error["details"] === "object"
        ? (error["details"] as Record<string, unknown>)
        : {};
    const raw = details["tabelas"] && typeof details["tabelas"] === "object"
        ? (details["tabelas"] as Record<string, unknown>)
        : {};
    const tabelas: Record<string, number> = {};
    for (const [table, value] of Object.entries(raw)) {
        if (typeof value === "number" && Number.isFinite(value) && value > 0) tabelas[table] = value;
    }
    const sumOfTables = Object.values(tabelas).reduce((sum, value) => sum + value, 0);
    const lines = detailNumber(details, "linhas") ?? sumOfTables;
    const examples = detailList(details, "exemplos").slice(0, SHOWN_EXAMPLES);
    return { tabelas, "lines": lines, "examples": examples };
}

export function sampleText(item: string): string {
    const clean = item.replace(/`/g, "").trim();
    const index = clean.indexOf(":");
    if (index <= 0) return clean;
    const table = clean.slice(0, index).trim();
    const rest = clean.slice(index + 1).trim();
    if (!Object.prototype.hasOwnProperty.call(TABLE_NAMES, table)) return clean;
    return `${tableName(table)}: ${rest}`;
}

export function backupFailure(error: unknown, operation: "export" | "import"): BackupFailure {
    if (!(error instanceof ApiError)) {
        return {
            "title": translate(operation === "export" ? "orgs.backupExportFailed" : "orgs.backupImportFailed"),
            texto: translate("orgs.backupGeneric"),
            "canRetry": true,
        };
    }

    if (error["noServer"]) {
        return {
            "title": translate(operation === "export" ? "orgs.backupExportFailed" : "orgs.backupImportFailed"),
            texto: translate("orgs.backupOffline"),
            "canRetry": true,
        };
    }

    if (error.status === 401) {
        return {
            "title": translate("orgs.backupSessionTitle"),
            texto: translate("orgs.backupSessionText"),
            "canRetry": false,
        };
    }

    if (error.status === 403) {
        return {
            "title": translate(operation === "export" ? "orgs.backupExportFailed" : "orgs.backupImportFailed"),
            texto: translate("orgs.backupForbiddenText"),
            "canRetry": true,
        };
    }

    if (error.status === 404) {
        return {
            "title": translate("orgs.backupNotFoundTitle"),
            texto: translate("orgs.backupNotFoundText"),
            "canRetry": false,
        };
    }

    if (error.status === 413 || error["code"] === "payload_too_large") {
        return {
            "title": translate("orgs.backupTooLargeTitle"),
            texto: translate("orgs.backupTooLargeText"),
            "canRetry": false,
        };
    }

    if (error["code"] === "backup_incompleto") {
        return {
            "title": translate("orgs.backupExportFailed"),
            texto: translate("orgs.backupIncompleteText"),
            "canRetry": false,
        };
    }

    if (error.status === 409 && error["code"] === "importacao_sobrescreve") {
        return {
            "title": translate("orgs.backupOverwriteTitle"),
            texto: translate("orgs.backupOverwriteText"),
            "canRetry": false,
        };
    }

    if (error.status === 409 && error["code"] === "conflict") {
        const conflicts = detailList(error["details"], "conflicts");
        const more = detailNumber(error["details"], "conflicts_more") ?? 0;
        return {
            "title": translate("orgs.backupConflictTitle"),
            texto: translate("orgs.backupConflictText"),
            "conflicts": {
                "items": conflicts.slice(0, SHOWN_CONFLICTS).map(conflictText),
                "more": more + Math.max(0, conflicts.length - SHOWN_CONFLICTS),
            },
            "canRetry": false,
        };
    }

    if (error.status === 400 && error["code"] === "invalid_backup") {
        const file = detailNumber(error["details"], "version");
        const installation = detailNumber(error["details"], "supported_version");
        return {
            "title": translate("orgs.backupInvalidTitle"),
            texto: translate("orgs.backupInvalidText"),
            "versions":
                file !== undefined && installation !== undefined
                    ? { "file": file, "installation": installation }
                    : undefined,
            "canRetry": false,
        };
    }

    if (error.status >= 500) {
        return {
            "title": translate(operation === "export" ? "orgs.backupExportFailed" : "orgs.backupImportFailed"),
            texto: translate("orgs.backupServerError", { status: error.status }),
            "canRetry": true,
        };
    }

    return {
        "title": translate(operation === "export" ? "orgs.backupExportFailed" : "orgs.backupImportFailed"),
        texto: translate("orgs.backupGeneric"),
        "canRetry": true,
    };
}
