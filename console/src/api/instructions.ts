
import { ApiError, request } from "./client";
import type { TranslationKey } from "../i18n/catalog";

export type Principle = {
    rule: string;
    fallback_action: string;
};

export type FlowStep = {
    name: string;
    description: string;
    tools_used: string[];
    auto_from_template: boolean;
};

export type ToolConfig = {
    name: string;
    enabled: boolean;
    when_to_use: string;
};

export type Structured = {
    identity: {
        name: string;
        role: string;
        company: string;
        company_description: string;
    };
    principles: Principle[];
    scope: {
        does: string;
        does_not: string;
        handoff: { department_id: string; reasons: string[] };
    };
    flow_steps: FlowStep[];
    tone: {
        language: string;
        style: string;
        emoji_policy: string;
        format: string;
        extra_restrictions: string;
    };
    tools: ToolConfig[];
    extra_instructions: string;
};

export type Instructions = {
    agent_id: string;
    org_id: string;
    structured: Structured;
    markdown: string;
    derived_from_json: boolean;
    version: number;
    created_at: number;
    updated_at: number;
};

export const CONTINGENCY_ACTIONS: { "valor": string; labelKey: TranslationKey }[] = [
    { "valor": "say_dont_know", labelKey: "instructions.fallbackSayDontKnow" },
    { "valor": "transfer", labelKey: "instructions.fallbackTransfer" },
    { "valor": "escalate", labelKey: "instructions.fallbackEscalate" },
] as const;

export const IDIOMAS = [
    { "valor": "pt-BR", labelKey: "instructions.languagePortuguese" },
    { "valor": "en", labelKey: "instructions.languageEnglish" },
    { "valor": "es", labelKey: "instructions.languageSpanish" },
] as const;

export const ESTILOS = [
    { "valor": "professional", labelKey: "instructions.styleProfessional" },
    { "valor": "casual", labelKey: "instructions.styleCasual" },
    { "valor": "technical", labelKey: "instructions.styleTechnical" },
    { "valor": "friendly", labelKey: "instructions.styleFriendly" },
] as const;

export const EMOJIS = [
    { "valor": "moderate", labelKey: "instructions.emojiModerate" },
    { "valor": "none", labelKey: "instructions.emojiNone" },
    { "valor": "free", labelKey: "instructions.emojiFree" },
] as const;

export const FORMATS = [
    { "valor": "simple_lists", labelKey: "instructions.formatSimpleLists" },
    { "valor": "free_text", labelKey: "instructions.formatFreeText" },
    { "valor": "tables", labelKey: "instructions.formatTables" },
] as const;

export const TOOLS: { nome: string; labelKey: TranslationKey; descriptionKey: TranslationKey }[] = [
    {
        nome: "consultar_base",
        labelKey: "instructions.toolConsultKnowledgeBase",
        descriptionKey: "instructions.toolConsultKnowledgeBaseDescription",
    },
    {
        nome: "buscar_produto",
        labelKey: "instructions.toolSearchProduct",
        descriptionKey: "instructions.toolSearchProductDescription",
    },
    {
        nome: "enviar_arquivo",
        labelKey: "instructions.toolSendFile",
        descriptionKey: "instructions.toolSendFileDescription",
    },
    {
        nome: "transferir",
        labelKey: "instructions.toolTransfer",
        descriptionKey: "instructions.toolTransferDescription",
    },
    {
        nome: "cart_add_item",
        labelKey: "instructions.toolCartAdd",
        descriptionKey: "instructions.toolCartAddDescription",
    },
    {
        nome: "cart_remove_item",
        labelKey: "instructions.toolCartRemove",
        descriptionKey: "instructions.toolCartRemoveDescription",
    },
    {
        nome: "cart_summary",
        labelKey: "instructions.toolCartSummary",
        descriptionKey: "instructions.toolCartSummaryDescription",
    },
    {
        nome: "cart_clear",
        labelKey: "instructions.toolCartClear",
        descriptionKey: "instructions.toolCartClearDescription",
    },
];

export const TOOL_NAMES = TOOLS.map((tool) => tool.nome);

