
import { afterEach, describe, expect, it } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { GuardChangesProvider } from "../components/GuardChanges";
import { waitForModalFocus } from "../testing/modal";
import { FakeServer, apiError } from "../testing/server";
import { setLocale } from "../i18n";
import { AgentFiles } from "./AgentFiles";

const ORG = "org_1";
const AGENTE = "ag_1";
const LIMIT = 16 * 1024 * 1024;

const AGENT_JSON = {
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

type StoredFile = {
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

function file(partial: Partial<StoredFile> & { id: string; name: string }): StoredFile {
    return {
        org_id: ORG,
        agent_id: AGENTE,
        description: "",
        mime: "application/pdf",
        size_bytes: 2048,
        sha256: "a".repeat(64),
        created_at: 1_700_000_000_000,
        ...partial,
    };
}

function build() {
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

function serverWith(library: StoredFile[], send?: () => { status: number; corpo?: unknown }) {
    const estado = { library: [...library] };
    const servidor = new FakeServer()
        .respond("POST", `/agents/${AGENTE}/files`, (call) => {
            if (send) return send();
            const nome = new URL(call.url, "http://x").searchParams.get("name") ?? "";
            const created = file({
                id: "arq_novo",
                name: nome || "sem nome",
                mime: "image/png",
                size_bytes: 69,
            });
            estado.library = [...estado.library, created];
            return { status: 201, corpo: created };
        })
        .respond("DELETE", `/files/`, (call) => {
            const id = call.url.split("/files/")[1] ?? "";
            estado.library = estado.library.filter((a) => a.id !== id);
            return { status: 200, corpo: { deleted: true } };
        })
        .respond("GET", `/agents/${AGENTE}/files`, () => ({
            status: 200,
            corpo: { items: estado.library, next_cursor: null },
        }))
        .respond("GET", `/agents/${AGENTE}`, { status: 200, corpo: AGENT_JSON })
        .respond("GET", `/admin/v1/orgs/${ORG}`, {
            status: 200,
            corpo: { id: ORG, name: "Clínica Teste", status: "active", version: 1 },
        });
    return servidor.install();
}

describe("Aba Arquivos", () => {
    it("lista a biblioteca do agente e pede até 200 itens", async () => {
        const servidor = serverWith([
            file({ id: "arq_1", name: "Tabela de preços", description: "Preços 2026", size_bytes: 3 * 1024 * 1024 }),
            file({ id: "arq_2", name: "Folder", mime: "image/png", size_bytes: 4096 }),
        ]);
        build();

        expect(await screen.findByText("Tabela de preços")).toBeInTheDocument();
        expect(screen.getByText("Folder")).toBeInTheDocument();
        expect(screen.getByText("Preços 2026")).toBeInTheDocument();
        expect(screen.getByText("3,0 MB")).toBeInTheDocument();
        expect(screen.getByText("4 KB")).toBeInTheDocument();
        expect(servidor.callsFor("GET", `/agents/${AGENTE}/files`)[0]?.url).toContain("limit=200");
    });

    it("mostra o estado vazio com o botão de enviar o primeiro arquivo", async () => {
        serverWith([]);
        build();

        expect(await screen.findByText("Nenhum arquivo na biblioteca")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Enviar primeiro arquivo" })).toBeEnabled();
    });

    it("envia o arquivo com nome e descrição na query e o corpo binário", async () => {
        const servidor = serverWith([]);
        build();
        await screen.findByText("Nenhum arquivo na biblioteca");
        const user = userEvent.setup();

        await user.click(screen.getByRole("button", { name: "Enviar arquivo" }));
        await waitForModalFocus();
        const chosen = new File([new Uint8Array(69)], "pixel.png", { type: "image/png" });
        const input = document.querySelector('input[type="file"]') as HTMLInputElement;
        await user.upload(input, chosen);
        await user.type(screen.getByLabelText(/Nome mostrado ao cliente/), "Mapa da clínica");
        await user.type(screen.getByLabelText(/Descrição para o agente/), "Use quando perguntarem onde fica");
        await user.click(within(document.querySelector('[role="dialog"]') as HTMLElement).getByRole("button", { name: "Enviar arquivo" }));

        await waitFor(() => expect(servidor.callsFor("POST", "/files").length).toBe(1));
        const send = servidor.callsFor("POST", "/files")[0];
        const parameters = new URL(send!.url, "http://x").searchParams;
        expect(parameters.get("filename")).toBe("pixel.png");
        expect(parameters.get("name")).toBe("Mapa da clínica");
        expect(parameters.get("description")).toBe("Use quando perguntarem onde fica");
        expect(send!.headers["content-type"]).toBe("application/octet-stream");
        expect(send!.raw).toBe(chosen);
        expect(await screen.findByText("Mapa da clínica")).toBeInTheDocument();
        expect(screen.getByRole("status")).toHaveTextContent('Arquivo "Mapa da clínica" enviado');
    });

    it("recusa arquivo acima de 16 MB antes de chamar a API e diz o tamanho e o limite", async () => {
        const servidor = serverWith([]);
        build();
        await screen.findByText("Nenhum arquivo na biblioteca");
        const user = userEvent.setup();

        await user.click(screen.getByRole("button", { name: "Enviar arquivo" }));
        await waitForModalFocus();
        const large = new File([new Uint8Array(LIMIT + 1024 * 1024)], "catalogo.pdf", {
            type: "application/pdf",
        });
        const input = document.querySelector('input[type="file"]') as HTMLInputElement;
        await user.upload(input, large);

        const dialog = document.querySelector('[role="dialog"]') as HTMLElement;
        expect(await within(dialog).findByText(/passa do limite de 16 MB por arquivo em 1,0 MB/)).toBeInTheDocument();
        await user.click(within(dialog).getByRole("button", { name: "Enviar arquivo" }));
        expect(servidor.callsFor("POST", "/files")).toHaveLength(0);
    });

    it("mostra o motivo que o servidor devolve quando recusa o arquivo", async () => {
        serverWith([], () =>
            apiError(400, "validation_error", "campo `arquivo` inválido", {
                field: "arquivo",
                reason:
                    "arquivo recusado: a extensão diz PDF, mas o conteúdo não é PDF. Envie o arquivo original.",
            }),
        );
        build();
        await screen.findByText("Nenhum arquivo na biblioteca");
        const user = userEvent.setup();

        await user.click(screen.getByRole("button", { name: "Enviar arquivo" }));
        await waitForModalFocus();
        const fake = new File(["oi"], "documento.pdf", { type: "application/pdf" });
        const input = document.querySelector('input[type="file"]') as HTMLInputElement;
        await user.upload(input, fake);
        const dialog = document.querySelector('[role="dialog"]') as HTMLElement;
        await user.click(within(dialog).getByRole("button", { name: "Enviar arquivo" }));

        expect(await within(dialog).findByText(/a extensão diz PDF, mas o conteúdo não é PDF/)).toBeInTheDocument();
        expect(dialog).toBeInTheDocument();
    });

    it("remove com confirmação pelo nome e tira o item da lista", async () => {
        const servidor = serverWith([file({ id: "arq_1", name: "Tabela de preços" })]);
        build();
        await screen.findByText("Tabela de preços");
        const user = userEvent.setup();

        await user.click(screen.getByRole("button", { name: "Excluir arquivo Tabela de preços" }));
        await waitForModalFocus();
        const dialog = document.querySelector('[role="dialog"]') as HTMLElement;
        const confirmAction = within(dialog).getByRole("button", { name: "Excluir arquivo" });
        expect(confirmAction).toBeDisabled();

        await user.type(within(dialog).getByLabelText(/para confirmar/), "Tabela de preços");
        expect(confirmAction).toBeEnabled();
        await user.click(confirmAction);

        await waitFor(() => expect(servidor.callsFor("DELETE", "/files/arq_1")).toHaveLength(1));
        await waitFor(() => expect(screen.queryByText("Tabela de preços")).not.toBeInTheDocument());
    });

    it("usa o cabeçalho comum do agente e a biblioteca como seção plana, sem cartão", async () => {
        serverWith([file({ id: "arq_1", name: "Tabela de preços" })]);
        const { container } = build();

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
        serverWith([
            file({
                id: "arq_1",
                name: "Price table",
                description: "Prices 2026",
                size_bytes: 3 * 1024 * 1024,
            }),
        ]);
        setLocale("en-US");
        build();

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
        serverWith([]);
        setLocale("en-US");
        build();

        expect(await screen.findByText("No file in the library")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Upload first file" })).toBeEnabled();
    });

    it("recusa o arquivo acima de 16 MB e envia com os metadados em inglês", async () => {
        const servidor = serverWith([]);
        setLocale("en-US");
        build();
        await screen.findByText("No file in the library");
        const user = userEvent.setup();

        await user.click(screen.getByRole("button", { name: "Upload file" }));
        await waitForModalFocus();
        const dialog = document.querySelector('[role="dialog"]') as HTMLElement;
        expect(within(dialog).getByText("Upload file to the agent")).toBeInTheDocument();

        const large = new File([new Uint8Array(LIMIT + 1024 * 1024)], "catalog.pdf", {
            type: "application/pdf",
        });
        const input = document.querySelector('input[type="file"]') as HTMLInputElement;
        await user.upload(input, large);
        expect(
            await within(dialog).findByText(/over the 16 MB per-file limit by 1\.0 MB/),
        ).toBeInTheDocument();
        await user.click(within(dialog).getByRole("button", { name: "Upload file" }));
        expect(servidor.callsFor("POST", "/files")).toHaveLength(0);

        const pequeno = new File([new Uint8Array(69)], "pixel.png", { type: "image/png" });
        await user.upload(input, pequeno);
        await user.type(
            within(dialog).getByLabelText(/Name shown to the customer/),
            "Clinic map",
        );
        expect(
            within(dialog).getByText(/Chosen: pixel\.png · 0\.0 MB \(within the 16 MB limit\)/),
        ).toBeInTheDocument();
        await user.click(within(dialog).getByRole("button", { name: "Upload file" }));

        await waitFor(() => expect(servidor.callsFor("POST", "/files").length).toBe(1));
        expect(await screen.findByText("Clinic map")).toBeInTheDocument();
        expect(screen.getByRole("status")).toHaveTextContent('File "Clinic map" uploaded');
    });

    it("remove com confirmação pelo nome em inglês e avisa na lista", async () => {
        const servidor = serverWith([file({ id: "arq_1", name: "Price table" })]);
        setLocale("en-US");
        build();
        await screen.findByText("Price table");
        const user = userEvent.setup();

        await user.click(screen.getByRole("button", { name: "Delete file Price table" }));
        await waitForModalFocus();
        const dialog = document.querySelector('[role="dialog"]') as HTMLElement;
        expect(within(dialog).getByText("Delete file?")).toBeInTheDocument();
        const confirmAction = within(dialog).getByRole("button", { name: "Delete file" });
        expect(confirmAction).toBeDisabled();

        await user.type(within(dialog).getByRole("textbox"), "Price table");
        await user.click(confirmAction);

        await waitFor(() => expect(servidor.callsFor("DELETE", "/files/arq_1")).toHaveLength(1));
        await waitFor(() => expect(screen.queryByText("Price table")).not.toBeInTheDocument());
        expect(screen.getByRole("status")).toHaveTextContent('File "Price table" removed');
    });
});
