
import { request } from "./client";
import type { Agente } from "./agents";
import { translate } from "../i18n";

export const LIMIT_REASONS = 15;

export const LIMIT_REASON = 300;

export type Routing = {
    "defaultDepartment": string | null;
    "humanHandoff": boolean;
    "handoffDepartment": string | null;
    "reasons": string[];
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
        ? raw.filter((reason): reason is string => typeof reason === "string")
        : [];
    return {
        "defaultDepartment": agente.default_department_id,
        "humanHandoff": agente.human_handoff_enabled ?? false,
        "handoffDepartment": agente.human_handoff_department_id,
        "reasons": reasons,
    };
}

export function cleanReasons(reasons: string[]): string[] {
    return reasons.map((reason) => reason.trim()).filter((reason) => reason.length > 0);
}

export function validateRouting(value: Routing): Record<string, string> {
    const errors: Record<string, string> = {};
    if (value["humanHandoff"] && !value["handoffDepartment"]) {
        errors["handoffDepartment"] = translate("routing.validationHumanDepartment");
    }
    const cleaned = cleanReasons(value["reasons"]);
    if (cleaned.length > LIMIT_REASONS) {
        errors["reasons"] = translate("routing.validationTooManyReasons", {
            limit: LIMIT_REASONS,
            count: cleaned.length,
        });
    }
    value["reasons"].forEach((reason, index) => {
        if (reason.trim().length > LIMIT_REASON) {
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