function texto(value: unknown, fallback = ""): string {
    return typeof value === "string" ? value : fallback;
}

function textArray(value: unknown): string[] {
    return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

function asRecord(value: unknown): Record<string, unknown> {
    return value !== null && typeof value === "object" && !Array.isArray(value)
        ? (value as Record<string, unknown>)
        : {};
}

export function defaultStructured(): Structured {
    return {
        identity: { name: "", role: "", company: "", company_description: "" },
        principles: [],
        scope: {
            does: "",
            does_not: "",
            handoff: { department_id: "", reasons: [] },
        },
        flow_steps: [],
        tone: {
            language: "pt-BR",
            style: "professional",
            emoji_policy: "moderate",
            format: "simple_lists",
            extra_restrictions: "",
        },
        tools: TOOLS.map((tool) => ({
            name: tool.nome,
            enabled: tool.nome === "consultar_base" || tool.nome === "buscar_produto",
            when_to_use: "",
        })),
        extra_instructions: "",
    };
}

export function normalizeStructured(value: unknown): Structured {
    const root = asRecord(value);
    const identity = asRecord(root.identity);
    const scope = asRecord(root.scope);
    const handoff = asRecord(scope.handoff);
    const tom = asRecord(root.tone);
    const fallback = defaultStructured();

    const tools = Array.isArray(root.tools)
        ? root.tools.map((entry) => {
              const tool = asRecord(entry);
              return {
                  name: texto(tool.name),
                  enabled: tool.enabled !== false,
                  when_to_use: texto(tool.when_to_use),
              };
          })
        : fallback.tools;

    return {
        identity: {
            name: texto(identity.name),
            role: texto(identity.role),
            company: texto(identity.company),
            company_description: texto(identity.company_description),
        },
        principles: Array.isArray(root.principles)
            ? root.principles.map((entry) => {
                  const principle = asRecord(entry);
                  return {
                      rule: texto(principle.rule),
                      fallback_action: texto(principle.fallback_action, "say_dont_know"),
                  };
              })
            : [],
        scope: {
            does: texto(scope.does),
            does_not: texto(scope.does_not),
            handoff: {
                department_id: texto(handoff.department_id),
                reasons: textArray(handoff.reasons),
            },
        },
        flow_steps: Array.isArray(root.flow_steps)
            ? root.flow_steps.map((entry) => {
                  const step = asRecord(entry);
                  return {
                      name: texto(step.name),
                      description: texto(step.description),
                      tools_used: textArray(step.tools_used),
                      auto_from_template: step.auto_from_template === true,
                  };
              })
            : [],
        tone: {
            language: texto(tom.language, fallback.tone.language),
            style: texto(tom.style, fallback.tone.style),
            emoji_policy: texto(tom.emoji_policy, fallback.tone.emoji_policy),
            format: texto(tom.format, fallback.tone.format),
            extra_restrictions: texto(tom.extra_restrictions),
        },
        tools: tools,
        extra_instructions: texto(root.extra_instructions),
    };
}

export function sameInstructions(a: Structured, b: Structured): boolean {
    return JSON.stringify(normalizeStructured(a)) === JSON.stringify(normalizeStructured(b));
}

function basePath(org: string, agente: string): string {
    return `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}/instructions`;
}

export async function readInstructions(org: string, agente: string): Promise<Instructions | null> {
    try {
        const response = await request<Instructions>(`${basePath(org, agente)}/current`);
        return { ...response, structured: normalizeStructured(response.structured) };
    } catch (failure) {
        if (failure instanceof ApiError && failure.status === 404) return null;
        throw failure;
    }
}

export async function saveInstructions(
    org: string,
    agente: string,
    structured: Structured,
    versao: number | null,
): Promise<Instructions> {
    const body = JSON.stringify({ structured: structured });
    const response =
        versao === null
            ? await request<Instructions>(basePath(org, agente), { method: "POST", body: body })
            : await request<Instructions>(`${basePath(org, agente)}/current`, {
                  method: "PATCH",
                  headers: { "if-match": String(versao) },
                  body: body,
              });
    return { ...response, structured: normalizeStructured(response.structured) };
}
