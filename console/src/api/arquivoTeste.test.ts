/**
 * Funções puras de ver e baixar o arquivo do cliente no chat de teste (HR-46c).
 */

import { describe, expect, it } from "vitest";
import {
    caminhoDoAnexo,
    caminhoDoArquivoDoAgente,
    lerComoDataUrl,
    nomeDoCabecalho,
    origemDoAnexoDoCliente,
    origemDoArquivoDoAgente,
    podeVerNaTela,
    rotuloDoEnvio,
} from "./arquivoTeste";

describe("nomeDoCabecalho", () => {
    it("prefere filename* em UTF-8 (acento e espaço)", () => {
        expect(
            nomeDoCabecalho("inline; filename=\"Laudo HR46b\"; filename*=UTF-8''Laudo%20HR46b"),
        ).toBe("Laudo HR46b");
        expect(nomeDoCabecalho("attachment; filename*=UTF-8''Procedimento%20de%20entrega%20(v2).pdf")).toBe(
            "Procedimento de entrega (v2).pdf",
        );
    });

    it("cai para filename entre aspas quando não há filename*", () => {
        expect(nomeDoCabecalho('attachment; filename="a b.pdf"')).toBe("a b.pdf");
    });

    it("aceita filename sem aspas e desfaz a barra no nome", () => {
        expect(nomeDoCabecalho("attachment; filename=laudo.png")).toBe("laudo.png");
        expect(nomeDoCabecalho('attachment; filename="x\\"y.pdf"')).toBe('x"y.pdf');
    });

    it("percentual malformado cai para o filename simples", () => {
        expect(nomeDoCabecalho("attachment; filename=\"seguro.pdf\"; filename*=UTF-8''%E0%A4%A")).toBe(
            "seguro.pdf",
        );
    });

    it("sem cabeçalho ou sem nome devolve null", () => {
        expect(nomeDoCabecalho(null)).toBeNull();
        expect(nomeDoCabecalho("inline")).toBeNull();
    });
});

describe("caminhoDoAnexo", () => {
    it("monta o caminho com org, agente, conversa, mensagem e anexo, escapados", () => {
        expect(caminhoDoAnexo("org 1", "ag/1", "conv_1", "m1", "anx_1")).toBe(
            "/admin/v1/orgs/org%201/agents/ag%2F1/test-chat/conv_1/messages/m1/attachments/anx_1/content",
        );
    });
});

describe("podeVerNaTela", () => {
    it("só imagem tem prévia; PDF e Office não", () => {
        expect(podeVerNaTela("image/png")).toBe(true);
        expect(podeVerNaTela("IMAGE/JPEG")).toBe(true);
        expect(podeVerNaTela("image/webp")).toBe(true);
        expect(podeVerNaTela("application/pdf")).toBe(false);
        expect(podeVerNaTela("application/vnd.openxmlformats-officedocument.wordprocessingml.document")).toBe(false);
        expect(podeVerNaTela("image/svg+xml")).toBe(false);
    });
});

describe("lerComoDataUrl", () => {
    it("converte os bytes em data URL com o tipo gravado", async () => {
        const bytes = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]);
        // `Response.blob()` é o mesmo tipo de Blob que o `fetch` devolve no navegador.
        const arquivo = await new Response(bytes).blob();
        const url = await lerComoDataUrl(arquivo, "image/png");
        expect(url).toBe("data:image/png;base64,iVBORw0KGgo=");
    });

    it("funciona com arquivo maior que um pedaço de conversão", async () => {
        const bytes = new Uint8Array(0x8000 * 2 + 5).fill(7);
        const arquivo = await new Response(bytes).blob();
        const url = await lerComoDataUrl(arquivo, "image/gif");
        const base64 = url.slice("data:image/gif;base64,".length);
        expect(atob(base64)).toHaveLength(bytes.length);
    });
});

describe("caminhoDoArquivoDoAgente (HR-46e)", () => {
    it("usa o arquivo_id da biblioteca, não o id do item da fila", () => {
        expect(caminhoDoArquivoDoAgente("org_1", "ag_1", "arq_9")).toBe(
            "/admin/v1/orgs/org_1/agents/ag_1/files/arq_9/content",
        );
    });
});

describe("origens de arquivo (HR-46e)", () => {
    it("o arquivo do agente sai da biblioteca, com a chave do item e a origem 'agente'", () => {
        const origem = origemDoArquivoDoAgente("org_1", "ag_1", {
            id: "env_1",
            arquivo_id: "arq_9",
            name: "Catalogo.pdf",
            mime: "application/pdf",
        });
        expect(origem).toEqual({
            chave: "envio:env_1",
            nome: "Catalogo.pdf",
            mime: "application/pdf",
            de: "agente",
            caminho: "/admin/v1/orgs/org_1/agents/ag_1/files/arq_9/content",
        });
    });

    it("o anexo do cliente sai do histórico da conversa, com a origem 'cliente'", () => {
        const origem = origemDoAnexoDoCliente("org_1", "ag_1", "conv_1", "m1", {
            id: "anx_1",
            name: "laudo.png",
            mime: "image/png",
        });
        expect(origem.de).toBe("cliente");
        expect(origem.chave).toBe("anexo:anx_1");
        expect(origem.caminho).toBe(
            "/admin/v1/orgs/org_1/agents/ag_1/test-chat/conv_1/messages/m1/attachments/anx_1/content",
        );
    });
});

describe("rotuloDoEnvio (HR-46e)", () => {
    it("traduz os quatro estados da fila e devolve os outros como vieram", () => {
        expect(rotuloDoEnvio("sent")).toBe("enviado");
        expect(rotuloDoEnvio("pending")).toBe("na fila para envio");
        expect(rotuloDoEnvio("sending")).toBe("na fila para envio");
        expect(rotuloDoEnvio("failed")).toBe("falhou ao enviar");
        expect(rotuloDoEnvio("outro")).toBe("outro");
    });
});
