
import { afterEach, describe, expect, it, vi } from "vitest";
import { setLocale } from "../i18n";
import { sendTestMessage, parseFrame, readFlow, type TurnEvent } from "./testChat";

const codificador = new TextEncoder();

function chunkFlow(chunks: string[]): ReadableStream<Uint8Array> {
    return new ReadableStream<Uint8Array>({
        start(controller) {
            for (const chunk of chunks) {
                controller.enqueue(codificador.encode(chunk));
            }
            controller.close();
        },
    });
}

describe("interpretarQuadro", () => {
    it("lê o nome do evento e o JSON da linha data", () => {
        const evento = parseFrame('id: 3\nevent: texto\ndata: {"delta":"olá"}');
        expect(evento).toEqual({ tipo: "texto", id: "3", dados: { delta: "olá" } });
    });

    it("ignora o ping e os comentários, que não são do turno", () => {
        expect(parseFrame(": comentário do servidor")).toBeNull();
        expect(parseFrame("event: ping\ndata: {}")).toBeNull();
    });

    it("mantém o texto cru quando a linha data não é JSON", () => {
        const evento = parseFrame("event: erro\ndata: não é json");
        expect(evento?.dados).toBe("não é json");
    });
});

describe("lerFluxo", () => {
    it("entrega os eventos na ordem, mesmo quando um quadro chega partido", async () => {
        const received: TurnEvent[] = [];
        const body = chunkFlow([
            'id: 1\nevent: mensagem\ndata: {"text":"oi"}\n\nid: 2\nevent: inicio\n',
            'data: {"turn_id":"t1"}\n\nid: 3\nevent: texto\r\ndata: {"delta":"Re',
            'cebi"}\r\n\r\nevent: fim\ndata: {"state":"done"}\n\n',
        ]);

        await readFlow(body, (evento) => received.push(evento));

        expect(received.map((evento) => evento.tipo)).toEqual(["mensagem", "inicio", "texto", "fim"]);
        expect(received[2]?.dados).toEqual({ delta: "Recebi" });
        expect(received[3]?.dados).toEqual({ state: "done" });
    });

    it("entrega o último quadro mesmo quando o fluxo acaba sem a linha em branco", async () => {
        const received: TurnEvent[] = [];
        await readFlow(chunkFlow(["event: fim\ndata: {\"state\":\"failed\"}"]), (evento) =>
            received.push(evento),
        );
        expect(received).toHaveLength(1);
        expect(received[0]?.tipo).toBe("fim");
    });

    it("aborto para a leitura com AbortError e não entrega mais eventos", async () => {
        const controller = new AbortController();
        const received: TurnEvent[] = [];
        const body = new ReadableStream<Uint8Array>({
            start(c) {
                c.enqueue(codificador.encode('event: texto\ndata: {"delta":"a"}\n\n'));
                controller.abort();
                c.enqueue(codificador.encode('event: texto\ndata: {"delta":"b"}\n\n'));
            },
        });

        await expect(readFlow(body, (evento) => received.push(evento), controller.signal)).rejects.toMatchObject({
            name: "AbortError",
        });
        expect(received.length).toBeLessThanOrEqual(1);
    });
});

describe("I18N-10: erro do turno sem eventos no idioma ativo", () => {
    afterEach(() => {
        vi.unstubAllGlobals();
        setLocale("pt-BR");
    });

    it("a frase do erro segue o catálogo em pt-BR e en-US", async () => {
        vi.stubGlobal(
            "fetch",
            vi.fn(async () => new Response(null, { status: 200 })),
        );
        const send = (texto: string) =>
            sendTestMessage("org_1", "ag_1", "conv_1", { texto, files: [] }, { "onEvent": () => undefined });

        await expect(send("oi")).rejects.toThrow("a resposta do turno não trouxe eventos");
        setLocale("en-US");
        await expect(send("oi")).rejects.toThrow("the turn response brought no events");
    });
});
