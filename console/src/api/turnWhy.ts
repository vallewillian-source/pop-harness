
import { translate } from "../i18n";
import { formatCurrency, formatNumber } from "../i18n/format";
import { GATE_DESCRIPTION, type IdGate } from "./modelJev";

export type GateDecision = {
    gate: string;
    "escolha": string | null;
    "jevChoice": string | null;
    "origem": string | null;
    "confianca": number | null;
    "margem": number | null;
    motivo: string | null;
    ms: number | null;
    tokens: number | null;
};

export type JevCall = {
    "perguntas": number | null;
    "decidedByCache": number | null;
    ms: number | null;
    tokens: number | null;
    "entrada": number | null;
    saida: number | null;
    total: number | null;
    "falha": string | null;
};

export type ReviewG6 = {
    "escolha": string | null;
    "confianca": number | null;
    regenerou: boolean;
    "secondChoice": string | null;
    "responseSafe": boolean;
};

export type TurnJev = {
    "chamada": JevCall | null;
    "decisoes": GateDecision[];
    "revisao": ReviewG6 | null;
};

export type TurnExcerpt = {
    item: string | null;
    "titulo": string;
    "grupo": string;
    texto: string;
    "similaridade": number | null;
    "relevancia": number | null;
};

export type TurnProduct = {
    "produto": string | null;
    nome: string;
    "precoTexto": string | null;
    "priceCents": number | null;
    "unidade": string;
    "condicoes": string;
};

export type TurnRetrieved = {
    "trechos": TurnExcerpt[];
    "produtos": TurnProduct[];
};

export type TurnTimings = {
    "queueMs": number | null;
    "buildMs": number | null;
    "firstTextMs": number | null;
    totalMs: number | null;
    "roundsTool": number | null;
    modelo: string | null;
    "reasonEnd": string | null;
};

type JsonObject = Record<string, unknown>;

function isObject(value: unknown): value is JsonObject {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}

