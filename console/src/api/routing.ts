
import { request } from "./client";
import type { Agente } from "./agents";
import { translate } from "../i18n";

export const LIMIT_REASONS = 15;

export const LIMIT_REASON = 300;

export type Routing = {
    "departamentoPadrao": string | null;
    "transferenciaHumana": boolean;
    "departamentoHumano": string | null;
    "motivos": string[];
};

export type RoutingChange = {
    department?: string;
    handoff?: boolean;
    handoff_department?: string;
    jev_json?: string;
};

export function agentRouting(agente: Agente): Routing {
    const raw = agente.jev?.["situacoes_de_handoff"];
    const reasons = Array.isArray(raw)
        ? raw.filter((motivo): motivo is string => typeof motivo === "string")
        : [];
    return {
        "departamentoPadrao": agente.default_department_id,
        "transferenciaHumana": agente.human_handoff_enabled ?? false,
        "departamentoHumano": agente.human_handoff_department_id,
        "motivos": reasons,
    };
}

export function cleanReasons(reasons: string[]): string[] {
    return reasons.map((motivo) => motivo.trim()).filter((motivo) => motivo.length > 0);
}

export function validateRouting(value: Routing): Record<string, string> {
    const errors: Record<string, string> = {};
    if (value["transferenciaHumana"] && !value["departamentoHumano"]) {
        errors["departamentoHumano"] = translate("routing.validationHumanDepartment");
    }
    const cleaned = cleanReasons(value["motivos"]);
    if (cleaned.length > LIMIT_REASONS) {
        errors["motivos"] = translate("routing.validationTooManyReasons", {
            limit: LIMIT_REASONS,
            count: cleaned.length,
        });
    }
    value["motivos"].forEach((motivo, index) => {
        if (motivo.trim().length > LIMIT_REASON) {
            errors[`motivo-${index}`] = translate("routing.validationReasonTooLong", {
                limit: LIMIT_REASON,
            });
        }
    });
    return errors;
}

export function jevWithReasons(jev: Record<string, unknown> | undefined, reasons: string[]): string {
    return JSON.stringify({ ...(jev ?? {}), "situacoes_de_handoff": reasons });
}

export async function saveRouting(
    org: string,
    agente: string,
    versao: number,
    change: RoutingChange,
): Promise<Agente> {
    return request<Agente>(
        `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}`,
        {
            method: "PATCH",
            headers: { "if-match": String(versao) },
            body: JSON.stringify(change),
        },
    );
}
