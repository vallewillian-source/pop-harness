/**
 * Modelo, RAG e Jev do agente (HR-45): leitura, validação e gravação da aba "Modelo e Jev".
 *
 * Nada aqui cria rota nova: tudo sai do `PATCH /admin/v1/orgs/{org}/agents/{agente}` que já existe,
 * com `If-Match` da versão do agente.
 *
 * Três regras que valem para a tela inteira:
 * - **Só o que mudou vai no corpo.** Campo que o operador não tocou não é enviado, então uma
 *   mudança feita por outra pessoa em outro campo não é desfeita por engano.
 * - **`jev_json` é substituído inteiro pela API.** Por isso a tela parte do `jev_json` **atual**
 *   (relido depois de um `412`) e mexe só nas chaves dos gates e do interruptor `ativo`. As outras
 *   chaves (`situacoes_de_handoff`, `despedida`, `resposta_segura`, `g7_acao`, `g7_resposta`, e
 *   qualquer chave desconhecida) voltam exatamente como foram lidas.
 * - **Gate e `ativo` seguem a leitura do motor** (`crates/poph-jev/src/flags.rs`): cada gate pode
 *   estar na raiz do `jev_json` ou dentro de `gates`; quando os dois existem, vale `gates`. A tela
 *   grava dentro de `gates`, que é o formato documentado em `docs/JEV.md`.
 */

import { pedir } from "./cliente";
import type { Agente } from "./agentes";
import { translate } from "../i18n";

/** Os oito portões do Jev, na ordem da tela. */
export const GATES = ["g1", "g2", "g3", "g4", "g5", "g6", "g7", "g8"] as const;
export type IdGate = (typeof GATES)[number];

/** Estado de um gate na tela: herda a configuração do servidor, ou é ligado/desligado no agente. */
export type EstadoGate = "herdar" | "ligado" | "desligado";

/**
 * Padrão de **fábrica** do código (`GatesJev::padrao()` em `crates/poph-types/src/config.rs`):
 * G3, G5 e G8 ligados; os demais desligados. A configuração do servidor (`poph.toml`) pode mudar
 * isso, e esta tela não consegue ler essa configuração: o texto da aba diz isso.
 */
export const PADRAO_DE_FABRICA: Record<IdGate, boolean> = {
    g1: false,
    g2: false,
    g3: true,
    g4: false,
    g5: true,
    g6: false,
    g7: false,
    g8: true,
};

/** Uma linha por gate: o que ele decide, em linguagem comum (sem números de medição). */
export const DESCRICAO_GATE: Record<IdGate, { titulo: string; explicacao: string }> = {
    g1: {
        get titulo() {
            return translate("modelJev.gateG1Title");
        },
        get explicacao() {
            return translate("modelJev.gateG1Description");
        },
    },
    g2: {
        get titulo() {
            return translate("modelJev.gateG2Title");
        },
        get explicacao() {
            return translate("modelJev.gateG2Description");
        },
    },
    g3: {
        get titulo() {
            return translate("modelJev.gateG3Title");
        },
        get explicacao() {
            return translate("modelJev.gateG3Description");
        },
    },
    g4: {
        get titulo() {
            return translate("modelJev.gateG4Title");
        },
        get explicacao() {
            return translate("modelJev.gateG4Description");
        },
    },
    g5: {
        get titulo() {
            return translate("modelJev.gateG5Title");
        },
        get explicacao() {
            return translate("modelJev.gateG5Description");
        },
    },
    g6: {
        get titulo() {
            return translate("modelJev.gateG6Title");
        },
        get explicacao() {
            return translate("modelJev.gateG6Description");
        },
    },
    g7: {
        get titulo() {
            return translate("modelJev.gateG7Title");
        },
        get explicacao() {
            return translate("modelJev.gateG7Description");
        },
    },
    g8: {
        get titulo() {
            return translate("modelJev.gateG8Title");
        },
        get explicacao() {
            return translate("modelJev.gateG8Description");
        },
    },
};

