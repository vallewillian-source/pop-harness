/**
 * Testes da lista de agentes (HR-40) e da moldura das abas do agente (RL-09).
 *
 * A lista continua coberta no caminho feliz (agente com departamento), no estado vazio (a tela não
 * inventa CRUD) e no estado de erro com "tentar de novo". O re-layout acrescenta as abas: ordem,
 * rotas reais, aba ativa marcada, teclado e a guarda de alterações não salvas.
 */

import { useEffect } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { GuardChangesProvider, useGuardChanges } from "../components/GuardChanges";
import { FakeServer, apiError } from "../testing/server";
import { TabsAgent, HeaderAgent, type IdTabAgent } from "../components/TabsAgent";
import { Agents } from "./Agentes";
import { setLocale } from "../i18n";

const ORG = "org_1";

function agente(parcial: Record<string, unknown> = {}) {
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
        ...parcial,
    };
}

function montar() {
    return render(
        <MemoryRouter initialEntries={[`/orgs/${ORG}/agents`]}>
            <Routes>
                <Route path="/orgs/:orgId/agents" element={<Agents />} />
            </Routes>
        </MemoryRouter>,
    );
}

function servidorCom(items: unknown[]) {
    return new FakeServer()
        .responder("GET", `/admin/v1/orgs/${ORG}/departments`, {
            status: 200,
            corpo: {
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
        .responder("GET", `/admin/v1/orgs/${ORG}/agents`, {
            status: 200,
            corpo: { items, next_cursor: null },
        })
        .responder("GET", `/admin/v1/orgs/${ORG}`, {
            status: 200,
            corpo: { id: ORG, name: "Exemplo Odontologia" },
        })
        .instalar();
}

/** Mostra o caminho atual num elemento, para o teste conferir navegação. */
function Destino() {
    const local = useLocation();
    return <output data-testid="path">{local.pathname}</output>;
}

/** Registra uma edição pendente na guarda: é o que segura a troca de aba no teste. */
function EdicaoPendente() {
    const guarda = useGuardChanges();
    useEffect(() => {
        guarda.registrar({ dirty: true, salvar: async () => true });
    }, [guarda]);
    return null;
}

function caminho(): string | null {
    return screen.getByTestId("path").textContent;
}

function montarAbas(ativa: IdTabAgent = "instructions", opcoes: { sujo?: boolean } = {}) {
    return render(
        <MemoryRouter initialEntries={[`/orgs/${ORG}/agents/ag_1/instructions`]}>
            <GuardChangesProvider>
                {opcoes.sujo ? <EdicaoPendente /> : null}
                <TabsAgent org={ORG} agente="ag_1" active={ativa} />
                <Destino />
            </GuardChangesProvider>
        </MemoryRouter>,
    );
}

function montarCabecalho() {
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
                                    organizacao="Exemplo Odontologia"
                                    descricao="modelo, busca na base e portões"
                                />
                                <Destino />
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
        servidorCom([agente(), agente({ id: "ag_2", name: "Triagem", slug: "triagem", default_department_id: null })]);
        montar();

        expect(await screen.findByText("Beatriz")).toBeInTheDocument();
        expect(screen.getByText("Triagem")).toBeInTheDocument();
        expect(screen.getByText("Recepção")).toBeInTheDocument();
        expect(screen.getByText(/Exemplo Odontologia/)).toBeInTheDocument();
        expect(screen.getAllByRole("button", { name: "Abrir instruções" })).toHaveLength(2);
    });

    it("estado vazio diz onde criar agente, sem oferecer CRUD que ainda não existe", async () => {
        servidorCom([]);
        montar();

        expect(await screen.findByText("Nenhum agente nesta organização")).toBeInTheDocument();
        expect(screen.getByText(/poph agent create/)).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Novo agente" })).toBeNull();
    });

    it("falha da API mostra o estado de erro e o botão repete a consulta", async () => {
        const usuario = userEvent.setup();
        let falhar = true;
        new FakeServer()
            .responder("GET", `/admin/v1/orgs/${ORG}/departments`, { status: 200, corpo: { items: [], next_cursor: null } })
            .responder("GET", `/admin/v1/orgs/${ORG}/agents`, () =>
                falhar
                    ? apiError(500, "internal_error", "a instância falhou")
                    : { status: 200, corpo: { items: [agente()], next_cursor: null } },
            )
            .responder("GET", `/admin/v1/orgs/${ORG}`, { status: 200, corpo: { id: ORG, name: "Exemplo" } })
            .instalar();
        montar();

        expect(await screen.findByText("Não consegui carregar os agentes")).toBeInTheDocument();
        falhar = false;
        await usuario.click(screen.getByRole("button", { name: "Tentar de novo" }));
        await waitFor(() => expect(screen.getByText("Beatriz")).toBeInTheDocument());
    });
});

