
import { useEffect } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { GuardChangesProvider, useGuardChanges } from "../components/GuardChanges";
import { FakeServer, apiError } from "../testing/server";
import { TabsAgent, HeaderAgent, type IdTabAgent } from "../components/TabsAgent";
import { Agents } from "./Agents";
import { setLocale } from "../i18n";

const ORG = "org_1";

function agente(partial: Record<string, unknown> = {}) {
    return {
        id: "ag_1",
        org_id: ORG,
        name: "Beatriz",
        slug: "beatriz",
        status: "active",
        default_department_id: "dep_1",
        human_handoff_department_id: null,
        version: 1,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
        ...partial,
    };
}

function build() {
    return render(
        <MemoryRouter initialEntries={[`/orgs/${ORG}/agents`]}>
            <Routes>
                <Route path="/orgs/:orgId/agents" element={<Agents />} />
            </Routes>
        </MemoryRouter>,
    );
}

function serverWith(items: unknown[]) {
    return new FakeServer()
        .respond("GET", `/admin/v1/orgs/${ORG}/departments`, {
            status: 200,
            body: {
                items: [
                    {
                        id: "dep_1",
                        org_id: ORG,
                        name: "Recepção",
                        description: "",
                        status: "active",
                        version: 1,
                        created_at: 1,
                        updated_at: 1,
                    },
                ],
                next_cursor: null,
            },
        })
        .respond("GET", `/admin/v1/orgs/${ORG}/agents`, {
            status: 200,
            body: { items, next_cursor: null },
        })
        .respond("GET", `/admin/v1/orgs/${ORG}`, {
            status: 200,
            body: { id: ORG, name: "Exemplo Odontologia" },
        })
        .install();
}

function Destination() {
    const local = useLocation();
    return <output data-testid="path">{local.pathname}</output>;
}

function EditPending() {
    const guard = useGuardChanges();
    useEffect(() => {
        guard.register({ dirty: true, save: async () => true });
    }, [guard]);
    return null;
}

function path(): string | null {
    return screen.getByTestId("path").textContent;
}

function buildTabs(active: IdTabAgent = "instructions", options: { dirty?: boolean } = {}) {
    return render(
        <MemoryRouter initialEntries={[`/orgs/${ORG}/agents/ag_1/instructions`]}>
            <GuardChangesProvider>
                {options.dirty ? <EditPending /> : null}
                <TabsAgent org={ORG} agente="ag_1" active={active} />
                <Destination />
            </GuardChangesProvider>
        </MemoryRouter>,
    );
}

function buildHeader() {
    return render(
        <MemoryRouter initialEntries={[`/orgs/${ORG}/agents/ag_1/instructions`]}>
            <GuardChangesProvider>
                <Routes>
                    <Route
                        path="*"
                        element={
                            <>
                                <HeaderAgent
                                    org={ORG}
                                    nome="Beatriz"
                                    slug="beatriz"
                                    situacao="active"
                                    organization="Exemplo Odontologia"
                                    descricao="modelo, busca na base e portões"
                                />
                                <Destination />
                            </>
                        }
                    />
                </Routes>
            </GuardChangesProvider>
        </MemoryRouter>,
    );
}

