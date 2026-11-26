
import { request } from "./client";
import type { Agente } from "./agents";
import { translate } from "../i18n";

export const GATES = ["g1", "g2", "g3", "g4", "g5", "g6", "g7", "g8"] as const;
export type IdGate = (typeof GATES)[number];

export type GateState = "herdar" | "ligado" | "desligado";

export const FACTORY_DEFAULTS: Record<IdGate, boolean> = {
    g1: false,
    g2: false,
    g3: true,
    g4: false,
    g5: true,
    g6: false,
    g7: false,
    g8: true,
};

export const GATE_DESCRIPTION: Record<IdGate, { "title": string; "explanation": string }> = {
    g1: {
        get "title"() {
            return translate("modelJev.gateG1Title");
        },
        get "explanation"() {
            return translate("modelJev.gateG1Description");
        },
    },
    g2: {
        get "title"() {
            return translate("modelJev.gateG2Title");
        },
        get "explanation"() {
            return translate("modelJev.gateG2Description");
        },
    },
    g3: {
        get "title"() {
            return translate("modelJev.gateG3Title");
        },
        get "explanation"() {
            return translate("modelJev.gateG3Description");
        },
    },
    g4: {
        get "title"() {
            return translate("modelJev.gateG4Title");
        },
        get "explanation"() {
            return translate("modelJev.gateG4Description");
        },
    },
    g5: {
        get "title"() {
            return translate("modelJev.gateG5Title");
        },
        get "explanation"() {
            return translate("modelJev.gateG5Description");
        },
    },
    g6: {
        get "title"() {
            return translate("modelJev.gateG6Title");
        },
        get "explanation"() {
            return translate("modelJev.gateG6Description");
        },
    },
    g7: {
        get "title"() {
            return translate("modelJev.gateG7Title");
        },
        get "explanation"() {
            return translate("modelJev.gateG7Description");
        },
    },
    g8: {
        get "title"() {
            return translate("modelJev.gateG8Title");
        },
        get "explanation"() {
            return translate("modelJev.gateG8Description");
        },
    },
};

export const PRESERVED_KEYS: ReadonlyArray<{ chave: string; "explanation": string; "ondeEdita"?: string }> = [
    {
        chave: "situacoes_de_handoff",
        get "explanation"() {
            return translate("modelJev.preservedHandoffDescription");
        },
        get "ondeEdita"() {
            return translate("modelJev.preservedHandoffWhere");
        },
    },
    {
        chave: "despedida",
        get "explanation"() {
            return translate("modelJev.preservedFarewellDescription");
        },
    },
    {
        chave: "resposta_segura",
        get "explanation"() {
            return translate("modelJev.preservedSafeReplyDescription");
        },
    },
    {
        chave: "g7_acao",
        get "explanation"() {
            return translate("modelJev.preservedG7ActionDescription");
        },
    },
    {
        chave: "g7_resposta",
        get "explanation"() {
            return translate("modelJev.preservedG7ReplyDescription");
        },
    },
];

export const LIMIT_WINDOW_MS = 8000;
export const LIMIT_TOP_K_MIN = 1;
export const LIMIT_TOP_K_MAX = 50;
export const LIMIT_CUT_MIN = 0;
export const LIMIT_CUT_MAX = 1;
export const LIMIT_TEMPERATURE_MIN = 0;
export const LIMIT_TEMPERATURE_MAX = 2;
export const LIMIT_CONTEXT_MIN = 1000;
export const LIMIT_ROUNDS_MAX = 20;
export const LIMIT_TOOLS_MAX = 100;

export type TextModel = {
    provider: string;
    model: string;
    temperature: string;
    max_output_tokens: string;
    max_context_tokens: string;
    grouping_window_ms: string;
    max_tool_rounds: string;
    max_tool_calls: string;
    rag_top_k: string;
    rag_min_score: string;
};

export type ModelField = keyof TextModel;

export type ModelChange = Partial<{
    provider: string;
    model: string;
    temperature: number;
    max_output_tokens: number;
    max_context_tokens: number;
    grouping_window_ms: number;
    max_tool_rounds: number;
    max_tool_calls: number;
    rag_top_k: number;
    rag_min_score: number;
}>;

