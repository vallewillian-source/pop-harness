
import { ApiError, request } from "./client";
import type { SchemaObject } from "../components/FormSchema";
import { translate } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";

export type Integracao = {
    id: string;
    kind: string;
    status: string;
    connection_state: string;
    version: number;
    "tem_segredo": boolean;
    configuracao?: Record<string, unknown> | null;
    "segredos_preenchidos"?: string[] | null;
};

export const INTEGRATION_STATUSES = [
    {
        "value": "active",
        get "label"() {
            return translate("integrations.statusActive");
        },
    },
    {
        "value": "inactive",
        get "label"() {
            return translate("integrations.statusInactive");
        },
    },
    {
        "value": "archived",
        get "label"() {
            return translate("integrations.statusArchived");
        },
    },
] as const;

export type IntegrationStatus = (typeof INTEGRATION_STATUSES)[number]["value"];

const CONNECTION_LABEL_KEYS: Record<string, TranslationKey> = {
    connected: "integrations.connectionConnected",
    disconnected: "integrations.connectionDisconnected",
    awaiting_scan: "integrations.connectionAwaitingScan",
    connecting: "integrations.connectionConnecting",
};

export function connectionLabel(estado: string): string {
    const key = CONNECTION_LABEL_KEYS[estado];
    return key ? translate(key) : estado;
}

export const KIND_WHATSAPP = "whatsapp";

export const LIMIT_INTEGRATION_LIST = 200;

export const LIMIT_KIND = 60;

function base(org: string, agente: string): string {
    return `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}/integrations`;
}

export async function listIntegrations(org: string, agente: string): Promise<Integracao[]> {
    const response = await request<{ items: Integracao[]; next_cursor?: string | null }>(
        `${base(org, agente)}?limit=${LIMIT_INTEGRATION_LIST}`,
    );
    return response.items;
}

export async function getIntegration(
    org: string,
    agente: string,
    id: string,
): Promise<Integracao> {
    return request<Integracao>(`${base(org, agente)}/${encodeURIComponent(id)}`);
}

export async function createIntegration(
    org: string,
    agente: string,
    kind: string,
    dados: IntegrationData = {},
): Promise<Integracao> {
    return request<Integracao>(base(org, agente), {
        method: "POST",
        body: JSON.stringify(integrationBody(kind, dados)),
    });
}

export type IntegrationData = {
    status?: IntegrationStatus;
    configuracao?: Record<string, unknown>;
    segredos?: Record<string, string>;
};

function integrationBody(
    kind: string | null,
    dados: IntegrationData,
): Record<string, unknown> {
    const body: Record<string, unknown> = {};
    if (kind !== null) body["kind"] = kind;
    if (dados.status !== undefined) body["status"] = dados.status;
    if (dados.configuracao !== undefined) body["config_json"] = dados.configuracao;
    if (dados.segredos !== undefined && Object.keys(dados.segredos).length > 0) {
        body["secrets"] = dados.segredos;
    }
    return body;
}

export async function updateIntegration(
    org: string,
    agente: string,
    id: string,
    versao: number,
    dados: IntegrationData,
): Promise<Integracao> {
    return request<Integracao>(`${base(org, agente)}/${encodeURIComponent(id)}`, {
        method: "PATCH",
        headers: { "if-match": String(versao) },
        body: JSON.stringify(integrationBody(null, dados)),
    });
}

export async function deleteIntegration(
    org: string,
    agente: string,
    id: string,
): Promise<void> {
    await request<{ deleted: boolean }>(`${base(org, agente)}/${encodeURIComponent(id)}`, {
        method: "DELETE",
    });
}


export type PluginManifest = {
    nome: string;
    kind: string;
    versao: string;
    configuracao: SchemaObject;
    segredos?: SchemaObject | null;
};

