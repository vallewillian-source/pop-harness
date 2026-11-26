/**
 * Testes da aba Arquivos do agente (HR-43).
 *
 * O que importa: listar a biblioteca, enviar com os metadados que a API espera (nome e descrição
 * na query, corpo binário), recusar acima de 16 MB **antes** de chamar a API, mostrar o motivo que
 * o servidor devolve quando recusa o tipo, remover com confirmação pelo nome e o estado vazio.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { GuardChangesProvider } from "../components/GuardChanges";
import { waitForModalFocus } from "../testing/modal";
import { FakeServer, apiError } from "../testing/server";
import { setLocale } from "../i18n";
import { AgentFiles } from "./AgenteArquivos";

const ORG = "org_1";
const AGENTE = "ag_1";
const LIMITE = 16 * 1024 * 1024;

const AGENTE_JSON = {
    id: AGENTE,
    org_id: ORG,
    name: "Beatriz",
    slug: "beatriz",
    description: "",
    status: "active",
    default_department_id: "dep_1",
    human_handoff_enabled: false,
    human_handoff_department_id: null,
    version: 1,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
};

type Arquivo = {
    id: string;
    org_id: string;
    agent_id: string;
    name: string;
    description: string;
    mime: string;
    size_bytes: number;
    sha256: string;
    created_at: number;
};

function arquivo(parcial: Partial<Arquivo> & { id: string; name: string }): Arquivo {
    return {
        org_id: ORG,
        agent_id: AGENTE,
        description: "",
        mime: "application/pdf",
        size_bytes: 2048,
        sha256: "a".repeat(64),
        created_at: 1_700_000_000_000,
        ...parcial,
    };
}

function montar() {
    return render(
        <MemoryRouter initialEntries={[`/orgs/${ORG}/agents/${AGENTE}/files`]}>
            <GuardChangesProvider>
                <Routes>
                    <Route path="/orgs/:orgId/agents/:agentId/files" element={<AgentFiles />} />
                    <Route path="/orgs/:orgId/agents" element={<div>Tela de agentes</div>} />
                </Routes>
            </GuardChangesProvider>
        </MemoryRouter>,
    );
}

/** Servidor falso com a biblioteca dada. Rotas de arquivo vêm antes da rota do agente. */
function servidorCom(biblioteca: Arquivo[], envio?: () => { status: number; corpo?: unknown }) {
    const estado = { biblioteca: [...biblioteca] };
    const servidor = new FakeServer()
        .responder("POST", `/agents/${AGENTE}/files`, (chamada) => {
            if (envio) return envio();
            const nome = new URL(chamada.url, "http://x").searchParams.get("name") ?? "";
            const criado = arquivo({
                id: "arq_novo",
                name: nome || "sem nome",
                mime: "image/png",
                size_bytes: 69,
            });
            estado.biblioteca = [...estado.biblioteca, criado];
            return { status: 201, corpo: criado };
        })
        .responder("DELETE", `/files/`, (chamada) => {
            const id = chamada.url.split("/files/")[1] ?? "";
            estado.biblioteca = estado.biblioteca.filter((a) => a.id !== id);
            return { status: 200, corpo: { deleted: true } };
        })
        .responder("GET", `/agents/${AGENTE}/files`, () => ({
            status: 200,
            corpo: { items: estado.biblioteca, next_cursor: null },
        }))
        .responder("GET", `/agents/${AGENTE}`, { status: 200, corpo: AGENTE_JSON })
        .responder("GET", `/admin/v1/orgs/${ORG}`, {
            status: 200,
            corpo: { id: ORG, name: "Clínica Teste", status: "active", version: 1 },
        });
    return servidor.instalar();
}

