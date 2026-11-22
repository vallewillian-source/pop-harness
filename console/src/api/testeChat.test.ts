/**
 * Leitor do fluxo do turno (HR-46). O que importa: cada evento sai na ordem do servidor, um quadro
 * partido entre dois pedaços de rede não se perde, o `ping` e os comentários não viram evento do
 * turno, e o aborto para a leitura com `AbortError` (o cancelamento).
 */

import { describe, expect, it } from "vitest";
import { interpretarQuadro, lerFluxo, type EventoDoTurno } from "./testeChat";

const codificador = new TextEncoder();

/** Fluxo que entrega os pedaços um a um, como a rede faria. */
function fluxoDePedacos(pedacos: string[]): ReadableStream<Uint8Array> {
    return new ReadableStream<Uint8Array>({
        start(controle) {
            for (const pedaco of pedacos) {
                controle.enqueue(codificador.encode(pedaco));
            }
            controle.close();
        },
    });
}

describe("interpretarQuadro", () => {
    it("lê o nome do evento e o JSON da linha data", () => {
        const evento = interpretarQuadro('id: 3\nevent: texto\ndata: {"delta":"olá"}');
        expect(evento).toEqual({ tipo: "texto", id: "3", dados: { delta: "olá" } });
    });

    it("ignora o ping e os comentários, que não são do turno", () => {
        expect(interpretarQuadro(": comentário do servidor")).toBeNull();
        expect(interpretarQuadro("event: ping\ndata: {}")).toBeNull();
    });

    it("mantém o texto cru quando a linha data não é JSON", () => {
        const evento = interpretarQuadro("event: erro\ndata: não é json");
        expect(evento?.dados).toBe("não é json");
    });
});

describe("lerFluxo", () => {
    it("entrega os eventos na ordem, mesmo quando um quadro chega partido", async () => {
        const recebidos: EventoDoTurno[] = [];
        const corpo = fluxoDePedacos([
            'id: 1\nevent: mensagem\ndata: {"text":"oi"}\n\nid: 2\nevent: inicio\n',
            'data: {"turn_id":"t1"}\n\nid: 3\nevent: texto\r\ndata: {"delta":"Re',
            'cebi"}\r\n\r\nevent: fim\ndata: {"state":"done"}\n\n',
        ]);

        await lerFluxo(corpo, (evento) => recebidos.push(evento));

        expect(recebidos.map((evento) => evento.tipo)).toEqual(["mensagem", "inicio", "texto", "fim"]);
        expect(recebidos[2]?.dados).toEqual({ delta: "Recebi" });
        expect(recebidos[3]?.dados).toEqual({ state: "done" });
    });

    it("entrega o último quadro mesmo quando o fluxo acaba sem a linha em branco", async () => {
        const recebidos: EventoDoTurno[] = [];
        await lerFluxo(fluxoDePedacos(["event: fim\ndata: {\"state\":\"failed\"}"]), (evento) =>
            recebidos.push(evento),
        );
        expect(recebidos).toHaveLength(1);
        expect(recebidos[0]?.tipo).toBe("fim");
    });

    it("aborto para a leitura com AbortError e não entrega mais eventos", async () => {
        const controle = new AbortController();
        const recebidos: EventoDoTurno[] = [];
        const corpo = new ReadableStream<Uint8Array>({
            start(c) {
                c.enqueue(codificador.encode('event: texto\ndata: {"delta":"a"}\n\n'));
                // Depois do primeiro pedaço o fluxo fica aberto; o aborto é o que encerra a leitura.
                controle.abort();
                c.enqueue(codificador.encode('event: texto\ndata: {"delta":"b"}\n\n'));
            },
        });

        await expect(lerFluxo(corpo, (evento) => recebidos.push(evento), controle.signal)).rejects.toMatchObject({
            name: "AbortError",
        });
        expect(recebidos.length).toBeLessThanOrEqual(1);
    });
});