export type JevValues = {
    "jevOff": boolean;
    gates: Record<IdGate, GateState>;
};

function numberOrEmpty(value: number | null | undefined): string {
    return typeof value === "number" && Number.isFinite(value) ? String(value) : "";
}

export function agentModel(agente: Agente): TextModel {
    return {
        provider: agente.provider ?? "",
        model: agente.model ?? "",
        temperature: numberOrEmpty(agente.temperature),
        max_output_tokens: numberOrEmpty(agente.max_output_tokens),
        max_context_tokens: numberOrEmpty(agente.max_context_tokens),
        grouping_window_ms: numberOrEmpty(agente.grouping_window_ms),
        max_tool_rounds: numberOrEmpty(agente.max_tool_rounds),
        max_tool_calls: numberOrEmpty(agente.max_tool_calls_per_message),
        rag_top_k: numberOrEmpty(agente.rag_top_k),
        rag_min_score: numberOrEmpty(agente.rag_min_score),
    };
}

function gatesDoJev(jev: Record<string, unknown>): Record<IdGate, GateState> {
    const nested =
        jev["gates"] && typeof jev["gates"] === "object" && !Array.isArray(jev["gates"])
            ? (jev["gates"] as Record<string, unknown>)
            : {};
    const saida = {} as Record<IdGate, GateState>;
    for (const gate of GATES) {
        const value =
            typeof nested[gate] === "boolean"
                ? nested[gate]
                : typeof jev[gate] === "boolean"
                  ? jev[gate]
                  : undefined;
        saida[gate] = value === undefined ? "herdar" : value ? "ligado" : "desligado";
    }
    return saida;
}

export function agentJev(jev: Record<string, unknown> | undefined): JevValues {
    const current = jev ?? {};
    return {
        "jevOff": current["ativo"] === false,
        gates: gatesDoJev(current),
    };
}

export function gateEffect(values: JevValues, gate: IdGate): boolean {
    if (values["jevOff"]) return false;
    const estado = values.gates[gate];
    if (estado === "ligado") return true;
    if (estado === "desligado") return false;
    return FACTORY_DEFAULTS[gate];
}

