/**
 * Testes do formulário de categoria e de departamento (HR-39b).
 *
 * Cobre criar (com o corpo que sai para a API), validação antes da chamada, erro de campo vindo da
 * API (nome repetido) e o `412` de concorrência, que precisa manter o que foi digitado.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Registro } from "../api/structure";
import { setLocale } from "../i18n";
import { waitForModalFocus } from "../testing/modal";
import { FakeServer, apiError } from "../testing/server";
import { ModalEstrutura } from "./ModalEstrutura";

const CATEGORIA: Registro = {
    id: "cat_1",
    org_id: "org_1",
    name: "Ortodontia",
    description: "Aparelhos",
    status: "active",
    version: 2,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
};

function campoNome(): HTMLElement {
    return screen.getByRole("textbox", { name: "Nome" });
}

describe("ModalEstrutura", () => {
    it("cria categoria enviando nome e descrição e fecha ao concluir", async () => {
        const usuario = userEvent.setup();
        const aoSalvar = vi.fn();
        const aoFechar = vi.fn();
        const criada: Registro = { ...CATEGORIA, id: "cat_9", name: "Implantes", description: "" };
        const servidor = new FakeServer()
            .responder("POST", "/admin/v1/orgs/org_1/categories", { status: 201, corpo: criada })
            .instalar();

        render(
            <ModalEstrutura
                tipo="categoria"
                org="org_1"
                aberto
                registro={null}
                aoFechar={aoFechar}
                aoSalvar={aoSalvar}
            />,
        );
        await waitForModalFocus();

        await usuario.type(campoNome(), "Implantes");
        await usuario.click(screen.getByRole("button", { name: "Criar categoria" }));

        await waitFor(() => expect(aoSalvar).toHaveBeenCalledWith(criada));
        expect(aoFechar).toHaveBeenCalled();
        const [pedido] = servidor.chamadasPara("POST", "/categories");
        expect(pedido?.corpo).toEqual({ name: "Implantes" });
    });

    it("o título de criar categoria é 'Nova categoria' (o feminino não vaza)", async () => {
        new FakeServer().instalar();
        render(
            <ModalEstrutura
                tipo="categoria"
                org="org_1"
                aberto
                registro={null}
                aoFechar={vi.fn()}
                aoSalvar={vi.fn()}
            />,
        );
        expect(await screen.findByRole("dialog", { name: "Nova categoria" })).toBeInTheDocument();
    });

    it("nome vazio não chama a API e mostra o erro no campo Nome", async () => {
        const usuario = userEvent.setup();
        const servidor = new FakeServer().instalar();

        render(
            <ModalEstrutura
                tipo="departamento"
                org="org_1"
                aberto
                registro={null}
                aoFechar={vi.fn()}
                aoSalvar={vi.fn()}
            />,
        );
        await waitForModalFocus();

        await usuario.click(screen.getByRole("button", { name: "Criar departamento" }));

        expect(await screen.findByText("informe o nome do departamento")).toBeInTheDocument();
        expect(servidor.chamadas).toHaveLength(0);
    });

    it("nome repetido volta da API e aparece no campo Nome, com o motivo real", async () => {
        const usuario = userEvent.setup();
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
            <ModalEstrutura
                tipo="departamento"
                org="org_1"
                aberto
                registro={null}
                aoFechar={vi.fn()}
                aoSalvar={vi.fn()}
            />,
        );
        await waitForModalFocus();

        await usuario.type(campoNome(), "Financeiro");
        await usuario.click(screen.getByRole("button", { name: "Criar departamento" }));

        expect(
            await screen.findByText("já existe um registro com esse valor nesta org"),
        ).toBeInTheDocument();
        expect(campoNome()).toHaveValue("Financeiro");
    });

    it("412 ao editar relê a categoria, mantém o que foi digitado e salva com a versão nova", async () => {
        const usuario = userEvent.setup();
        const aoSalvar = vi.fn();
        const aoFechar = vi.fn();
        let tentativa = 0;
        const servidor = new FakeServer()
            .responder("PATCH", "/categories/cat_1", () => {
                tentativa += 1;
                if (tentativa === 1) {
                    return apiError(412, "version_conflict", "mudou desde a leitura", {
                        current_version: 4,
                        expected_version: 2,
                    });
                }
                return {
                    status: 200,
                    corpo: { ...CATEGORIA, name: "Ortodontia digitada", version: 5 },
                };
            })
            .responder("GET", "/categories/cat_1", {
                status: 200,
                corpo: { ...CATEGORIA, name: "Ortodontia (outra pessoa)", version: 4 },
            })
            .instalar();

        render(
            <ModalEstrutura
                tipo="categoria"
                org="org_1"
                aberto
                registro={CATEGORIA}
                aoFechar={aoFechar}
                aoSalvar={aoSalvar}
            />,
        );
        await waitForModalFocus();

        await usuario.clear(campoNome());
        await usuario.type(campoNome(), "Ortodontia digitada");
        await usuario.click(screen.getByRole("button", { name: "Salvar alterações" }));

        expect(await screen.findByText(/Outra pessoa salvou esta categoria/)).toBeInTheDocument();
        expect(screen.getByText(/agora na versão 4/)).toBeInTheDocument();
        // O que foi digitado continua no campo: a tela não descarta o trabalho do operador.
        expect(campoNome()).toHaveValue("Ortodontia digitada");
        expect(aoFechar).not.toHaveBeenCalled();

        await usuario.click(screen.getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() => expect(aoFechar).toHaveBeenCalled());

        const patches = servidor.chamadasPara("PATCH", "/categories/cat_1");
        expect(patches[0]?.cabecalhos["if-match"]).toBe("2");
        expect(patches[1]?.cabecalhos["if-match"]).toBe("4");
        expect(patches[1]?.corpo).toMatchObject({
            name: "Ortodontia digitada",
            status: "active",
        });
        expect(aoSalvar).toHaveBeenCalledWith(expect.objectContaining({ version: 5 }));
    });

    it("renders in en-US and reacts to live locale switch", async () => {
        const usuario = userEvent.setup();
        setLocale("en-US");
        render(
            <ModalEstrutura
                tipo="categoria"
                org="org_1"
                aberto
                registro={CATEGORIA}
                aoFechar={vi.fn()}
                aoSalvar={vi.fn()}
            />,
        );
        await waitForModalFocus();

        expect(screen.getByRole("heading", { name: "Edit category" })).toBeInTheDocument();
        expect(screen.getByRole("option", { name: "Active" })).toBeInTheDocument();
        expect(screen.getByRole("option", { name: "Inactive" })).toBeInTheDocument();
        expect(screen.getByRole("option", { name: "Archived" })).toBeInTheDocument();

        await usuario.clear(screen.getByLabelText(/Name/i));
        await usuario.click(screen.getByRole("button", { name: "Save changes" }));
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