describe("Agentes", () => {
    it("lista os agentes da organização com o departamento padrão resolvido", async () => {
        serverWith([agente(), agente({ id: "ag_2", name: "Triagem", slug: "triagem", default_department_id: null })]);
        build();

        expect(await screen.findByText("Beatriz")).toBeInTheDocument();
        expect(screen.getByText("Triagem")).toBeInTheDocument();
        expect(screen.getByText("Recepção")).toBeInTheDocument();
        expect(screen.getByText(/Exemplo Odontologia/)).toBeInTheDocument();
        expect(screen.getAllByRole("button", { name: "Abrir instruções" })).toHaveLength(2);
    });

    it("estado vazio diz onde criar agente, sem oferecer CRUD que ainda não existe", async () => {
        serverWith([]);
        build();

        expect(await screen.findByText("Nenhum agente nesta organização")).toBeInTheDocument();
        expect(screen.getByText(/poph agent create/)).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Novo agente" })).toBeNull();
    });

    it("falha da API mostra o estado de erro e o botão repete a consulta", async () => {
        const user = userEvent.setup();
        let fail = true;
        new FakeServer()
            .respond("GET", `/admin/v1/orgs/${ORG}/departments`, { status: 200, body: { items: [], next_cursor: null } })
            .respond("GET", `/admin/v1/orgs/${ORG}/agents`, () =>
                fail
                    ? apiError(500, "internal_error", "a instância falhou")
                    : { status: 200, body: { items: [agente()], next_cursor: null } },
            )
            .respond("GET", `/admin/v1/orgs/${ORG}`, { status: 200, body: { id: ORG, name: "Exemplo" } })
            .install();
        build();

        expect(await screen.findByText("Não consegui carregar os agentes")).toBeInTheDocument();
        fail = false;
        await user.click(screen.getByRole("button", { name: "Tentar de novo" }));
        await waitFor(() => expect(screen.getByText("Beatriz")).toBeInTheDocument());
    });
});

describe("AbasDoAgente (RL-09)", () => {
    it("mostra as seis abas na ordem do TASKS e marca a ativa", () => {
        buildTabs();

        const tabs = screen.getAllByRole("tab").map((tab) => tab.textContent);
        expect(tabs).toEqual([
            "Instruções",
            "Base de conhecimento",
            "Arquivos",
            "Roteamento",
            "Integrações",
            "Modelo e Jev",
            "Chat de teste",
        ]);
        expect(screen.queryByRole("tab", { name: "Produtos" })).toBeNull();

        const active = screen.getByRole("tab", { name: "Instruções" });
        expect(active).toHaveAttribute("aria-selected", "true");
        expect(active).toHaveAttribute("aria-current", "page");
        expect(screen.getByRole("tablist", { name: "Seções do agente" })).toBeInTheDocument();
    });

    it("cada aba leva à rota real do agente", async () => {
        const user = userEvent.setup();
        buildTabs();

        await user.click(screen.getByRole("tab", { name: "Arquivos" }));
        expect(path()).toBe(`/orgs/${ORG}/agents/ag_1/files`);

        await user.click(screen.getByRole("tab", { name: "Integrações" }));
        expect(path()).toBe(`/orgs/${ORG}/agents/ag_1/integrations`);

        await user.click(screen.getByRole("tab", { name: "Modelo e Jev" }));
        expect(path()).toBe(`/orgs/${ORG}/agents/ag_1/model-jev`);

        await user.click(screen.getByRole("tab", { name: "Chat de teste" }));
        expect(path()).toBe(`/orgs/${ORG}/agents/ag_1/test-chat`);
    });

    it("a aba Chat de teste é alcançável e acionável só pelo teclado", async () => {
        const user = userEvent.setup();
        buildTabs();

        await user.tab();
        expect(screen.getByRole("tab", { name: "Instruções" })).toHaveFocus();

        await user.keyboard("{End}");
        const last = screen.getByRole("tab", { name: "Chat de teste" });
        expect(last).toHaveFocus();
        expect(path()).toBe(`/orgs/${ORG}/agents/ag_1/instructions`);

        await user.keyboard("{Enter}");
        expect(path()).toBe(`/orgs/${ORG}/agents/ag_1/test-chat`);
    });

    it("troca de aba com alterações pendentes para na guarda e não sai da tela", async () => {
        const user = userEvent.setup();
        buildTabs("instructions", { dirty: true });

        await user.click(screen.getByRole("tab", { name: "Roteamento" }));

        expect(path()).toBe(`/orgs/${ORG}/agents/ag_1/instructions`);
        expect(await screen.findByText(/alterações não salvas/i)).toBeInTheDocument();
    });
});