function isObject(value: unknown): value is Record<string, unknown> {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function jevChange(
    currentJev: Record<string, unknown> | undefined,
    base: JevValues,
    draft: JevValues,
): string | undefined {
    const gatesChanged = GATES.filter((gate) => base.gates[gate] !== draft.gates[gate]);
    const switchChanged = base["jevOff"] !== draft["jevOff"];
    if (gatesChanged.length === 0 && !switchChanged) return undefined;

    const updated: Record<string, unknown> = { ...(currentJev ?? {}) };

    if (switchChanged) {
        if (draft["jevOff"]) updated["ativo"] = false;
        else delete updated["ativo"];
    }

    if (gatesChanged.length > 0) {
        const currentGates = updated["gates"];
        const nested: Record<string, unknown> = isObject(currentGates) ? { ...currentGates } : {};
        for (const gate of gatesChanged) {
            delete updated[gate];
            const estado = draft.gates[gate];
            if (estado === "herdar") delete nested[gate];
            else nested[gate] = estado === "ligado";
        }
        if (Object.keys(nested).length > 0) updated["gates"] = nested;
        else if (isObject(currentGates)) delete updated["gates"];
    }

    return JSON.stringify(updated);
}

function typedNumber(texto: string): number {
    const clean = texto.trim().replace(",", ".");
    if (clean === "") return Number["NaN"];
    return Number(clean);
}

function typedInteger(texto: string): number {
    const clean = texto.trim();
    if (!/^\d+$/.test(clean)) return Number["NaN"];
    return Number(clean);
}

export function validateModel(
    texto: TextModel,
    base: TextModel,
): Partial<Record<ModelField, string>> {
    const errors: Partial<Record<ModelField, string>> = {};

    if (texto.provider.trim() === "") errors.provider = translate("modelJev.validationProviderRequired");
    if (texto.model.trim() === "") errors.model = translate("modelJev.validationModelRequired");

    const deletedForGood = (campo: ModelField) =>
        texto[campo].trim() === "" && base[campo].trim() !== "";

    const temperatura = texto.temperature.trim();
    if (deletedForGood("temperature")) {
        errors.temperature = translate("modelJev.validationTemperatureLocked");
    } else if (temperatura !== "") {
        const n = typedNumber(temperatura);
        if (!Number.isFinite(n) || n < LIMIT_TEMPERATURE_MIN || n > LIMIT_TEMPERATURE_MAX) {
            errors.temperature = translate("modelJev.validationTemperatureRange");
        }
    }

    const saida = texto.max_output_tokens.trim();
    if (deletedForGood("max_output_tokens")) {
        errors.max_output_tokens = translate("modelJev.validationMaxOutputLocked");
    } else if (saida !== "" && !(typedInteger(saida) >= 1)) {
        errors.max_output_tokens = translate("modelJev.validationMaxOutputPositive");
    }

    const integers: Array<[ModelField, string, (n: number) => boolean]> = [
        ["max_context_tokens", translate("modelJev.validationMaxContextMin", { min: LIMIT_CONTEXT_MIN }), (n) => n >= LIMIT_CONTEXT_MIN],
        ["grouping_window_ms", translate("modelJev.validationGroupingWindowRange", { max: LIMIT_WINDOW_MS }), (n) => n >= 0 && n <= LIMIT_WINDOW_MS],
        ["max_tool_rounds", translate("modelJev.validationMaxToolRoundsRange", { max: LIMIT_ROUNDS_MAX }), (n) => n >= 0 && n <= LIMIT_ROUNDS_MAX],
        ["max_tool_calls", translate("modelJev.validationMaxToolCallsRange", { max: LIMIT_TOOLS_MAX }), (n) => n >= 0 && n <= LIMIT_TOOLS_MAX],
        ["rag_top_k", translate("modelJev.validationRagTopKRange", { min: LIMIT_TOP_K_MIN, max: LIMIT_TOP_K_MAX }), (n) => n >= LIMIT_TOP_K_MIN && n <= LIMIT_TOP_K_MAX],
    ];
    for (const [campo, mensagem, valid] of integers) {
        if (!valid(typedInteger(texto[campo]))) errors[campo] = mensagem;
    }

    const cut = texto.rag_min_score.trim();
    if (cut === "") {
        errors.rag_min_score = translate("modelJev.validationRagMinScoreRange", { min: LIMIT_CUT_MIN, max: LIMIT_CUT_MAX });
    } else {
        const n = typedNumber(cut);
        if (!Number.isFinite(n) || n < LIMIT_CUT_MIN || n > LIMIT_CUT_MAX) {
            errors.rag_min_score = translate("modelJev.validationRagMinScoreRange", { min: LIMIT_CUT_MIN, max: LIMIT_CUT_MAX });
        }
    }

    return errors;
}

export function modelChanges(base: TextModel, texto: TextModel): ModelChange {
    const change: ModelChange = {};
    if (texto.provider.trim() !== base.provider.trim()) change.provider = texto.provider.trim();
    if (texto.model.trim() !== base.model.trim()) change.model = texto.model.trim();

    const decimais: Array<"temperature" | "rag_min_score"> = ["temperature", "rag_min_score"];
    for (const campo of decimais) {
        const typed = texto[campo].trim();
        if (typed === "") continue;
        const n = typedNumber(typed);
        const before = typedNumber(base[campo]);
        if (n !== before) change[campo] = n;
    }

    const integers: Array<"max_output_tokens" | "max_context_tokens" | "grouping_window_ms" | "max_tool_rounds" | "max_tool_calls" | "rag_top_k"> = [
        "max_output_tokens",
        "max_context_tokens",
        "grouping_window_ms",
        "max_tool_rounds",
        "max_tool_calls",
        "rag_top_k",
    ];
    for (const campo of integers) {
        const typed = texto[campo].trim();
        if (typed === "") continue;
        const n = typedInteger(typed);
        const before = typedInteger(base[campo]);
        if (n !== before) change[campo] = n;
    }

    return change;
}

export async function saveModelJev(
    org: string,
    agente: string,
    versao: number,
    change: ModelChange & { jev_json?: string },
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
