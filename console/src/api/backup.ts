
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
    const { blob, "cabecalhos": headers } = await requestBytes(exportPath(org, withConversations));
    const nome = headerName(headers.get("content-disposition")) ?? `organizacao-${org}-backup.json`;
    return { blob, nome };
}

export type ImportReport = {
    org: { id: string; "criada": boolean };
    com_conversas: boolean;
    tabelas: Record<string, { "inseridas": number; "atualizadas": number }>;
    total: number;
    "segredos_a_redigitar": number;
    "indice": {
        "base_conhecimento": { "agentes": number; "trechos": number };
        "produtos": { "trechos": number };
    };
    "aviso": string | null;
};

export async function importOrg(
    org: string,
    texto: string,
    sobrescrever = false,
): Promise<ImportReport> {
    return request<ImportReport>(importPath(org, sobrescrever), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: texto,
    });
}

export function readFileText(file: Blob): Promise<string> {
    return new Promise((resolver, rejeitar) => {
        const reader = new FileReader();
        reader.onload = () => resolver(typeof reader.result === "string" ? reader.result : "");
        reader.onerror = () =>
            rejeitar(reader.error ?? new Error(translate("orgs.backupReadFileError")));
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
    "titulo": string;
    texto: string;
    "conflitos"?: { "itens": string[]; "mais": number };
    "versoes"?: { "arquivo": number; "instalacao": number };
    "podeTentarDeNovo": boolean;
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
    "linhas": number;
    "exemplos": string[];
};

export const SHOWN_EXAMPLES = 5;

export function overwriteRefusal(error: unknown): OverwriteRefusal | null {
    if (!(error instanceof ApiError)) return null;
    if (error.status !== 409 || error["codigo"] !== "importacao_sobrescreve") return null;

    const details = error["detalhes"] && typeof error["detalhes"] === "object"
        ? (error["detalhes"] as Record<string, unknown>)
        : {};
    const raw = details.tabelas && typeof details.tabelas === "object"
        ? (details.tabelas as Record<string, unknown>)
        : {};
    const tabelas: Record<string, number> = {};
    for (const [table, value] of Object.entries(raw)) {
        if (typeof value === "number" && Number.isFinite(value) && value > 0) tabelas[table] = value;
    }
    const sumOfTables = Object.values(tabelas).reduce((sum, value) => sum + value, 0);
    const lines = detailNumber(details, "linhas") ?? sumOfTables;
    const examples = detailList(details, "exemplos").slice(0, SHOWN_EXAMPLES);
    return { tabelas, "linhas": lines, "exemplos": examples };
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

export function backupFailure(error: unknown, operation: "exportar" | "importar"): BackupFailure {
    if (!(error instanceof ApiError)) {
        return {
            "titulo": translate(operation === "exportar" ? "orgs.backupExportFailed" : "orgs.backupImportFailed"),
            texto: translate("orgs.backupGeneric"),
            "podeTentarDeNovo": true,
        };
    }

    if (error["semServidor"]) {
        return {
            "titulo": translate(operation === "exportar" ? "orgs.backupExportFailed" : "orgs.backupImportFailed"),
            texto: translate("orgs.backupOffline"),
            "podeTentarDeNovo": true,
        };
    }

    if (error.status === 401) {
        return {
            "titulo": translate("orgs.backupSessionTitle"),
            texto: translate("orgs.backupSessionText"),
            "podeTentarDeNovo": false,
        };
    }

    if (error.status === 403) {
        return {
            "titulo": translate(operation === "exportar" ? "orgs.backupExportFailed" : "orgs.backupImportFailed"),
            texto: translate("orgs.backupForbiddenText"),
            "podeTentarDeNovo": true,
        };
    }

    if (error.status === 404) {
        return {
            "titulo": translate("orgs.backupNotFoundTitle"),
            texto: translate("orgs.backupNotFoundText"),
            "podeTentarDeNovo": false,
        };
    }

    if (error.status === 413 || error["codigo"] === "payload_too_large") {
        return {
            "titulo": translate("orgs.backupTooLargeTitle"),
            texto: translate("orgs.backupTooLargeText"),
            "podeTentarDeNovo": false,
        };
    }

    if (error["codigo"] === "backup_incompleto") {
        return {
            "titulo": translate("orgs.backupExportFailed"),
            texto: translate("orgs.backupIncompleteText"),
            "podeTentarDeNovo": false,
        };
    }

    if (error.status === 409 && error["codigo"] === "importacao_sobrescreve") {
        return {
            "titulo": translate("orgs.backupOverwriteTitle"),
            texto: translate("orgs.backupOverwriteText"),
            "podeTentarDeNovo": false,
        };
    }

    if (error.status === 409 && error["codigo"] === "conflict") {
        const conflicts = detailList(error["detalhes"], "conflicts");
        const more = detailNumber(error["detalhes"], "conflicts_more") ?? 0;
        return {
            "titulo": translate("orgs.backupConflictTitle"),
            texto: translate("orgs.backupConflictText"),
            "conflitos": {
                "itens": conflicts.slice(0, SHOWN_CONFLICTS).map(conflictText),
                "mais": more + Math.max(0, conflicts.length - SHOWN_CONFLICTS),
            },
            "podeTentarDeNovo": false,
        };
    }

    if (error.status === 400 && error["codigo"] === "invalid_backup") {
        const file = detailNumber(error["detalhes"], "version");
        const installation = detailNumber(error["detalhes"], "supported_version");
        return {
            "titulo": translate("orgs.backupInvalidTitle"),
            texto: translate("orgs.backupInvalidText"),
            "versoes":
                file !== undefined && installation !== undefined
                    ? { "arquivo": file, "instalacao": installation }
                    : undefined,
            "podeTentarDeNovo": false,
        };
    }

    if (error.status >= 500) {
        return {
            "titulo": translate(operation === "exportar" ? "orgs.backupExportFailed" : "orgs.backupImportFailed"),
            texto: translate("orgs.backupServerError", { status: error.status }),
            "podeTentarDeNovo": true,
        };
    }

    return {
        "titulo": translate(operation === "exportar" ? "orgs.backupExportFailed" : "orgs.backupImportFailed"),
        texto: translate("orgs.backupGeneric"),
        "podeTentarDeNovo": true,
    };
}
