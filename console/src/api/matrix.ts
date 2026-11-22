/**
 * Módulo Matrix na organização (MX-22): estado, ligar/desligar e espaços por departamento.
 *
 * O contrato é o OpenAPI (`schema.d.ts`), gerado por `console/gen-api.sh`; os tipos daqui saem dele.
 * A concorrência do `PUT …/matrix` é o `If-Match` com o `updated_at` lido antes: o servidor manda
 * esse mesmo valor no cabeçalho `ETag` (entre aspas), e o corpo do `GET` traz o campo `updated_at`.
 * A tela usa o campo do corpo, que é o mesmo número, para não depender de ler cabeçalho.
 */

import { pedir } from "./cliente";
import type { components } from "./schema";

export type EstadoMatrix = components["schemas"]["EstadoMatrixJson"];
export type Espaco = components["schemas"]["EspacoJson"];
export type EspacoDesligado = components["schemas"]["EspacoDesligadoJson"];
export type EntradaEspaco = components["schemas"]["LigarEspaco"];

/** Caminho base do módulo de uma organização. */
function base(org: string): string {
    return `/admin/v1/orgs/${encodeURIComponent(org)}/matrix`;
}

/** Lê o estado do módulo na organização (ligado, atraso, falhas, conexão). */
export async function lerEstadoMatrix(org: string): Promise<EstadoMatrix> {
    return pedir<EstadoMatrix>(base(org));
}

/** Versão que vai no `If-Match` do próximo `PUT`: a lida no `GET` (`updated_at`). */
export function versaoDoEstado(estado: EstadoMatrix): number {
    return estado.updated_at;
}

/**
 * Liga ou desliga o módulo na org com o `If-Match` da versão lida antes.
 *
 * `412` quer dizer que o estado mudou depois dessa leitura; quem chama relê e avisa o operador.
 */
export async function alterarModuloMatrix(
    org: string,
    ligado: boolean,
    versao: number,
): Promise<EstadoMatrix> {
    const corpo: components["schemas"]["LigarDesligar"] = { enabled: ligado };
    return pedir<EstadoMatrix>(base(org), {
        method: "PUT",
        headers: { "if-match": String(versao) },
        body: JSON.stringify(corpo),
    });
}

/** Lista os espaços por departamento da org. */
export async function listarEspacosMatrix(org: string): Promise<Espaco[]> {
    const resposta = await pedir<components["schemas"]["EspacosJson"]>(`${base(org)}/spaces`);
    return resposta.items;
}

/**
 * Cria o espaço do departamento (sem `space`) ou liga um que já existe no Element (`space` com
 * `!id` ou `#alias`). Repetir não cria segundo espaço: a API devolve o que já existe.
 */
export async function ligarEspacoMatrix(org: string, entrada: EntradaEspaco): Promise<Espaco> {
    const corpo: EntradaEspaco = { department_id: entrada.department_id };
    if (entrada.space && entrada.space.trim()) {
        corpo.space = entrada.space.trim();
    }
    return pedir<Espaco>(`${base(org)}/spaces`, {
        method: "PUT",
        body: JSON.stringify(corpo),
    });
}

/**
 * Desliga o espaço do departamento. Não apaga nada no Element nem conversa: só o registro que o
 * módulo guarda deixa de ser usado (ver `docs/API.md`).
 */
export async function desligarEspacoMatrix(
    org: string,
    departamento: string,
): Promise<EspacoDesligado> {
    return pedir<EspacoDesligado>(`${base(org)}/spaces/${encodeURIComponent(departamento)}`, {
        method: "DELETE",
    });
}
