/**
 * Instruções do agente (HR-40): leitura, gravação com `If-Match` e normalização do JSON.
 *
 * A API grava dois formatos: `structured` (o JSON do `StructuredInstructions`, do qual o servidor
 * **compõe** o markdown) e `markdown` (texto editado à mão, sem recomposição). O Console edita o
 * estruturado e mostra a prévia que o servidor devolve — o cliente nunca remonta o texto.
 *
 * Concorrência otimista (docs/API.md): `PATCH …/instructions/current` sem `If-Match` responde 428,
 * com versão velha responde 412 `version_conflict`; criar (`POST …/instructions`) só vale quando o
 * agente ainda não tem instruções (a versão esperada é 0).
 */

import { ErroApi, pedir } from "./cliente";

/** Regra inegociável com a ação de contingência. */
export type Principio = {
    rule: string;
    /** `say_dont_know` | `transfer` | `escalate` (só texto no markdown). */
    fallback_action: string;
};

/** Passo de fluxo: trafega e é preservado, mas não tem efeito no motor. */
export type PassoFluxo = {
    name: string;
    description: string;
    tools_used: string[];
    auto_from_template: boolean;
};

/** Ferramenta habilitada para o agente (lista fechada da V1). */
export type FerramentaConfig = {
    name: string;
    enabled: boolean;
    when_to_use: string;
};

/**
 * Instruções estruturadas — os mesmos nomes de campo do `StructuredInstructions` do servidor.
 *
 * `scope.handoff` existe no contrato e é preservado, mas o compositor não o emite: o departamento
 * padrão é vínculo do agente e os motivos são lidos pelo Jev.
 */
export type Estruturadas = {
    identity: {
        name: string;
        role: string;
        company: string;
        company_description: string;
    };
    principles: Principio[];
    scope: {
        does: string;
        does_not: string;
        handoff: { department_id: string; reasons: string[] };
    };
    flow_steps: PassoFluxo[];
    tone: {
        language: string;
        style: string;
        emoji_policy: string;
        format: string;
        extra_restrictions: string;
    };
    tools: FerramentaConfig[];
    extra_instructions: string;
};

/** Instruções como `GET …/instructions/current` devolve. */
export type Instrucoes = {
    agent_id: string;
    org_id: string;
    structured: Estruturadas;
    markdown: string;
    /** `true` quando o markdown foi composto do estruturado (o caso do Console). */
    derived_from_json: boolean;
    version: number;
    created_at: number;
    updated_at: number;
};

/** Ações de contingência que o formulário oferece (as mesmas que o compositor entende). */
export const ACOES_DE_CONTINGENCIA = [
    { valor: "say_dont_know", rotulo: "Dizer que não sabe" },
    { valor: "transfer", rotulo: "Transferir para humano" },
    { valor: "escalate", rotulo: "Escalar para o time" },
] as const;

/** Idiomas, estilos, emojis e formatos que o compositor traduz. */
export const IDIOMAS = [
    { valor: "pt-BR", rotulo: "Português (Brasil)" },
    { valor: "en", rotulo: "Inglês" },
    { valor: "es", rotulo: "Espanhol" },
] as const;

export const ESTILOS = [
    { valor: "professional", rotulo: "Profissional" },
    { valor: "casual", rotulo: "Casual" },
    { valor: "technical", rotulo: "Técnico" },
    { valor: "friendly", rotulo: "Acolhedor" },
] as const;

export const EMOJIS = [
    { valor: "moderate", rotulo: "Com moderação" },
    { valor: "none", rotulo: "Sem emojis" },
    { valor: "free", rotulo: "Livres" },
] as const;

export const FORMATOS = [
    { valor: "simple_lists", rotulo: "Listas simples" },
    { valor: "free_text", rotulo: "Texto corrido" },
    { valor: "tables", rotulo: "Tabelas" },
] as const;

/**
 * Ferramentas que o motor oferece na V1 (`FERRAMENTAS_V1` em `poph-types`).
 *
 * Nome fora desta lista é recusado pelo servidor na escrita (400 `validation_error`, campo
 * `structured`); a tela mostra o nome desconhecido como aviso, e nunca deixa marcar um novo.
 */
export const FERRAMENTAS: { nome: string; rotulo: string; descricao: string }[] = [
    {
        nome: "consultar_base",
        rotulo: "Consultar a base",
        descricao: "Busca o que está na base de conhecimento do agente.",
    },
    {
        nome: "buscar_produto",
        rotulo: "Buscar produto",
        descricao: "Consulta o catálogo; é a única fonte de preço e prazo.",
    },
    {
        nome: "enviar_arquivo",
        rotulo: "Enviar arquivo",
        descricao: "Envia um arquivo da biblioteca do agente.",
    },
    {
        nome: "transferir",
        rotulo: "Transferir",
        descricao: "Passa a conversa para o atendimento humano.",
    },
    {
        nome: "cart_add_item",
        rotulo: "Carrinho: adicionar",
        descricao: "Inclui um produto no orçamento em andamento.",
    },
    {
        nome: "cart_remove_item",
        rotulo: "Carrinho: remover",
        descricao: "Tira um produto do orçamento em andamento.",
    },
    {
        nome: "cart_summary",
        rotulo: "Carrinho: resumo",
        descricao: "Mostra o total do orçamento, calculado pelo servidor.",
    },
    {
        nome: "cart_clear",
        rotulo: "Carrinho: limpar",
        descricao: "Esvazia o orçamento em andamento.",
    },
];

/** Nomes aceitos pelo servidor nesta versão. */
export const NOMES_DE_FERRAMENTA = FERRAMENTAS.map((ferramenta) => ferramenta.nome);