function numero(value: unknown): number | null {
    return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function texto(value: unknown): string | null {
    return typeof value === "string" && value.trim() !== "" ? value : null;
}

function logico(value: unknown): boolean {
    return value === true;
}

function list(value: unknown): unknown[] {
    return Array.isArray(value) ? value : [];
}

export function readTurnJev(value: unknown): TurnJev | null {
    if (!isObject(value)) return null;

    const rawCall = isObject(value["chamada"]) ? value["chamada"] : null;
    const usage = rawCall && isObject(rawCall.usage) ? rawCall.usage : null;
    const call: JevCall | null = rawCall
        ? {
              "perguntas": numero(rawCall["perguntas"]),
              "decidedByCache": numero(rawCall["decididas_por_cache"]),
              ms: numero(rawCall.ms),
              tokens: numero(rawCall.tokens),
              "entrada": usage ? numero(usage["entrada"]) : null,
              saida: usage ? numero(usage.saida) : null,
              total: usage ? numero(usage.total) : null,
              "falha": texto(rawCall["falha"]),
          }
        : null;

    const decisions: GateDecision[] = list(value["decisoes"])
        .filter(isObject)
        .map((item) => ({
            gate: texto(item.gate) ?? translate("testChat.unknownGate"),
            "escolha": texto(item["escolha"]),
            "jevChoice": texto(item["escolha_do_jev"]),
            "origem": texto(item["origem"]),
            "confianca": numero(item["confianca"]),
            "margem": numero(item["margem"]),
            motivo: texto(item.motivo),
            ms: numero(item.ms),
            tokens: numero(item.tokens),
        }));

    const rawReview = isObject(value["revisao"]) ? value["revisao"] : null;
    const review: ReviewG6 | null = rawReview
        ? {
              "escolha": texto(rawReview["escolha"]),
              "confianca": numero(rawReview["confianca"]),
              regenerou: logico(rawReview.regenerou),
              "secondChoice": texto(rawReview["segunda_escolha"]),
              "responseSafe": logico(rawReview["resposta_segura"]),
          }
        : null;

    if (!call && decisions.length === 0 && !review) return null;
    return { "chamada": call, "decisoes": decisions, "revisao": review };
}

export function readTurnRetrieved(value: unknown): TurnRetrieved {
    const excerpts: TurnExcerpt[] = [];
    const products: TurnProduct[] = [];
    for (const item of list(value)) {
        if (!isObject(item)) continue;
        if (item.tipo === "produto") {
            products.push({
                "produto": texto(item["produto"]),
                nome: texto(item.nome) ?? translate("testChat.productWithoutName"),
                "precoTexto": texto(item["preco_texto"]),
                "priceCents": numero(item["preco_centavos"]),
                "unidade": texto(item["unidade"]) ?? "",
                "condicoes": texto(item["condicoes"]) ?? "",
            });
        } else if (item.tipo === "trecho" || texto(item["titulo"]) !== null) {
            excerpts.push({
                item: texto(item.item),
                "titulo": texto(item["titulo"]) ?? translate("testChat.snippetWithoutTitle"),
                "grupo": texto(item["grupo"]) ?? "",
                texto: texto(item.texto) ?? "",
                "similaridade": numero(item["similaridade"]),
                "relevancia": numero(item["relevancia"]),
            });
        }
    }
    return { "trechos": excerpts, "produtos": products };
}

export function readTurnTools(value: unknown): string[] {
    return list(value).filter((item): item is string => typeof item === "string" && item !== "");
}

export type ToolCall = {
    nome: string;
    "rodada": number | null;
    ordem: number | null;
    estado: "executada" | "recusada" | "nao_registrado";
    motivo: string | null;
    "durationMs": number | null;
    "argumentos": string | null;
    resultado: string | null;
};

export function readToolCalls(value: unknown): ToolCall[] {
    const calls: ToolCall[] = [];
    for (const item of list(value)) {
        if (!isObject(item)) continue;
        const nome = texto(item.nome);
        if (nome === null) continue;
        const estado =
            item.estado === "executada" || item.estado === "recusada" ? item.estado : "nao_registrado";
        calls.push({
            nome,
            "rodada": numero(item["rodada"]),
            ordem: numero(item.ordem),
            estado,
            motivo: estado === "recusada" ? texto(item.motivo) : null,
            "durationMs": numero(item["duracao_ms"]),
            "argumentos": texto(item["argumentos"]),
            resultado: texto(item.resultado),
        });
    }
    return calls.sort((a, b) => (a["rodada"] ?? 0) - (b["rodada"] ?? 0) || (a.ordem ?? 0) - (b.ordem ?? 0));
}

export function refusalReasonLabel(motivo: string | null): string {
    switch (motivo) {
        case "nao_habilitada":
            return translate("testChat.reasonNotEnabled");
        case "desconhecida":
            return translate("testChat.reasonUnknownTool");
        case "argumento_invalido":
            return translate("testChat.reasonInvalidArgument");
        case "erro_de_execucao":
            return translate("testChat.reasonExecutionError");
        case "limite_de_chamadas":
            return translate("testChat.reasonCallLimit");
        case "fora_do_turno":
            return translate("testChat.reasonOutOfTurn");
        case null:
            return translate("testChat.reasonNotRecorded");
        default:
            return motivo;
    }
}

export function formatToolDuration(ms: number | null): string {
    if (ms === null) return translate("testChat.durationNotRecorded");
    if (ms === 0) return translate("testChat.durationUnderOneMs");
    return formatMs(ms);
}

export function formatToolContent(texto: string): string {
    if (!/^\s*[[{]/.test(texto) || /\d{16,}/.test(texto)) return texto;
    try {
        JSON.parse(texto);
    } catch {
        return texto;
    }
    const items = tokensJson(texto);
    let saida = "";
    let nivel = 0;
    const newLine = () => `\n${"  ".repeat(nivel)}`;
    items.forEach((item, index) => {
        const previous = index > 0 ? items[index - 1] : undefined;
        if (item === "," || item === ":") {
            saida += item;
            return;
        }
        if (item === "}" || item === "]") {
            if (previous === "{" || previous === "[") {
                saida += item;
            } else {
                nivel -= 1;
                saida += `${newLine()}${item}`;
            }
            return;
        }
        if (previous === ":") saida += " ";
        else if (previous === ",") saida += newLine();
        else if (previous === "{" || previous === "[") {
            nivel += 1;
            saida += newLine();
        }
        saida += item;
    });
    return saida;
}

function tokensJson(texto: string): string[] {
    const items: string[] = [];
    let i = 0;
    while (i < texto.length) {
        const character = texto[i] as string;
        if (/\s/.test(character)) {
            i += 1;
        } else if (character === '"') {
            let fim = i + 1;
            let escapado = false;
            while (fim < texto.length) {
                const current = texto[fim];
                if (escapado) escapado = false;
                else if (current === "\\") escapado = true;
                else if (current === '"') break;
                fim += 1;
            }
            items.push(texto.slice(i, fim + 1));
            i = fim + 1;
        } else if ("{}[],:".includes(character)) {
            items.push(character);
            i += 1;
        } else {
            let fim = i;
            while (fim < texto.length && !/[\s"{}[\],:]/.test(texto[fim] as string)) fim += 1;
            items.push(texto.slice(i, fim));
            i = fim;
        }
    }
    return items;
}

export function countTools(names: string[]): Array<{ nome: string; "times": number }> {
    const count = new Map<string, number>();
    for (const nome of names) count.set(nome, (count.get(nome) ?? 0) + 1);
    return [...count.entries()].map(([nome, times]) => ({ nome, "times": times }));
}

export function readTurnTimings(value: unknown): TurnTimings | null {
    if (!isObject(value)) return null;
    return {
        "queueMs": numero(value["fila_ms"]),
        "buildMs": numero(value["montagem_ms"]),
        "firstTextMs": numero(value["primeiro_texto_ms"]),
        totalMs: numero(value.total_ms),
        "roundsTool": numero(value["rodadas_ferramenta"]),
        modelo: texto(value.modelo),
        "reasonEnd": texto(value.motivo_fim),
    };
}

export function gateTitle(gate: string): string {
    const chave = gate as IdGate;
    return GATE_DESCRIPTION[chave]?.["title"] ?? gate.toUpperCase();
}

export function originLabel(origin: string | null): string {
    switch (origin) {
        case "jev":
            return translate("testChat.originJev");
        case "cache":
            return translate("testChat.originCache");
        case "fallback":
            return translate("testChat.originFallback");
        default:
            return translate("testChat.notRecorded");
    }
}

export function endReasonLabel(motivo: string | null): string {
    switch (motivo) {
        case "concluido":
            return translate("testChat.endCompleted");
        case "chamadas_ferramenta":
            return translate("testChat.endToolCalls");
        case "limite_de_tokens":
            return translate("testChat.endTokenLimit");
        case null:
            return translate("testChat.notRecorded");
        default:
            return motivo;
    }
}

export function formatConfidence(value: number | null): string {
    if (value === null) return translate("testChat.notRecorded");
    return `${formatNumber(Math.round(value * 100))}%`;
}

export function formatMargin(value: number | null): string {
    if (value === null) return translate("testChat.notRecorded");
    return `${formatNumber(value * 100, { minimumFractionDigits: 1, maximumFractionDigits: 1 })}%`;
}

export function formatMs(value: number | null): string {
    if (value === null) return translate("testChat.notRecorded");
    return translate("testChat.milliseconds", { value: formatNumber(value) });
}

export function formatTokens(value: number | null): string {
    if (value === null) return translate("testChat.notRecorded");
    return translate("testChat.tokenCount", { value: formatNumber(value) });
}

export function formatCostMicros(value: number | null): string {
    if (value === null || value === 0) return translate("testChat.costNotCalculated");
    return translate("testChat.costMicros", { value: formatNumber(value) });
}

export function formatPriceCents(cents: number | null): string | null {
    if (cents === null) return null;
    return formatCurrency(cents / 100, "BRL");
}

export function turnDuration(
    start: number,
    fim: number | null,
    timings: TurnTimings | null,
): number | null {
    if (timings && timings.totalMs !== null) return timings.totalMs;
    if (fim === null) return null;
    return Math.max(0, fim - start);
}

export function turnStateLabel(estado: string): string {
    switch (estado) {
        case "done":
            return translate("testChat.stateDone");
        case "failed":
            return translate("testChat.stateFailed");
        case "skipped":
            return translate("testChat.stateSkipped");
        case "running":
            return translate("testChat.stateRunning");
        default:
            return estado;
    }
}

export function phraseForStateWithoutResponse(estado: string): string {
    switch (estado) {
        case "failed":
            return translate("testChat.noResponseFailed");
        case "skipped":
            return translate("testChat.noResponseSkipped");
        case "done":
            return translate("testChat.noResponseDone");
        case "running":
            return translate("testChat.noResponseRunning");
        default:
            return translate("testChat.noResponseDefault");
    }
}

export type RegistryItem<M, T> = { tipo: "mensagem"; mensagem: M } | { tipo: "turno"; turno: T };

export function interleaveTurns<M extends { created_at: number }, T extends { started_at: number }>(
    messages: M[],
    turns: T[],
): RegistryItem<M, T>[] {
    const ordenados = [...turns].sort((a, b) => a.started_at - b.started_at);
    const saida: RegistryItem<M, T>[] = [];
    let next = 0;
    for (const mensagem of messages) {
        let turno = ordenados[next];
        while (turno !== undefined && turno.started_at < mensagem.created_at) {
            saida.push({ tipo: "turno", turno });
            next += 1;
            turno = ordenados[next];
        }
        saida.push({ tipo: "mensagem", mensagem });
    }
    for (const turno of ordenados.slice(next)) {
        saida.push({ tipo: "turno", turno });
    }
    return saida;
}
