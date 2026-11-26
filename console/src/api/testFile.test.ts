
import { afterEach, describe, expect, it } from "vitest";
import { setLocale } from "../i18n";
import {
    attachmentPath,
    agentFilePath,
    readAsDataUrl,
    headerName,
    clientAttachmentOrigin,
    agentFileOrigin,
    canSeeOnScreen,
    sendLabel,
} from "./testFile";

describe("nomeDoCabecalho", () => {
    it("prefere filename* em UTF-8 (acento e espaço)", () => {
        expect(
            headerName("inline; filename=\"Laudo HR46b\"; filename*=UTF-8''Laudo%20HR46b"),
        ).toBe("Laudo HR46b");
        expect(headerName("attachment; filename*=UTF-8''Procedimento%20de%20entrega%20(v2).pdf")).toBe(
            "Procedimento de entrega (v2).pdf",
        );
    });

    it("cai para filename entre aspas quando não há filename*", () => {
        expect(headerName('attachment; filename="a b.pdf"')).toBe("a b.pdf");
    });

    it("aceita filename sem aspas e desfaz a barra no nome", () => {
        expect(headerName("attachment; filename=laudo.png")).toBe("laudo.png");
        expect(headerName('attachment; filename="x\\"y.pdf"')).toBe('x"y.pdf');
    });

    it("percentual malformado cai para o filename simples", () => {
        expect(headerName("attachment; filename=\"seguro.pdf\"; filename*=UTF-8''%E0%A4%A")).toBe(
            "seguro.pdf",
        );
    });

    it("sem cabeçalho ou sem nome devolve null", () => {
        expect(headerName(null)).toBeNull();
        expect(headerName("inline")).toBeNull();
    });
});

describe("caminhoDoAnexo", () => {
    it("monta o caminho com org, agente, conversa, mensagem e anexo, escapados", () => {
        expect(attachmentPath("org 1", "ag/1", "conv_1", "m1", "anx_1")).toBe(
            "/admin/v1/orgs/org%201/agents/ag%2F1/test-chat/conv_1/messages/m1/attachments/anx_1/content",
        );
    });
});

describe("podeVerNaTela", () => {
    it("só imagem tem prévia; PDF e Office não", () => {
        expect(canSeeOnScreen("image/png")).toBe(true);
        expect(canSeeOnScreen("IMAGE/JPEG")).toBe(true);
        expect(canSeeOnScreen("image/webp")).toBe(true);
        expect(canSeeOnScreen("application/pdf")).toBe(false);
        expect(canSeeOnScreen("application/vnd.openxmlformats-officedocument.wordprocessingml.document")).toBe(false);
        expect(canSeeOnScreen("image/svg+xml")).toBe(false);
    });
});

describe("lerComoDataUrl", () => {
    it("converte os bytes em data URL com o tipo gravado", async () => {
        const bytes = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]);
        const file = await new Response(bytes).blob();
        const url = await readAsDataUrl(file, "image/png");
        expect(url).toBe("data:image/png;base64,iVBORw0KGgo=");
    });

    it("funciona com arquivo maior que um pedaço de conversão", async () => {
        const bytes = new Uint8Array(0x8000 * 2 + 5).fill(7);
        const file = await new Response(bytes).blob();
        const url = await readAsDataUrl(file, "image/gif");
        const base64 = url.slice("data:image/gif;base64,".length);
        expect(atob(base64)).toHaveLength(bytes.length);
    });
});

describe("caminhoDoArquivoDoAgente (HR-46e)", () => {
    it("usa o arquivo_id da biblioteca, não o id do item da fila", () => {
        expect(agentFilePath("org_1", "ag_1", "arq_9")).toBe(
            "/admin/v1/orgs/org_1/agents/ag_1/files/arq_9/content",
        );
    });
});

describe("origens de arquivo (HR-46e)", () => {
    it("o arquivo do agente sai da biblioteca, com a chave do item e a origem 'agente'", () => {
        const origin = agentFileOrigin("org_1", "ag_1", {
            id: "env_1",
            "arquivo_id": "arq_9",
            name: "Catalogo.pdf",
            mime: "application/pdf",
        });
        expect(origin).toEqual({
            chave: "envio:env_1",
            nome: "Catalogo.pdf",
            mime: "application/pdf",
            de: "agente",
            "path": "/admin/v1/orgs/org_1/agents/ag_1/files/arq_9/content",
        });
    });

    it("o anexo do cliente sai do histórico da conversa, com a origem 'cliente'", () => {
        const origin = clientAttachmentOrigin("org_1", "ag_1", "conv_1", "m1", {
            id: "anx_1",
            name: "laudo.png",
            mime: "image/png",
        });
        expect(origin.de).toBe("cliente");
        expect(origin.chave).toBe("anexo:anx_1");
        expect(origin["path"]).toBe(
            "/admin/v1/orgs/org_1/agents/ag_1/test-chat/conv_1/messages/m1/attachments/anx_1/content",
        );
    });
});

describe("rotuloDoEnvio (HR-46e)", () => {
    it("traduz os quatro estados da fila e devolve os outros como vieram", () => {
        expect(sendLabel("sent")).toBe("enviado");
        expect(sendLabel("pending")).toBe("na fila para envio");
        expect(sendLabel("sending")).toBe("na fila para envio");
        expect(sendLabel("failed")).toBe("falhou ao enviar");
        expect(sendLabel("outro")).toBe("outro");
    });
});

describe("I18N-10: rotuloDoEnvio em en-US", () => {
    afterEach(() => {
        setLocale("pt-BR");
    });

    it("os quatro estados da fila em inglês; estado desconhecido sai como veio", () => {
        setLocale("en-US");
        expect(sendLabel("sent")).toBe("sent");
        expect(sendLabel("pending")).toBe("queued to send");
        expect(sendLabel("sending")).toBe("queued to send");
        expect(sendLabel("failed")).toBe("failed to send");
        expect(sendLabel("outro")).toBe("outro");
    });
});
