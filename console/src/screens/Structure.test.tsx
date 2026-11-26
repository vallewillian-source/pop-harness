
import { afterEach, describe, expect, it } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { setLocale } from "../i18n";
import { waitForModalFocus } from "../testing/modal";
import { FakeServer, apiError } from "../testing/server";
import { Structure } from "./Structure";

const ORG = "org_1";

function departamento(id: string, nome: string, descricao = "") {
    return {
        id,
        org_id: ORG,
        name: nome,
        description: descricao,
        status: "active",
        version: 1,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
    };
}

function category(id: string, nome: string) {
    return { ...departamento(id, nome), description: "" };
}

function build() {
    return render(
        <MemoryRouter initialEntries={[`/orgs/${ORG}/structure`]}>
            <Routes>
                <Route path="/orgs/:orgId/structure" element={<Structure />} />
            </Routes>
        </MemoryRouter>,
    );
}

describe("Estrutura", () => {
    it("mostra departamentos com a contagem de agentes e categorias na mesma tela", async () => {
        new FakeServer()
            .respond("GET", `/admin/v1/orgs/${ORG}/departments`, {
                status: 200,
                body: {
                    items: [
                        departamento("dep_1", "Financeiro", "Cobrança e convênios"),
                        departamento("dep_2", "Plantão"),
                    ],
                    next_cursor: null,
                },
            })
            .respond(
                "GET",
                `/admin/v1/orgs/${ORG}/agents`,
                {
                    status: 200,
                    body: {
                        items: [
                            { id: "ag_1", name: "Atendente", default_department_id: "dep_1", human_handoff_department_id: "dep_1" },
                            { id: "ag_2", name: "Triagem", default_department_id: null, human_handoff_department_id: "dep_1" },
                        ],
                        next_cursor: null,
                    },
                },
            )
            .respond("GET", `/admin/v1/orgs/${ORG}/categories`, {
                status: 200,
                body: { items: [category("cat_1", "Ortodontia")], next_cursor: null },
            })
            .respond("GET", `/admin/v1/orgs/${ORG}`, {
                status: 200,
                body: { id: ORG, name: "Exemplo Odontologia" },
            })
            .install();

        build();

        const list = await screen.findByRole("list", { name: "Lista de departamentos" });
        expect(within(list).getByText("Financeiro")).toBeInTheDocument();
        expect(within(list).getByText("2 agentes")).toBeInTheDocument();
        expect(within(list).getByText("Plantão")).toBeInTheDocument();
        expect(within(list).getByText("0 agentes")).toBeInTheDocument();
        expect(
            within(screen.getByRole("list", { name: "Lista de categorias de produtos" })).getByText(
                "Ortodontia",
            ),
        ).toBeInTheDocument();
    });

    it("categoria tem botão de excluir, e a tela não diz mais que a exclusão ficou de fora", async () => {
        new FakeServer()
            .respond("GET", `/admin/v1/orgs/${ORG}/departments`, {
                status: 200,
                body: { items: [], next_cursor: null },
            })
            .respond("GET", `/admin/v1/orgs/${ORG}/agents`, { status: 200, body: { items: [], next_cursor: null } })
            .respond("GET", `/admin/v1/orgs/${ORG}/categories`, {
                status: 200,
                body: { items: [category("cat_1", "Ortodontia")], next_cursor: null },
            })
            .respond("GET", `/admin/v1/orgs/${ORG}`, { status: 200, body: { id: ORG, name: "Exemplo" } })
            .install();

        build();

        const list = await screen.findByRole("list", { name: "Lista de categorias de produtos" });
        expect(within(list).getByRole("button", { name: "Excluir categoria Ortodontia" })).toBeInTheDocument();
        expect(screen.queryByText(/Excluir categoria não está nesta tela/)).toBeNull();
    });

    it("estado vazio de departamento oferece o CTA e abre o formulário de criação", async () => {
        const user = userEvent.setup();
        new FakeServer()
            .respond("GET", `/admin/v1/orgs/${ORG}/departments`, {
                status: 200,
                body: { items: [], next_cursor: null },
            })
            .respond("GET", `/admin/v1/orgs/${ORG}/agents`, { status: 200, body: { items: [], next_cursor: null } })
            .respond("GET", `/admin/v1/orgs/${ORG}/categories`, {
                status: 200,
                body: { items: [], next_cursor: null },
            })
            .respond("GET", `/admin/v1/orgs/${ORG}`, { status: 200, body: { id: ORG, name: "Exemplo" } })
            .install();

        build();

        await user.click(await screen.findByRole("button", { name: "Criar primeiro departamento" }));
        await waitForModalFocus();

        expect(screen.getByRole("dialog", { name: "Novo departamento" })).toBeInTheDocument();
    });

    it("excluir departamento mostra quem fica sem ele, libera o botão só com o nome e apaga sem force", async () => {
        const user = userEvent.setup();
        let departments = [departamento("dep_1", "Financeiro"), departamento("dep_2", "Plantão")];
        const servidor = new FakeServer()
            .respond("DELETE", `/departments/dep_1`, () => {
                departments = departments.filter((item) => item.id !== "dep_1");
                return { status: 204 };
            })
            .respond("GET", `/admin/v1/orgs/${ORG}/departments`, () => ({
                status: 200,
                body: { items: departments, next_cursor: null },
            }))
            .respond("GET", `/admin/v1/orgs/${ORG}/agents`, {
                status: 200,
                body: {
                    items: [
                        { id: "ag_1", name: "Atendente", default_department_id: "dep_1", human_handoff_department_id: null },
                    ],
                    next_cursor: null,
                },
            })
            .respond("GET", `/admin/v1/orgs/${ORG}/categories`, { status: 200, body: { items: [], next_cursor: null } })
            .respond("GET", `/admin/v1/orgs/${ORG}`, { status: 200, body: { id: ORG, name: "Exemplo" } })
            .install();

        build();

        await user.click(await screen.findByRole("button", { name: "Excluir departamento Financeiro" }));
        const dialog = await screen.findByRole("dialog", { name: "Excluir departamento?" });
        await waitForModalFocus();

        expect(await within(dialog).findByText(/1 agente usa este departamento/)).toBeInTheDocument();
        expect(within(dialog).getByText("Atendente")).toBeInTheDocument();

        const confirmAction = within(dialog).getByRole("button", { name: "Excluir departamento" });
        expect(confirmAction).toBeDisabled();

        await user.type(within(dialog).getByRole("textbox"), "Financeir");
        expect(confirmAction).toBeDisabled();
        await user.type(within(dialog).getByRole("textbox"), "o");
        expect(confirmAction).toBeEnabled();

        await user.click(confirmAction);

        await waitFor(() =>
            expect(screen.queryByRole("dialog", { name: "Excluir departamento?" })).toBeNull(),
        );
        const [order] = servidor.callsFor("DELETE", "/departments/dep_1");
        expect(order).toBeDefined();
        expect(order?.url).not.toContain("force");
        await waitFor(() =>
            expect(screen.queryByRole("button", { name: "Excluir departamento Financeiro" })).toBeNull(),
        );
    });

    it("erro de categorias não derruba a lista de departamentos e tenta de novo sozinho", async () => {
        const user = userEvent.setup();
        let failures = 1;
        new FakeServer()
            .respond("GET", `/admin/v1/orgs/${ORG}/departments`, {
                status: 200,
                body: { items: [departamento("dep_1", "Financeiro")], next_cursor: null },
            })
            .respond("GET", `/admin/v1/orgs/${ORG}/agents`, { status: 200, body: { items: [], next_cursor: null } })
            .respond("GET", `/admin/v1/orgs/${ORG}/categories`, () => {
                if (failures > 0) {
                    failures -= 1;
                    return apiError(500, "internal", "falha simulada");
                }
                return { status: 200, body: { items: [category("cat_1", "Implantes")], next_cursor: null } };
            })
            .respond("GET", `/admin/v1/orgs/${ORG}`, { status: 200, body: { id: ORG, name: "Exemplo" } })
            .install();

        build();

        expect(await screen.findByText("Financeiro")).toBeInTheDocument();
        expect(await screen.findByText("Não consegui carregar categorias de produtos")).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Tentar de novo" }));
        expect(await screen.findByText("Implantes")).toBeInTheDocument();
        expect(screen.getByText("Financeiro")).toBeInTheDocument();
    });

    it("é duas seções planas com título, sem cartão", async () => {
        new FakeServer()
            .respond("GET", `/admin/v1/orgs/${ORG}/departments`, { status: 200, body: { items: [departamento("dep_1", "Financeiro")], next_cursor: null } })
            .respond("GET", `/admin/v1/orgs/${ORG}/agents`, { status: 200, body: { items: [], next_cursor: null } })
            .respond("GET", `/admin/v1/orgs/${ORG}/categories`, { status: 200, body: { items: [category("cat_1", "Ortodontia")], next_cursor: null } })
            .respond("GET", `/admin/v1/orgs/${ORG}`, { status: 200, body: { id: ORG, name: "Exemplo" } })
            .install();

        const { container } = build();

        await screen.findByRole("list", { name: "Lista de departamentos" });
        expect(screen.getByRole("heading", { level: 2, name: "Departamentos" })).toBeInTheDocument();
        expect(screen.getByRole("heading", { level: 2, name: "Categorias de produtos" })).toBeInTheDocument();
        expect(container.querySelector(".sofa-card")).toBeNull();
        expect(container.querySelectorAll("section.poph-section")).toHaveLength(2);
    });

    it("renders in en-US and updates live when locale changes", async () => {
        setLocale("en-US");
        new FakeServer()
            .respond("GET", `/admin/v1/orgs/${ORG}/departments`, {
                status: 200,
                body: { items: [departamento("dep_1", "Finance")], next_cursor: null },
            })
            .respond("GET", `/admin/v1/orgs/${ORG}/agents`, {
                status: 200,
                body: { items: [], next_cursor: null },
            })
            .respond("GET", `/admin/v1/orgs/${ORG}/categories`, {
                status: 200,
                body: { items: [category("cat_1", "Orthodontics")], next_cursor: null },
            })
            .respond("GET", `/admin/v1/orgs/${ORG}`, {
                status: 200,
                body: { id: ORG, name: "Example" },
            })
            .install();

        build();

        expect(
            await screen.findByRole("heading", { level: 1, name: "Departments and categories" }),
        ).toBeInTheDocument();
        expect(screen.getByRole("heading", { level: 2, name: "Departments" })).toBeInTheDocument();
        expect(
            screen.getByRole("heading", { level: 2, name: "Product categories" }),
        ).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "New department" })).toBeInTheDocument();
        expect(screen.getAllByText("Active")).toHaveLength(2);

        act(() => {
            setLocale("pt-BR");
        });
        expect(
            await screen.findByRole("heading", { level: 1, name: "Departamentos e categorias" }),
        ).toBeInTheDocument();
        expect(screen.getAllByText("Ativo")).toHaveLength(2);
    });
});

afterEach(() => {
    setLocale("pt-BR");
});