describe("CabecalhoDoAgente (RL-09)", () => {
    it("mostra organização, nome, situação e slug, e volta para a lista de agentes", async () => {
        const user = userEvent.setup();
        buildHeader();

        expect(screen.getByText("Agente · Exemplo Odontologia")).toBeInTheDocument();
        expect(screen.getByRole("heading", { name: "Beatriz" })).toBeInTheDocument();
        expect(screen.getByText("Ativo")).toBeInTheDocument();
        expect(screen.getByText("beatriz")).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Voltar para agentes" }));
        expect(path()).toBe(`/orgs/${ORG}/agents`);
    });
});

describe("I18N-04: tela de agentes e abas em en-US", () => {
    afterEach(() => {
        act(() => setLocale("pt-BR"));
    });

    it("a lista de agentes troca cabeçalhos, situação e ações para inglês", async () => {
        serverWith([agente()]);
        setLocale("en-US");
        build();

        expect(await screen.findByText("Beatriz")).toBeInTheDocument();
        expect(screen.getByRole("heading", { name: /Service agents/ })).toBeInTheDocument();
        expect(screen.getByText(/Organization · Exemplo Odontologia/)).toBeInTheDocument();
        expect(screen.getByRole("columnheader", { name: "Agent" })).toBeInTheDocument();
        expect(screen.getByRole("columnheader", { name: "Status" })).toBeInTheDocument();
        expect(screen.getByRole("columnheader", { name: "Default department" })).toBeInTheDocument();
        expect(screen.getByRole("columnheader", { name: "Version" })).toBeInTheDocument();
        expect(screen.getByRole("columnheader", { name: "Actions" })).toBeInTheDocument();
        expect(screen.getByText("Active")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Open instructions" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Refresh" })).toBeInTheDocument();
        expect(screen.getByText("1 agent")).toBeInTheDocument();
    });

    it("o estado vazio explica em inglês onde criar agente", async () => {
        serverWith([]);
        setLocale("en-US");
        build();

        expect(await screen.findByText("No agent in this organization")).toBeInTheDocument();
        expect(screen.getByText(/poph agent create/)).toBeInTheDocument();
    });

    it("a falha da API mostra o estado de erro em inglês e o botão repete a consulta", async () => {
        const user = userEvent.setup();
        let fail = true;
        new FakeServer()
            .respond("GET", `/admin/v1/orgs/${ORG}/departments`, {
                status: 200,
                body: { items: [], next_cursor: null },
            })
            .respond("GET", `/admin/v1/orgs/${ORG}/agents`, () =>
                fail
                    ? apiError(500, "internal_error", "the instance failed")
                    : { status: 200, body: { items: [agente()], next_cursor: null } },
            )
            .respond("GET", `/admin/v1/orgs/${ORG}`, {
                status: 200,
                body: { id: ORG, name: "Exemplo" },
            })
            .install();
        setLocale("en-US");
        build();

        expect(await screen.findByText("Could not load the agents")).toBeInTheDocument();
        fail = false;
        await user.click(screen.getByRole("button", { name: "Try again" }));
        await waitFor(() => expect(screen.getByText("Beatriz")).toBeInTheDocument());
    });

    it("as abas e o cabeçalho do agente falam inglês", () => {
        setLocale("en-US");
        buildTabs();

        expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
            "Instructions",
            "Knowledge base",
            "Files",
            "Routing",
            "Integrations",
            "Model and Jev",
            "Test chat",
        ]);
        expect(screen.getByRole("tablist", { name: "Agent sections" })).toBeInTheDocument();

        buildHeader();
        expect(screen.getByText("Agent · Exemplo Odontologia")).toBeInTheDocument();
        expect(screen.getByText("Active")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Back to agents" })).toBeInTheDocument();
    });
});