export async function getManifest(org: string, kind: string): Promise<PluginManifest> {
    return request<PluginManifest>(
        `/admin/v1/orgs/${encodeURIComponent(org)}/manifests/${encodeURIComponent(kind)}`,
    );
}

export function noManifestForType(failure: unknown): boolean {
    return failure instanceof ApiError && failure.status === 404;
}


export type WhatsappQr = {
    qrcode: {
        base64?: string | null;
        code?: string | null;
        count?: number | null;
    } | null;
    pairing_code?: string | null;
    state: string;
    connection_state: string;
    instance?: string;
    "expira_em"?: string | null;
};

export type WhatsappState = {
    state: string;
    connection_state: string;
    instance?: string;
};

export type WhatsappDisconnect = {
    connection_state: string;
    disconnected: boolean;
    instance?: string;
};

function whatsappBase(org: string, agente: string, id: string): string {
    return `${base(org, agente)}/${encodeURIComponent(id)}/whatsapp`;
}

export async function generateWhatsappQr(
    org: string,
    agente: string,
    id: string,
    versao: number,
    numero?: string,
): Promise<WhatsappQr> {
    const body = numero ? JSON.stringify({ number: numero }) : undefined;
    return request<WhatsappQr>(`${whatsappBase(org, agente, id)}/qrcode`, {
        method: "POST",
        headers: { "if-match": String(versao) },
        body: body,
    });
}

export async function readWhatsappState(
    org: string,
    agente: string,
    id: string,
): Promise<WhatsappState> {
    return request<WhatsappState>(`${whatsappBase(org, agente, id)}/state`);
}

export async function disconnectWhatsapp(
    org: string,
    agente: string,
    id: string,
    versao: number,
): Promise<WhatsappDisconnect> {
    return request<WhatsappDisconnect>(`${whatsappBase(org, agente, id)}/disconnect`, {
        method: "POST",
        headers: { "if-match": String(versao) },
    });
}

export function qrSource(base64: string): string {
    return base64.startsWith("data:") ? base64 : `data:image/png;base64,${base64}`;
}

export type WhatsappFailure = { texto: string; "code"?: string; "hint"?: string };

const WHATSAPP_TIP_KEYS: Record<string, TranslationKey> = {
    "whatsapp_nao_configurado": "integrations.whatsappTipNotConfigured",
    "evolution_indisponivel": "integrations.whatsappTipEvolutionUnavailable",
    "integracao_nao_e_whatsapp": "integrations.whatsappTipNotWhatsapp",
    "evolution_recusou": "integrations.whatsappTipEvolutionRefused",
    "evolution_sem_qr": "integrations.whatsappTipEvolutionNoQr",
    version_conflict: "integrations.whatsappTipVersionConflict",
    precondition_required: "integrations.whatsappTipPreconditionRequired",
};

export function explainWhatsappFailure(failure: unknown): WhatsappFailure {
    if (failure instanceof ApiError) {
        const tipKey = WHATSAPP_TIP_KEYS[failure["code"]];
        return {
            texto: failure.message,
            "code": failure["code"],
            "hint": tipKey ? translate(tipKey) : undefined,
        };
    }
    if (failure instanceof Error) {
        return { texto: failure.message };
    }
    return { texto: translate("integrations.whatsappUnexpectedFailure") };
}

export function validateIntegration(campos: {
    kind: string;
    situacao: string;
    "editing": boolean;
}): Record<string, string> {
    const errors: Record<string, string> = {};
    if (!campos["editing"]) {
        const kind = campos.kind.trim();
        if (kind === "") {
            errors["kind"] = translate("integrations.validationKindRequired");
        } else if (kind.length > LIMIT_KIND) {
            errors["kind"] = translate("integrations.validationKindMaxLength", { max: LIMIT_KIND });
        }
    }
    if (!INTEGRATION_STATUSES.some((situacao) => situacao["value"] === campos.situacao)) {
        errors["situacao"] = translate("integrations.validationStatusRequired");
    }
    return errors;
}