function texto(valor: unknown, padrao = ""): string {
    return typeof valor === "string" ? valor : padrao;
}

function textoArray(valor: unknown): string[] {
    return Array.isArray(valor) ? valor.filter((item): item is string => typeof item === "string") : [];
}

function objeto(valor: unknown): Record<string, unknown> {
    return valor !== null && typeof valor === "object" && !Array.isArray(valor)
        ? (valor as Record<string, unknown>)
        : {};
}

/**
 * Instruções padrão de um agente que ainda não tem nenhuma.
 *
 * Base de conhecimento e catálogo já vêm ligados — é o mínimo de um atendente; o resto fica
 * desligado e o operador liga o que fizer sentido. O tom usa os mesmos padrões do contrato.
 * `flow_steps` e `scope.handoff` são preservados, por isso entram vazios.
 */
export function estruturadasPadrao(): Estruturadas {
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
        tools: FERRAMENTAS.map((ferramenta) => ({
            name: ferramenta.nome,
            enabled: ferramenta.nome === "consultar_base" || ferramenta.nome === "buscar_produto",
            when_to_use: "",
        })),
        extra_instructions: "",
    };
}

/**
 * Normaliza o JSON vindo do servidor (ou de um rascunho) para a forma do formulário.
 *
 * Objetos e listas ausentes viram vazios; chaves desconhecidas são descartadas porque o contrato
 * é fechado (`deny_unknown_fields` não vale aqui, mas o servidor recompõe do estruturado). A
 * ordem dos campos é fixa — é o que permite comparar dois estados por `JSON.stringify`.
 */
export function normalizarEstruturadas(valor: unknown): Estruturadas {
    const raiz = objeto(valor);
    const identidade = objeto(raiz.identity);
    const escopo = objeto(raiz.scope);
    const handoff = objeto(escopo.handoff);
    const tom = objeto(raiz.tone);
    const padrao = estruturadasPadrao();

    const ferramentas = Array.isArray(raiz.tools)
        ? raiz.tools.map((item) => {
              const ferramenta = objeto(item);
              return {
                  name: texto(ferramenta.name),
                  enabled: ferramenta.enabled !== false,
                  when_to_use: texto(ferramenta.when_to_use),
              };
          })
        : padrao.tools;

    return {
        identity: {
            name: texto(identidade.name),
            role: texto(identidade.role),
            company: texto(identidade.company),
            company_description: texto(identidade.company_description),
        },
        principles: Array.isArray(raiz.principles)
            ? raiz.principles.map((item) => {
                  const principio = objeto(item);
                  return {
                      rule: texto(principio.rule),
                      fallback_action: texto(principio.fallback_action, "say_dont_know"),
                  };
              })
            : [],
        scope: {
            does: texto(escopo.does),
            does_not: texto(escopo.does_not),
            handoff: {
                department_id: texto(handoff.department_id),
                reasons: textoArray(handoff.reasons),
            },
        },
        flow_steps: Array.isArray(raiz.flow_steps)
            ? raiz.flow_steps.map((item) => {
                  const passo = objeto(item);
                  return {
                      name: texto(passo.name),
                      description: texto(passo.description),
                      tools_used: textoArray(passo.tools_used),
                      auto_from_template: passo.auto_from_template === true,
                  };
              })
            : [],
        tone: {
            language: texto(tom.language, padrao.tone.language),
            style: texto(tom.style, padrao.tone.style),
            emoji_policy: texto(tom.emoji_policy, padrao.tone.emoji_policy),
            format: texto(tom.format, padrao.tone.format),
            extra_restrictions: texto(tom.extra_restrictions),
        },
        tools: ferramentas,
        extra_instructions: texto(raiz.extra_instructions),
    };
}

/** Dois estados têm o mesmo conteúdo? (ordem de campos fixa pela normalização). */
export function mesmasInstrucoes(a: Estruturadas, b: Estruturadas): boolean {
    return JSON.stringify(normalizarEstruturadas(a)) === JSON.stringify(normalizarEstruturadas(b));
}

function caminhoBase(org: string, agente: string): string {
    return `/admin/v1/orgs/${encodeURIComponent(org)}/agents/${encodeURIComponent(agente)}/instructions`;
}

/** Lê as instruções atuais; `null` quando o agente ainda não tem nenhuma (404). */
export async function lerInstrucoes(org: string, agente: string): Promise<Instrucoes | null> {
    try {
        const resposta = await pedir<Instrucoes>(`${caminhoBase(org, agente)}/current`);
        return { ...resposta, structured: normalizarEstruturadas(resposta.structured) };
    } catch (falha) {
        if (falha instanceof ErroApi && falha.status === 404) return null;
        throw falha;
    }
}

/**
 * Grava as instruções: cria quando `versao` é `null`, altera com `If-Match` quando há versão.
 *
 * O corpo manda só `structured` — o servidor compõe o markdown e o devolve na resposta. Texto
 * editado à mão (`markdown` sozinho) não é produzido por esta tela: a prévia é sempre a do
 * servidor.
 */
export async function salvarInstrucoes(
    org: string,
    agente: string,
    estruturadas: Estruturadas,
    versao: number | null,
): Promise<Instrucoes> {
    const corpo = JSON.stringify({ structured: estruturadas });
    const resposta =
        versao === null
            ? await pedir<Instrucoes>(caminhoBase(org, agente), { method: "POST", body: corpo })
            : await pedir<Instrucoes>(`${caminhoBase(org, agente)}/current`, {
                  method: "PATCH",
                  headers: { "if-match": String(versao) },
                  body: corpo,
              });
    return { ...resposta, structured: normalizarEstruturadas(resposta.structured) };
}