describe("Aba Arquivos", () => {
    it("lista a biblioteca do agente e pede até 200 itens", async () => {
        const servidor = servidorCom([
            arquivo({ id: "arq_1", name: "Tabela de preços", description: "Preços 2026", size_bytes: 3 * 1024 * 1024 }),
            arquivo({ id: "arq_2", name: "Folder", mime: "image/png", size_bytes: 4096 }),
        ]);
        montar();

        expect(await screen.findByText("Tabela de preços")).toBeInTheDocument();
        expect(screen.getByText("Folder")).toBeInTheDocument();
        expect(screen.getByText("Preços 2026")).toBeInTheDocument();
        expect(screen.getByText("3,0 MB")).toBeInTheDocument();
        expect(screen.getByText("4 KB")).toBeInTheDocument();
        expect(servidor.chamadasPara("GET", `/agents/${AGENTE}/files`)[0]?.url).toContain("limit=200");
    });

    it("mostra o estado vazio com o botão de enviar o primeiro arquivo", async () => {
        servidorCom([]);
        montar();

        expect(await screen.findByText("Nenhum arquivo na biblioteca")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Enviar primeiro arquivo" })).toBeEnabled();
    });

    it("envia o arquivo com nome e descrição na query e o corpo binário", async () => {
        const servidor = servidorCom([]);
        montar();
        await screen.findByText("Nenhum arquivo na biblioteca");
        const usuario = userEvent.setup();

        await usuario.click(screen.getByRole("button", { name: "Enviar arquivo" }));
        await waitForModalFocus();
        const escolhido = new File([new Uint8Array(69)], "pixel.png", { type: "image/png" });
        const entrada = document.querySelector('input[type="file"]') as HTMLInputElement;
        await usuario.upload(entrada, escolhido);
        await usuario.type(screen.getByLabelText(/Nome mostrado ao cliente/), "Mapa da clínica");
        await usuario.type(screen.getByLabelText(/Descrição para o agente/), "Use quando perguntarem onde fica");
        await usuario.click(within(document.querySelector('[role="dialog"]') as HTMLElement).getByRole("button", { name: "Enviar arquivo" }));

        await waitFor(() => expect(servidor.chamadasPara("POST", "/files").length).toBe(1));
        const envio = servidor.chamadasPara("POST", "/files")[0];
        const parametros = new URL(envio!.url, "http://x").searchParams;
        expect(parametros.get("filename")).toBe("pixel.png");
        expect(parametros.get("name")).toBe("Mapa da clínica");
        expect(parametros.get("description")).toBe("Use quando perguntarem onde fica");
        expect(envio!.cabecalhos["content-type"]).toBe("application/octet-stream");
        expect(envio!.bruto).toBe(escolhido);
        expect(await screen.findByText("Mapa da clínica")).toBeInTheDocument();
        expect(screen.getByRole("status")).toHaveTextContent('Arquivo "Mapa da clínica" enviado');
    });

    it("recusa arquivo acima de 16 MB antes de chamar a API e diz o tamanho e o limite", async () => {
        const servidor = servidorCom([]);
        montar();
        await screen.findByText("Nenhum arquivo na biblioteca");
        const usuario = userEvent.setup();

        await usuario.click(screen.getByRole("button", { name: "Enviar arquivo" }));
        await waitForModalFocus();
        const grande = new File([new Uint8Array(LIMITE + 1024 * 1024)], "catalogo.pdf", {
            type: "application/pdf",
        });
        const entrada = document.querySelector('input[type="file"]') as HTMLInputElement;
        await usuario.upload(entrada, grande);

        const dialogo = document.querySelector('[role="dialog"]') as HTMLElement;
        expect(await within(dialogo).findByText(/passa do limite de 16 MB por arquivo em 1,0 MB/)).toBeInTheDocument();
        await usuario.click(within(dialogo).getByRole("button", { name: "Enviar arquivo" }));
        expect(servidor.chamadasPara("POST", "/files")).toHaveLength(0);
    });

    it("mostra o motivo que o servidor devolve quando recusa o arquivo", async () => {
        servidorCom([], () =>
            apiError(400, "validation_error", "campo `arquivo` inválido", {
                field: "arquivo",
                reason:
                    "arquivo recusado: a extensão diz PDF, mas o conteúdo não é PDF. Envie o arquivo original.",
            }),
        );
        montar();
        await screen.findByText("Nenhum arquivo na biblioteca");
        const usuario = userEvent.setup();

        await usuario.click(screen.getByRole("button", { name: "Enviar arquivo" }));
        await waitForModalFocus();
        const falso = new File(["oi"], "documento.pdf", { type: "application/pdf" });
        const entrada = document.querySelector('input[type="file"]') as HTMLInputElement;
        await usuario.upload(entrada, falso);
        const dialogo = document.querySelector('[role="dialog"]') as HTMLElement;
        await usuario.click(within(dialogo).getByRole("button", { name: "Enviar arquivo" }));

        // O motivo do servidor vai para o campo do arquivo, como veio; o modal continua aberto.
        expect(await within(dialogo).findByText(/a extensão diz PDF, mas o conteúdo não é PDF/)).toBeInTheDocument();
        expect(dialogo).toBeInTheDocument();
    });

    it("remove com confirmação pelo nome e tira o item da lista", async () => {
        const servidor = servidorCom([arquivo({ id: "arq_1", name: "Tabela de preços" })]);
        montar();
        await screen.findByText("Tabela de preços");
        const usuario = userEvent.setup();

        await usuario.click(screen.getByRole("button", { name: "Excluir arquivo Tabela de preços" }));
        await waitForModalFocus();
        const dialogo = document.querySelector('[role="dialog"]') as HTMLElement;
        const confirmar = within(dialogo).getByRole("button", { name: "Excluir arquivo" });
        expect(confirmar).toBeDisabled();

        await usuario.type(within(dialogo).getByLabelText(/para confirmar/), "Tabela de preços");
        expect(confirmar).toBeEnabled();
        await usuario.click(confirmar);

        await waitFor(() => expect(servidor.chamadasPara("DELETE", "/files/arq_1")).toHaveLength(1));
        await waitFor(() => expect(screen.queryByText("Tabela de preços")).not.toBeInTheDocument());
    });

    it("usa o cabeçalho comum do agente e a biblioteca como seção plana, sem cartão", async () => {
        servidorCom([arquivo({ id: "arq_1", name: "Tabela de preços" })]);
        const { container } = montar();

        expect(await screen.findByRole("heading", { level: 1, name: "Beatriz" })).toBeInTheDocument();
        expect(await screen.findByText("Agente · Clínica Teste")).toBeInTheDocument();
        expect(screen.getByRole("region", { name: "Biblioteca de arquivos" })).toBeInTheDocument();
        expect(await screen.findByText("Tabela de preços")).toBeInTheDocument();
        expect(container.querySelector(".sofa-card")).toBeNull();
    });
});

describe("I18N-08: aba Arquivos em en-US", () => {
    afterEach(() => {
        act(() => setLocale("pt-BR"));
    });

    it("lista a biblioteca em inglês, com tamanho e contagem no formato do idioma", async () => {
        servidorCom([
            arquivo({
                id: "arq_1",
                name: "Price table",
                description: "Prices 2026",
                size_bytes: 3 * 1024 * 1024,
            }),
        ]);
        setLocale("en-US");
        montar();

        expect(await screen.findByText("Price table")).toBeInTheDocument();
        expect(screen.getByRole("columnheader", { name: "File" })).toBeInTheDocument();
        expect(screen.getByRole("columnheader", { name: "Type" })).toBeInTheDocument();
        expect(screen.getByRole("columnheader", { name: "Size" })).toBeInTheDocument();
        expect(screen.getByRole("columnheader", { name: "Actions" })).toBeInTheDocument();
        expect(screen.getByText("3.0 MB")).toBeInTheDocument();
        expect(screen.getByText("1 file")).toBeInTheDocument();
        expect(
            screen.getByRole("button", { name: "Delete file Price table" }),
        ).toBeInTheDocument();
        expect(screen.getByText("Agent · Clínica Teste")).toBeInTheDocument();
        expect(screen.getByRole("region", { name: "File library" })).toBeInTheDocument();
    });

    it("mostra o estado vazio em inglês com o botão de enviar o primeiro arquivo", async () => {
        servidorCom([]);
        setLocale("en-US");
        montar();

        expect(await screen.findByText("No file in the library")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Upload first file" })).toBeEnabled();
    });

    it("recusa o arquivo acima de 16 MB e envia com os metadados em inglês", async () => {
        const servidor = servidorCom([]);
        setLocale("en-US");
        montar();
        await screen.findByText("No file in the library");
        const usuario = userEvent.setup();

        await usuario.click(screen.getByRole("button", { name: "Upload file" }));
        await waitForModalFocus();
        const dialogo = document.querySelector('[role="dialog"]') as HTMLElement;
        expect(within(dialogo).getByText("Upload file to the agent")).toBeInTheDocument();

        const grande = new File([new Uint8Array(LIMITE + 1024 * 1024)], "catalog.pdf", {
            type: "application/pdf",
        });
        const entrada = document.querySelector('input[type="file"]') as HTMLInputElement;
        await usuario.upload(entrada, grande);
        expect(
            await within(dialogo).findByText(/over the 16 MB per-file limit by 1\.0 MB/),
        ).toBeInTheDocument();
        await usuario.click(within(dialogo).getByRole("button", { name: "Upload file" }));
        expect(servidor.chamadasPara("POST", "/files")).toHaveLength(0);

        const pequeno = new File([new Uint8Array(69)], "pixel.png", { type: "image/png" });
        await usuario.upload(entrada, pequeno);
        await usuario.type(
            within(dialogo).getByLabelText(/Name shown to the customer/),
            "Clinic map",
        );
        expect(
            within(dialogo).getByText(/Chosen: pixel\.png · 0\.0 MB \(within the 16 MB limit\)/),
        ).toBeInTheDocument();
        await usuario.click(within(dialogo).getByRole("button", { name: "Upload file" }));

        await waitFor(() => expect(servidor.chamadasPara("POST", "/files").length).toBe(1));
        expect(await screen.findByText("Clinic map")).toBeInTheDocument();
        expect(screen.getByRole("status")).toHaveTextContent('File "Clinic map" uploaded');
    });

    it("remove com confirmação pelo nome em inglês e avisa na lista", async () => {
        const servidor = servidorCom([arquivo({ id: "arq_1", name: "Price table" })]);
        setLocale("en-US");
        montar();
        await screen.findByText("Price table");
        const usuario = userEvent.setup();

        await usuario.click(screen.getByRole("button", { name: "Delete file Price table" }));
        await waitForModalFocus();
        const dialogo = document.querySelector('[role="dialog"]') as HTMLElement;
        expect(within(dialogo).getByText("Delete file?")).toBeInTheDocument();
        const confirmar = within(dialogo).getByRole("button", { name: "Delete file" });
        expect(confirmar).toBeDisabled();

        await usuario.type(within(dialogo).getByRole("textbox"), "Price table");
        await usuario.click(confirmar);

        await waitFor(() => expect(servidor.chamadasPara("DELETE", "/files/arq_1")).toHaveLength(1));
        await waitFor(() => expect(screen.queryByText("Price table")).not.toBeInTheDocument());
        expect(screen.getByRole("status")).toHaveTextContent('File "Price table" removed');
    });
});
