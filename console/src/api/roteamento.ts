/**
 * Roteamento do agente (HR-43): departamento padrão, transferência humana e motivos.
 *
 * Onde cada coisa mora, pelo código que roda (não pela spec):
 * - departamento padrão → `department` do agente (`default_department_id`);
 * - transferência humana ligada/desligada → `handoff`; departamento humano → `handoff_department`;
 * - **motivos de transferência** → `situacoes_de_handoff` dentro do `jev_json` do agente. É ele que
 *   o motor lê (`poph_jev::situacoes_do_agente`). O campo `scope.handoff.reasons` das instruções
 *   existe no contrato, mas o motor não o lê: a tela não grava ali (ver docs/CONSOLE.md).
 *
 * Gravação com `If-Match` da versão do **agente** (a mesma das outras abas do agente). Como
 * `jev_json` substitui o JSON inteiro, a tela devolve as outras chaves (despedida, gates…) como
 * vieram na leitura: só `situacoes_de_handoff` muda.
 *
 * Limites da API, medidos (não impostos pela tela): não há como voltar a departamento padrão ou
 * humano para "nenhum" depois de definido (`null` é tratado como "não mexer"), e a API aceita mais
 * de 15 motivos e textos longos; por isso a tela aplica os limites da documentação do Jev.
 */

import { pedir } from "./cliente";
import type { Agente } from "./agentes";
import { translate } from "../i18n";

/** Motivos de transferência: até 15 (docs/JEV.md, seção de flags do agente). */
export const LIMITE_MOTIVOS = 15;

/** Cada motivo: até 300 caracteres. */
export const LIMITE_MOTIVO = 300;

/** O que a tela mostra e grava na aba Roteamento. */
export type Roteamento = {
    departamentoPadrao: string | null;
    transferenciaHumana: boolean;
    departamentoHumano: string | null;
    motivos: string[];
};

/** Alteração enviada no `PATCH` do agente: só o que mudou. */
export type MudancaRoteamento = {
    department?: string;
    handoff?: boolean;
    handoff_department?: string;
    jev_json?: string;
};

/** Lê o roteamento do agente como a tela o mostra. */
export function roteamentoDoAgente(agente: Agente): Roteamento {
    const bruto = agente.jev?.["situacoes_de_handoff"];
    const motivos = Array.isArray(bruto)
        ? bruto.filter((motivo): motivo is string => typeof motivo === "string")
        : [];
    return {
        departamentoPadrao: agente.default_department_id,
        transferenciaHumana: agente.human_handoff_enabled ?? false,
        departamentoHumano: agente.human_handoff_department_id,
        motivos,
    };
}

/** Limpa a lista antes de gravar: tira espaços e motivos vazios. */
export function motivosLimpos(motivos: string[]): string[] {
    return motivos.map((motivo) => motivo.trim()).filter((motivo) => motivo.length > 0);
}

/**
 * Valida o roteamento antes de chamar a API. Devolve mensagens por campo; vazio = pode gravar.
 * Chave `motivo-N` aponta a linha da lista (N começa em 0).
 */
export function validarRoteamento(valor: Roteamento): Record<string, string> {
    const erros: Record<string, string> = {};
    if (valor.transferenciaHumana && !valor.departamentoHumano) {
        erros["departamentoHumano"] = translate("routing.validationHumanDepartment");
    }
    const limpos = motivosLimpos(valor.motivos);
    if (limpos.length > LIMITE_MOTIVOS) {
        erros["motivos"] = translate("routing.validationTooManyReasons", {
            limit: LIMITE_MOTIVOS,
            count: limpos.length,
        });
    }
    valor.motivos.forEach((motivo, indice) => {
        if (motivo.trim().length > LIMITE_MOTIVO) {
            erros[`motivo-${indice}`] = translate("routing.validationReasonTooLong", {
                limit: LIMITE_MOTIVO,
            });
        }
    });
    return erros;
}

/**
 * Monta o `jev_json` com a lista nova de motivos e as outras chaves que já estavam no agente.
 * Chaves fora de `situacoes_de_handoff` voltam exatamente como foram lidas.
 */
export function jevComMotivos(jev: Record<string, unknown> | undefined, motivos: string[]): string {
    return JSON.stringify({ ...(jev ?? {}), situacoes_de_handoff: motivos });
}

/**
 * Grava o roteamento com o `If-Match` da versão lida do agente.
 *
 * `412` significa que outra pessoa salvou o agente antes; quem chama relê e decide o que mostrar,
 * sem perder o que foi digitado.
 */
export async function salvarRoteamento(
    org: string,
    agente: string,
    versao: number,
    mudanca: MudancaRoteamento,
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