/** Chaves do `jev_json` que o motor lê e que esta aba **não** edita: são mostradas e preservadas. */
export const CHAVES_PRESERVADAS: ReadonlyArray<{ chave: string; explicacao: string; ondeEdita?: string }> = [
    {
        chave: "situacoes_de_handoff",
        get explicacao() {
            return translate("modelJev.preservedHandoffDescription");
        },
        get ondeEdita() {
            return translate("modelJev.preservedHandoffWhere");
        },
    },
    {
        chave: "despedida",
        get explicacao() {
            return translate("modelJev.preservedFarewellDescription");
        },
    },
    {
        chave: "resposta_segura",
        get explicacao() {
            return translate("modelJev.preservedSafeReplyDescription");
        },
    },
    {
        chave: "g7_acao",
        get explicacao() {
            return translate("modelJev.preservedG7ActionDescription");
        },
    },
    {
        chave: "g7_resposta",
        get explicacao() {
            return translate("modelJev.preservedG7ReplyDescription");
        },
    },
];

/** Limites da tela (a API não aplica estes; ver docs/CONSOLE.md). */
export const LIMITE_JANELA_MS = 8000;
export const LIMITE_TOP_K_MIN = 1;
export const LIMITE_TOP_K_MAX = 50;
export const LIMITE_CORTE_MIN = 0;
export const LIMITE_CORTE_MAX = 1;
export const LIMITE_TEMPERATURA_MIN = 0;
export const LIMITE_TEMPERATURA_MAX = 2;
/** Faixas que o banco impõe (restrições CHECK do esquema de `agents`). */
export const LIMITE_CONTEXTO_MIN = 1000;
export const LIMITE_RODADAS_MAX = 20;
export const LIMITE_FERRAMENTAS_MAX = 100;