describe("AbasDoAgente (RL-09)", () => {
    it("mostra as seis abas na ordem do TASKS e marca a ativa", () => {
        montarAbas();

        const abas = screen.getAllByRole("tab").map((aba) => aba.textContent);
        expect(abas).toEqual([
            "Instruções",
            "Base de conhecimento",
            "Arquivos",
            "Roteamento",
            "Integrações",
            "Modelo e Jev",
            "Chat de teste",
        ]);
        // "Produtos" não tem rota no App.tsx e saiu da moldura.
        expect(screen.queryByRole("tab", { name: "Produtos" })).toBeNull();

        const ativa = screen.getByRole("tab", { name: "Instruções" });
        expect(ativa).toHaveAttribute("aria-selected", "true");
        expect(ativa).toHaveAttribute("aria-current", "page");
        expect(screen.getByRole("tablist", { name: "Seções do agente" })).toBeInTheDocument();
    });

    it("cada aba leva à rota real do agente", async () => {
        const usuario = userEvent.setup();
        montarAbas();

        await usuario.click(screen.getByRole("tab", { name: "Arquivos" }));
        expect(caminho()).toBe(`/orgs/${ORG}/agents/ag_1/files`);

        await usuario.click(screen.getByRole("tab", { name: "Integrações" }));
        expect(caminho()).toBe(`/orgs/${ORG}/agents/ag_1/integrations`);

        await usuario.click(screen.getByRole("tab", { name: "Modelo e Jev" }));
        expect(caminho()).toBe(`/orgs/${ORG}/agents/ag_1/model-jev`);

        await usuario.click(screen.getByRole("tab", { name: "Chat de teste" }));
        expect(caminho()).toBe(`/orgs/${ORG}/agents/ag_1/test-chat`);
    });

    it("a aba Chat de teste é alcançável e acionável só pelo teclado", async () => {
        const usuario = userEvent.setup();
        montarAbas();

        // Tab entra na aba ativa (as outras têm tabIndex -1).
        await usuario.tab();
        expect(screen.getByRole("tab", { name: "Instruções" })).toHaveFocus();

        // End leva o foco à última aba, sem trocar de aba durante o percurso.
        await usuario.keyboard("{End}");
        const ultima = screen.getByRole("tab", { name: "Chat de teste" });
        expect(ultima).toHaveFocus();
        expect(caminho()).toBe(`/orgs/${ORG}/agents/ag_1/instructions`);

        await usuario.keyboard("{Enter}");
        expect(caminho()).toBe(`/orgs/${ORG}/agents/ag_1/test-chat`);
    });

    it("troca de aba com alterações pendentes para na guarda e não sai da tela", async () => {
        const usuario = userEvent.setup();
        montarAbas("instructions", { sujo: true });

        await usuario.click(screen.getByRole("tab", { name: "Roteamento" }));

        expect(caminho()).toBe(`/orgs/${ORG}/agents/ag_1/instructions`);
        expect(await screen.findByText(/alterações não salvas/i)).toBeInTheDocument();
    });
});

describe("CabecalhoDoAgente (RL-09)", () => {
    it("mostra organização, nome, situação e slug, e volta para a lista de agentes", async () => {
        const usuario = userEvent.setup();
        montarCabecalho();

        expect(screen.getByText("Agente · Exemplo Odontologia")).toBeInTheDocument();
        expect(screen.getByRole("heading", { name: "Beatriz" })).toBeInTheDocument();
        expect(screen.getByText("Ativo")).toBeInTheDocument();
        expect(screen.getByText("beatriz")).toBeInTheDocument();

        await usuario.click(screen.getByRole("button", { name: "Voltar para agentes" }));
        expect(caminho()).toBe(`/orgs/${ORG}/agents`);
    });
});

describe("I18N-04: tela de agentes e abas em en-US", () => {
    afterEach(() => {
        act(() => setLocale("pt-BR"));
    });

    it("a lista de agentes troca cabeçalhos, situação e ações para inglês", async () => {
        servidorCom([agente()]);
        setLocale("en-US");
        montar();

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
        servidorCom([]);
        setLocale("en-US");
        montar();

        expect(await screen.findByText("No agent in this organization")).toBeInTheDocument();
        expect(screen.getByText(/poph agent create/)).toBeInTheDocument();
    });

    it("a falha da API mostra o estado de erro em inglês e o botão repete a consulta", async () => {
        const usuario = userEvent.setup();
        let falhar = true;
        new FakeServer()
            .responder("GET", `/admin/v1/orgs/${ORG}/departments`, {
                status: 200,
                corpo: { items: [], next_cursor: null },
            })
            .responder("GET", `/admin/v1/orgs/${ORG}/agents`, () =>
                falhar
                    ? apiError(500, "internal_error", "the instance failed")
                    : { status: 200, corpo: { items: [agente()], next_cursor: null } },
            )
            .responder("GET", `/admin/v1/orgs/${ORG}`, {
                status: 200,
                corpo: { id: ORG, name: "Exemplo" },
            })
            .instalar();
        setLocale("en-US");
        montar();

        expect(await screen.findByText("Could not load the agents")).toBeInTheDocument();
        falhar = false;
        await usuario.click(screen.getByRole("button", { name: "Try again" }));
        await waitFor(() => expect(screen.getByText("Beatriz")).toBeInTheDocument());
    });

    it("as abas e o cabeçalho do agente falam inglês", () => {
        setLocale("en-US");
        montarAbas();

        expect(screen.getAllByRole("tab").map((aba) => aba.textContent)).toEqual([
            "Instructions",
            "Knowledge base",
            "Files",
            "Routing",
            "Integrations",
            "Model and Jev",
            "Test chat",
        ]);
        expect(screen.getByRole("tablist", { name: "Agent sections" })).toBeInTheDocument();

        montarCabecalho();
        expect(screen.getByText("Agent · Exemplo Odontologia")).toBeInTheDocument();
        expect(screen.getByText("Active")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Back to agents" })).toBeInTheDocument();
    });
});
