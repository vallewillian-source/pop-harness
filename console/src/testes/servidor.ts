/**
 * Servidor HTTP falso para os testes de componente.
 *
 * A tela fala com a API pelo `cliente.ts` (que usa `fetch`), então o teste troca `fetch` por este
 * roteador: as chamadas saem iguais às de produção (método, caminho, cabeçalhos, corpo JSON) e o
 * teste confere o que realmente foi enviado — inclusive o `If-Match` do `PATCH` e o `?force` do
 * `DELETE`.
 */

import { vi } from "vitest";

export type Chamada = {
    metodo: string;
    url: string;
    cabecalhos: Record<string, string>;
    corpo: unknown;
    /** Corpo como foi entregue ao `fetch` (string, `File` ou `Blob`), sem interpretar. */
    bruto?: unknown;
};

type Resposta = {
    status: number;
    corpo?: unknown;
    /** Corpo como texto puro (ex.: `/metrics`, que não é JSON). Tem prioridade sobre `corpo`. */
    texto?: string;
    /** Cabeçalhos da resposta (ex.: `Content-Disposition` de um download). */
    cabecalhos?: Record<string, string>;
};

type Rota = {
    metodo: string;
    trecho: string;
    responder: (chamada: Chamada) => Resposta;
};

export class ServidorFalso {
    readonly chamadas: Chamada[] = [];
    private rotas: Rota[] = [];

    /** Declara uma resposta para método + trecho de caminho. A primeira rota que casa responde. */
    responder(
        metodo: string,
        trecho: string,
        resposta: Resposta | ((chamada: Chamada) => Resposta),
    ): this {
        this.rotas.push({
            metodo: metodo.toUpperCase(),
            trecho,
            responder: typeof resposta === "function" ? resposta : () => resposta,
        });
        return this;
    }

    /** Instala o `fetch` falso no ambiente do teste. */
    instalar(): this {
        vi.stubGlobal(
            "fetch",
            vi.fn(async (entrada: RequestInfo | URL, init?: RequestInit) => {
                const url =
                    typeof entrada === "string"
                        ? entrada
                        : entrada instanceof URL
                          ? entrada.toString()
                          : entrada.url;
                const metodo = (init?.method ?? "GET").toUpperCase();
                const cabecalhos: Record<string, string> = {};
                new Headers(init?.headers).forEach((valor, chave) => {
                    cabecalhos[chave.toLowerCase()] = valor;
                });
                let corpo: unknown;
                if (typeof init?.body === "string") {
                    try {
                        corpo = JSON.parse(init.body);
                    } catch {
                        corpo = init.body;
                    }
                }
                const chamada: Chamada = { metodo, url, cabecalhos, corpo, bruto: init?.body };
                this.chamadas.push(chamada);

                const rota = this.rotas.find(
                    (candidata) =>
                        candidata.metodo === metodo && url.includes(candidata.trecho),
                );
                if (!rota) {
                    throw new Error(`sem rota falsa para ${metodo} ${url}`);
                }
                const { status, corpo: dados, texto, cabecalhos: cabecalhosResposta } = rota.responder(chamada);
                const conteudo =
                    texto !== undefined
                        ? texto
                        : dados === undefined || status === 204
                          ? ""
                          : JSON.stringify(dados);
                return {
                    status,
                    ok: status >= 200 && status < 300,
                    headers: new Headers(cabecalhosResposta),
                    text: async () => conteudo,
                    json: async () => JSON.parse(conteudo),
                    blob: async () => new Blob([conteudo]),
                } as unknown as Response;
            }),
        );
        return this;
    }

    /** Chamadas que casaram com método e trecho, na ordem. */
    chamadasPara(metodo: string, trecho: string): Chamada[] {
        return this.chamadas.filter(
            (chamada) => chamada.metodo === metodo.toUpperCase() && chamada.url.includes(trecho),
        );
    }
}

/** Resposta de erro no formato do contrato (`{ code, message, details }`). */
export function erroApi(
    status: number,
    code: string,
    message: string,
    details: unknown = null,
): Resposta {
    return { status, corpo: { code, message, details } };
}