/** Campos de modelo e RAG, como texto digitado. Chaves = nomes que a API usa no `PATCH`. */
export type ModeloTexto = {
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

export type CampoModelo = keyof ModeloTexto;

/** Corpo do `PATCH` de modelo e RAG: só os campos que mudaram, já convertidos. */
export type MudancaModelo = Partial<{
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

/** Interruptor e gates, como a tela os mostra. */
export type ValoresJev = {
    /** `true` = `ativo: false` no agente: nenhum gate roda aqui. */
    jevDesligado: boolean;
    gates: Record<IdGate, EstadoGate>;
};

function numeroOuVazio(valor: number | null | undefined): string {
    return typeof valor === "number" && Number.isFinite(valor) ? String(valor) : "";
}

/** Lê os campos de modelo e RAG do agente como texto para o formulário. */
export function modeloDoAgente(agente: Agente): ModeloTexto {
    return {
        provider: agente.provider ?? "",
        model: agente.model ?? "",
        temperature: numeroOuVazio(agente.temperature),
        max_output_tokens: numeroOuVazio(agente.max_output_tokens),
        max_context_tokens: numeroOuVazio(agente.max_context_tokens),
        grouping_window_ms: numeroOuVazio(agente.grouping_window_ms),
        max_tool_rounds: numeroOuVazio(agente.max_tool_rounds),
        max_tool_calls: numeroOuVazio(agente.max_tool_calls_per_message),
        rag_top_k: numeroOuVazio(agente.rag_top_k),
        rag_min_score: numeroOuVazio(agente.rag_min_score),
    };
}

/** Objeto de gate lido de `jev_json` (aninhado ou na raiz); só `true`/`false` contam. */
function gatesDoJev(jev: Record<string, unknown>): Record<IdGate, EstadoGate> {
    const aninhado =
        jev["gates"] && typeof jev["gates"] === "object" && !Array.isArray(jev["gates"])
            ? (jev["gates"] as Record<string, unknown>)
            : {};
    const saida = {} as Record<IdGate, EstadoGate>;
    for (const gate of GATES) {
        // Mesma precedência do motor: o valor aninhado vence; não-booleano é ignorado.
        const valor =
            typeof aninhado[gate] === "boolean"
                ? aninhado[gate]
                : typeof jev[gate] === "boolean"
                  ? jev[gate]
                  : undefined;
        saida[gate] = valor === undefined ? "herdar" : valor ? "ligado" : "desligado";
    }
    return saida;
}

/** Lê o interruptor e os gates do `jev_json` do agente. */
export function jevDoAgente(jev: Record<string, unknown> | undefined): ValoresJev {
    const atual = jev ?? {};
    return {
        jevDesligado: atual["ativo"] === false,
        gates: gatesDoJev(atual),
    };
}

/** Valores efetivos hoje: o que o gate faz neste agente, considerando o padrão de fábrica. */
export function efeitoDoGate(valores: ValoresJev, gate: IdGate): boolean {
    if (valores.jevDesligado) return false;
    const estado = valores.gates[gate];
    if (estado === "ligado") return true;
    if (estado === "desligado") return false;
    return PADRAO_DE_FABRICA[gate];
}

function ehObjeto(valor: unknown): valor is Record<string, unknown> {
    return typeof valor === "object" && valor !== null && !Array.isArray(valor);
}

/**
 * Monta o `jev_json` para gravar: parte do JSON **atual** do agente e aplica só as mudanças de gate
 * e de interruptor que o operador fez (`base` é o que ele viu, `rascunho` é o que está na tela).
 *
 * Devolve `undefined` quando nenhum gate nem o interruptor mudou: aí o `jev_json` nem vai no corpo.
 */
export function mudancaDoJev(
    jevAtual: Record<string, unknown> | undefined,
    base: ValoresJev,
    rascunho: ValoresJev,
): string | undefined {
    const gatesMudados = GATES.filter((gate) => base.gates[gate] !== rascunho.gates[gate]);
    const interruptorMudou = base.jevDesligado !== rascunho.jevDesligado;
    if (gatesMudados.length === 0 && !interruptorMudou) return undefined;

    const novo: Record<string, unknown> = { ...(jevAtual ?? {}) };

    if (interruptorMudou) {
        if (rascunho.jevDesligado) novo["ativo"] = false;
        else delete novo["ativo"];
    }

    if (gatesMudados.length > 0) {
        const gatesAtuais = novo["gates"];
        const aninhado: Record<string, unknown> = ehObjeto(gatesAtuais) ? { ...gatesAtuais } : {};
        for (const gate of gatesMudados) {
            // Tira a cópia da raiz também: sem isso, um valor antigo na raiz voltaria a valer.
            delete novo[gate];
            const estado = rascunho.gates[gate];
            if (estado === "herdar") delete aninhado[gate];
            else aninhado[gate] = estado === "ligado";
        }
        if (Object.keys(aninhado).length > 0) novo["gates"] = aninhado;
        else if (ehObjeto(gatesAtuais)) delete novo["gates"];
    }

    return JSON.stringify(novo);
}

/** Lê um número digitado: aceita vírgula decimal. `NaN` quando não é número. */
function numeroDigitado(texto: string): number {
    const limpo = texto.trim().replace(",", ".");
    if (limpo === "") return Number.NaN;
    return Number(limpo);
}

function inteiroDigitado(texto: string): number {
    const limpo = texto.trim();
    if (!/^\d+$/.test(limpo)) return Number.NaN;
    return Number(limpo);
}

/**
 * Valida o formulário antes de chamar a API. Devolve mensagem por campo; vazio = pode gravar.
 *
 * Campos vazios de temperatura e de máximo de saída significam "herda o padrão do servidor" quando
 * o agente ainda não tem valor. Se o agente já tem valor, a API não aceita voltar a `null`: a tela
 * avisa, em vez de fingir que apagou.
 */
export function validarModelo(
    texto: ModeloTexto,
    base: ModeloTexto,
): Partial<Record<CampoModelo, string>> {
    const erros: Partial<Record<CampoModelo, string>> = {};

    if (texto.provider.trim() === "") erros.provider = translate("modelJev.validationProviderRequired");
    if (texto.model.trim() === "") erros.model = translate("modelJev.validationModelRequired");

    const apagadoSemVolta = (campo: CampoModelo) =>
        texto[campo].trim() === "" && base[campo].trim() !== "";

    const temperatura = texto.temperature.trim();
    if (apagadoSemVolta("temperature")) {
        erros.temperature = translate("modelJev.validationTemperatureLocked");
    } else if (temperatura !== "") {
        const n = numeroDigitado(temperatura);
        if (!Number.isFinite(n) || n < LIMITE_TEMPERATURA_MIN || n > LIMITE_TEMPERATURA_MAX) {
            erros.temperature = translate("modelJev.validationTemperatureRange");
        }
    }

    const saida = texto.max_output_tokens.trim();
    if (apagadoSemVolta("max_output_tokens")) {
        erros.max_output_tokens = translate("modelJev.validationMaxOutputLocked");
    } else if (saida !== "" && !(inteiroDigitado(saida) >= 1)) {
        erros.max_output_tokens = translate("modelJev.validationMaxOutputPositive");
    }

    const inteiros: Array<[CampoModelo, string, (n: number) => boolean]> = [
        ["max_context_tokens", translate("modelJev.validationMaxContextMin", { min: LIMITE_CONTEXTO_MIN }), (n) => n >= LIMITE_CONTEXTO_MIN],
        ["grouping_window_ms", translate("modelJev.validationGroupingWindowRange", { max: LIMITE_JANELA_MS }), (n) => n >= 0 && n <= LIMITE_JANELA_MS],
        ["max_tool_rounds", translate("modelJev.validationMaxToolRoundsRange", { max: LIMITE_RODADAS_MAX }), (n) => n >= 0 && n <= LIMITE_RODADAS_MAX],
        ["max_tool_calls", translate("modelJev.validationMaxToolCallsRange", { max: LIMITE_FERRAMENTAS_MAX }), (n) => n >= 0 && n <= LIMITE_FERRAMENTAS_MAX],
        ["rag_top_k", translate("modelJev.validationRagTopKRange", { min: LIMITE_TOP_K_MIN, max: LIMITE_TOP_K_MAX }), (n) => n >= LIMITE_TOP_K_MIN && n <= LIMITE_TOP_K_MAX],
    ];
    for (const [campo, mensagem, valido] of inteiros) {
        if (!valido(inteiroDigitado(texto[campo]))) erros[campo] = mensagem;
    }

    const corte = texto.rag_min_score.trim();
    if (corte === "") {
        erros.rag_min_score = translate("modelJev.validationRagMinScoreRange", { min: LIMITE_CORTE_MIN, max: LIMITE_CORTE_MAX });
    } else {
        const n = numeroDigitado(corte);
        if (!Number.isFinite(n) || n < LIMITE_CORTE_MIN || n > LIMITE_CORTE_MAX) {
            erros.rag_min_score = translate("modelJev.validationRagMinScoreRange", { min: LIMITE_CORTE_MIN, max: LIMITE_CORTE_MAX });
        }
    }

    return erros;
}

/** Só os campos que mudaram em relação a `base`, já convertidos para o corpo do `PATCH`. */
export function mudancasDoModelo(base: ModeloTexto, texto: ModeloTexto): MudancaModelo {
    const mudanca: MudancaModelo = {};
    if (texto.provider.trim() !== base.provider.trim()) mudanca.provider = texto.provider.trim();
    if (texto.model.trim() !== base.model.trim()) mudanca.model = texto.model.trim();

    const decimais: Array<"temperature" | "rag_min_score"> = ["temperature", "rag_min_score"];
    for (const campo of decimais) {
        const digitado = texto[campo].trim();
        if (digitado === "") continue;
        const n = numeroDigitado(digitado);
        const antes = numeroDigitado(base[campo]);
        if (n !== antes) mudanca[campo] = n;
    }

    const inteiros: Array<"max_output_tokens" | "max_context_tokens" | "grouping_window_ms" | "max_tool_rounds" | "max_tool_calls" | "rag_top_k"> = [
        "max_output_tokens",
        "max_context_tokens",
        "grouping_window_ms",
        "max_tool_rounds",
        "max_tool_calls",
        "rag_top_k",
    ];
    for (const campo of inteiros) {
        const digitado = texto[campo].trim();
        if (digitado === "") continue;
        const n = inteiroDigitado(digitado);
        const antes = inteiroDigitado(base[campo]);
        if (n !== antes) mudanca[campo] = n;
    }

    return mudanca;
}

/** Grava a aba com `If-Match` da versão do agente. `412` é tratado por quem chama. */
export async function salvarModeloJev(
    org: string,
    agente: string,
    versao: number,
    mudanca: MudancaModelo & { jev_json?: string },
): Promise<Agente> {
    return pedir<Agente>(
        `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}`,
        {
            method: "PATCH",
            headers: { "if-match": String(versao) },
            body: JSON.stringify(mudanca),
        },
    );
}
