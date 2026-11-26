
import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Registro } from "../api/structure";
import { setLocale } from "../i18n";
import { waitForModalFocus } from "../testing/modal";
import { FakeServer, apiError } from "../testing/server";
import { ModalStructure } from "../components/ModalStructure";

const CATEGORY: Registro = {
    id: "cat_1",
    org_id: "org_1",
    name: "Ortodontia",
    description: "Aparelhos",
    status: "active",
    version: 2,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
};

function fieldName(): HTMLElement {
    return screen.getByRole("textbox", { name: "Nome" });
}

describe("ModalEstrutura", () => {
    it("cria categoria enviando nome e descrição e fecha ao concluir", async () => {
        const user = userEvent.setup();
        const onSave = vi.fn();
        const onClose = vi.fn();
        const created: Registro = { ...CATEGORY, id: "cat_9", name: "Implantes", description: "" };
        const servidor = new FakeServer()
            .responder("POST", "/admin/v1/orgs/org_1/categories", { status: 201, corpo: created })
            .instalar();

        render(
            <ModalStructure
                tipo="categoria"
                org="org_1"
                aberto
                registro={null}
                onClose={onClose}
                onSave={onSave}
            />,
        );
        await waitForModalFocus();

        await user.type(fieldName(), "Implantes");
        await user.click(screen.getByRole("button", { name: "Criar categoria" }));

        await waitFor(() => expect(onSave).toHaveBeenCalledWith(created));
        expect(onClose).toHaveBeenCalled();
        const [request] = servidor.chamadasPara("POST", "/categories");
        expect(request?.corpo).toEqual({ name: "Implantes" });
    });

    it("o título de criar categoria é 'Nova categoria' (o feminino não vaza)", async () => {
        new FakeServer().instalar();
        render(
            <ModalStructure
                tipo="categoria"
                org="org_1"
                aberto
                registro={null}
                onClose={vi.fn()}
                onSave={vi.fn()}
            />,
        );
        expect(await screen.findByRole("dialog", { name: "Nova categoria" })).toBeInTheDocument();
    });

    it("nome vazio não chama a API e mostra o erro no campo Nome", async () => {
        const user = userEvent.setup();
        const servidor = new FakeServer().instalar();

        render(
            <ModalStructure
                tipo="departamento"
                org="org_1"
                aberto
                registro={null}
                onClose={vi.fn()}
                onSave={vi.fn()}
            />,
        );
        await waitForModalFocus();

        await user.click(screen.getByRole("button", { name: "Criar departamento" }));

        expect(await screen.findByText("informe o nome do departamento")).toBeInTheDocument();
        expect(servidor.calls).toHaveLength(0);
    });

    it("nome repetido volta da API e aparece no campo Nome, com o motivo real", async () => {
        const user = userEvent.setup();
        new FakeServer()
            .responder(
                "POST",
                "/departments",
                apiError(400, "validation_error", "campo `nome` inválido", {
                    field: "nome",
                    reason: "já existe um registro com esse valor nesta org",
                }),
            )
            .instalar();

        render(
            <ModalStructure
                tipo="departamento"
                org="org_1"
                aberto
                registro={null}
                onClose={vi.fn()}
                onSave={vi.fn()}
            />,
        );
        await waitForModalFocus();

        await user.type(fieldName(), "Financeiro");
        await user.click(screen.getByRole("button", { name: "Criar departamento" }));

        expect(
            await screen.findByText("já existe um registro com esse valor nesta org"),
        ).toBeInTheDocument();
        expect(fieldName()).toHaveValue("Financeiro");
    });

    it("412 ao editar relê a categoria, mantém o que foi digitado e salva com a versão nova", async () => {
        const user = userEvent.setup();
        const onSave = vi.fn();
        const onClose = vi.fn();
        let attempt = 0;
        const servidor = new FakeServer()
            .responder("PATCH", "/categories/cat_1", () => {
                attempt += 1;
                if (attempt === 1) {
                    return apiError(412, "version_conflict", "mudou desde a leitura", {
                        current_version: 4,
                        expected_version: 2,
                    });
                }
                return {
                    status: 200,
                    corpo: { ...CATEGORY, name: "Ortodontia digitada", version: 5 },
                };
            })
            .responder("GET", "/categories/cat_1", {
                status: 200,
                corpo: { ...CATEGORY, name: "Ortodontia (outra pessoa)", version: 4 },
            })
            .instalar();

        render(
            <ModalStructure
                tipo="categoria"
                org="org_1"
                aberto
                registro={CATEGORY}
                onClose={onClose}
                onSave={onSave}
            />,
        );
        await waitForModalFocus();

        await user.clear(fieldName());
        await user.type(fieldName(), "Ortodontia digitada");
        await user.click(screen.getByRole("button", { name: "Salvar alterações" }));

        expect(await screen.findByText(/Outra pessoa salvou esta categoria/)).toBeInTheDocument();
        expect(screen.getByText(/agora na versão 4/)).toBeInTheDocument();
        expect(fieldName()).toHaveValue("Ortodontia digitada");
        expect(onClose).not.toHaveBeenCalled();

        await user.click(screen.getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() => expect(onClose).toHaveBeenCalled());

        const patches = servidor.chamadasPara("PATCH", "/categories/cat_1");
        expect(patches[0]?.cabecalhos["if-match"]).toBe("2");
        expect(patches[1]?.cabecalhos["if-match"]).toBe("4");
        expect(patches[1]?.corpo).toMatchObject({
            name: "Ortodontia digitada",
            status: "active",
        });
        expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ version: 5 }));
    });

    it("renders in en-US and reacts to live locale switch", async () => {
        const user = userEvent.setup();
        setLocale("en-US");
        render(
            <ModalStructure
                tipo="categoria"
                org="org_1"
                aberto
                registro={CATEGORY}
                onClose={vi.fn()}
                onSave={vi.fn()}
            />,
        );
        await waitForModalFocus();

        expect(screen.getByRole("heading", { name: "Edit category" })).toBeInTheDocument();
        expect(screen.getByRole("option", { name: "Active" })).toBeInTheDocument();
        expect(screen.getByRole("option", { name: "Inactive" })).toBeInTheDocument();
        expect(screen.getByRole("option", { name: "Archived" })).toBeInTheDocument();

        await user.clear(screen.getByLabelText(/Name/i));
        await user.click(screen.getByRole("button", { name: "Save changes" }));
        expect(await screen.findByText("enter the category name")).toBeInTheDocument();

        act(() => {
            setLocale("pt-BR");
        });
        expect(await screen.findByRole("heading", { name: "Editar categoria" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Salvar alterações" })).toBeInTheDocument();
    });
});

afterEach(() => {
    setLocale("pt-BR");
});
