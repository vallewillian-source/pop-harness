
import { afterEach, describe, expect, it } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { GuardChangesProvider } from "../components/GuardChanges";
import { FakeServer, apiError } from "../testing/server";
import { AgentRouting } from "./AgentRouting";
import { setLocale } from "../i18n";

const ORG = "org_1";
const AGENTE = "ag_1";

const DEPARTMENTS = [
    { id: "dep_1", org_id: ORG, name: "Atendimento geral", description: "", status: "active", version: 1, created_at: 1, updated_at: 1 },
    { id: "dep_2", org_id: ORG, name: "Suporte clínico", description: "", status: "active", version: 1, created_at: 1, updated_at: 1 },
    { id: "dep_3", org_id: ORG, name: "Financeiro", description: "", status: "inactive", version: 1, created_at: 1, updated_at: 1 },
];

function agentJson(partial: Record<string, unknown> = {}) {
    return {
        id: AGENTE,
        org_id: ORG,
        name: "Beatriz",
        slug: "beatriz",
        description: "",
        status: "active",
        default_department_id: "dep_1",
        human_handoff_enabled: false,
        human_handoff_department_id: null,
        jev: { despedida: "Até logo!", situacoes_de_handoff: ["cliente pede segunda via"] },
        version: 1,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
        ...partial,
    };
}

function build() {
    return render(
        <MemoryRouter initialEntries={[`/orgs/${ORG}/agents/${AGENTE}/routing`]}>
            <GuardChangesProvider>
                <Routes>
                    <Route path="/orgs/:orgId/agents/:agentId/routing" element={<AgentRouting />} />
                    <Route path="/orgs/:orgId/structure" element={<div>Tela de estrutura</div>} />
                    <Route path="/orgs/:orgId/agents" element={<div>Tela de agentes</div>} />
                </Routes>
            </GuardChangesProvider>
        </MemoryRouter>,
    );
}

function serverWith(options: {
    agente?: Record<string, unknown>;
    patches?: Array<{ status: number; corpo?: unknown }>;
    relido?: Record<string, unknown>;
} = {}) {
    let reads = 0;
    let patches = 0;
    const agente = options.agente ?? agentJson();
    const servidor = new FakeServer()
        .respond("GET", `/admin/v1/orgs/${ORG}/departments`, {
            status: 200,
            corpo: { items: DEPARTMENTS, next_cursor: null },
        })
        .respond("PATCH", `/agents/${AGENTE}`, () => {
            const response = options.patches?.[patches] ?? { status: 200, corpo: agentJson({ version: 2 }) };
            patches += 1;
            return response;
        })
        .respond("GET", `/agents/${AGENTE}`, () => {
            reads += 1;
            if (reads > 1 && options.relido) return { status: 200, corpo: options.relido };
            return { status: 200, corpo: agente };
        })
        .respond("GET", `/admin/v1/orgs/${ORG}`, {
            status: 200,
            corpo: { id: ORG, name: "Clínica Teste", status: "active", version: 1 },
        });
    return servidor.install();
}

describe("Aba Roteamento", () => {
    it("carrega o departamento padrão, a transferência e os motivos do agente", async () => {
        const servidor = serverWith();
        build();

        const standard = await screen.findByLabelText("Departamento padrão");
        expect(standard).toHaveValue("dep_1");
        expect(screen.getByRole("switch", { name: /Transferir para atendente humano/ })).not.toBeChecked();
        expect(screen.getByLabelText("Motivo 1")).toHaveValue("cliente pede segunda via");
        expect(servidor.callsFor("GET", "/departments")[0]?.url).toContain("limit=200");
    });

    it("grava só o departamento padrão e a transferência, com If-Match da versão do agente", async () => {
        const servidor = serverWith();
        build();
        await screen.findByLabelText("Departamento padrão");
        const user = userEvent.setup();

        await user.selectOptions(screen.getByLabelText("Departamento padrão"), "dep_2");
        await user.click(screen.getByRole("switch", { name: /Transferir para atendente humano/ }));
        await user.selectOptions(screen.getByLabelText("Departamento humano de destino"), "dep_2");
        await user.click(screen.getByRole("button", { name: "Salvar roteamento" }));

        await waitFor(() => expect(servidor.callsFor("PATCH", `/agents/${AGENTE}`)).toHaveLength(1));
        const send = servidor.callsFor("PATCH", `/agents/${AGENTE}`)[0]!;
        expect(send.headers["if-match"]).toBe("1");
        expect(send.corpo).toEqual({ department: "dep_2", handoff: true, handoff_department: "dep_2" });
        expect(await screen.findByRole("status")).toHaveTextContent("Roteamento gravado (versão 2)");
    });

    it("ao mudar os motivos, manda o jev_json com as outras chaves do agente preservadas", async () => {
        const servidor = serverWith();
        build();
        await screen.findByLabelText("Motivo 1");
        const user = userEvent.setup();

        await user.click(screen.getByRole("button", { name: "Adicionar motivo" }));
        await user.type(screen.getByLabelText("Motivo 2"), "reclamação de atraso");
        await user.click(screen.getByRole("button", { name: "Salvar roteamento" }));

        await waitFor(() => expect(servidor.callsFor("PATCH", `/agents/${AGENTE}`)).toHaveLength(1));
        const corpo = servidor.callsFor("PATCH", `/agents/${AGENTE}`)[0]!.corpo as { jev_json: string };
        expect(JSON.parse(corpo.jev_json)).toEqual({
            despedida: "Até logo!",
            situacoes_de_handoff: ["cliente pede segunda via", "reclamação de atraso"],
        });
        expect(Object.keys(corpo)).toEqual(["jev_json"]);
    });

    it("no 412 relê o agente, mantém o que foi digitado e grava de novo na versão atual", async () => {
        const servidor = serverWith({
            relido: agentJson({ version: 2, default_department_id: "dep_1", jev: { despedida: "Até logo!", situacoes_de_handoff: ["cliente pede segunda via"] } }),
            patches: [
                { status: 412, corpo: { code: "version_conflict", message: "agente mudou desde a leitura", details: { current_version: 2, expected_version: 1 } } },
                { status: 200, corpo: agentJson({ version: 3, default_department_id: "dep_2" }) },
            ],
        });
        build();
        await screen.findByLabelText("Departamento padrão");
        const user = userEvent.setup();

        await user.selectOptions(screen.getByLabelText("Departamento padrão"), "dep_2");
        await user.click(screen.getByRole("button", { name: "Salvar roteamento" }));

        expect(
            await screen.findByText(/Outra pessoa salvou este agente antes de você \(agora na versão 2\)/),
        ).toBeInTheDocument();
        expect(screen.getByLabelText("Departamento padrão")).toHaveValue("dep_2");

        await user.click(screen.getByRole("button", { name: "Salvar roteamento" }));
        await waitFor(() => expect(servidor.callsFor("PATCH", `/agents/${AGENTE}`)).toHaveLength(2));
        expect(servidor.callsFor("PATCH", `/agents/${AGENTE}`)[1]!.headers["if-match"]).toBe("2");
        expect(servidor.callsFor("PATCH", `/agents/${AGENTE}`)[1]!.corpo).toEqual({ department: "dep_2" });
    });

    it("exige o departamento humano para ligar a transferência, sem chamar a API", async () => {
        const servidor = serverWith();
        build();
        await screen.findByLabelText("Departamento padrão");
        const user = userEvent.setup();

        await user.click(screen.getByRole("switch", { name: /Transferir para atendente humano/ }));
        await user.click(screen.getByRole("button", { name: "Salvar roteamento" }));

        expect(await screen.findByText(/escolha o departamento humano de destino/)).toBeInTheDocument();
        expect(servidor.callsFor("PATCH", `/agents/${AGENTE}`)).toHaveLength(0);
    });

    it("não grava motivo acima de 300 caracteres nem deixa passar de 15 motivos", async () => {
        const reasons = Array.from({ length: 15 }, (_, i) => `motivo ${i + 1}`);
        const servidor = serverWith({ agente: agentJson({ jev: { situacoes_de_handoff: reasons } }) });
        build();
        await screen.findByLabelText("Motivo 15");
        const user = userEvent.setup();

        expect(screen.getByRole("button", { name: "Adicionar motivo" })).toBeDisabled();

        fireEvent.change(screen.getByLabelText("Motivo 1"), { target: { value: "a".repeat(301) } });
        await user.click(screen.getByRole("button", { name: "Salvar roteamento" }));

        expect(await screen.findByText("o motivo aceita até 300 caracteres")).toBeInTheDocument();
        expect(servidor.callsFor("PATCH", `/agents/${AGENTE}`)).toHaveLength(0);
    });

    it("mostra o código e o motivo reais quando a API recusa a gravação", async () => {
        const servidor = serverWith({
            patches: [
                apiError(404, "not_found", "departamento `dep_2` não encontrado", null),
            ],
        });
        build();
        await screen.findByLabelText("Departamento padrão");
        const user = userEvent.setup();

        await user.selectOptions(screen.getByLabelText("Departamento padrão"), "dep_2");
        await user.click(screen.getByRole("button", { name: "Salvar roteamento" }));

        expect(await screen.findByText(/departamento `dep_2` não encontrado \(código not_found\)/)).toBeInTheDocument();
        expect(servidor.callsFor("PATCH", `/agents/${AGENTE}`)).toHaveLength(1);
    });

    it("sem alteração, o botão de gravar fica desabilitado", async () => {
        serverWith();
        build();
        await screen.findByLabelText("Departamento padrão");

        expect(screen.getByRole("button", { name: "Salvar roteamento" })).toBeDisabled();
        expect(screen.getByText(/Salvo \(versão 1\)/)).toBeInTheDocument();
    });

    it("sem departamentos na org, mostra o estado vazio com caminho para Estrutura", async () => {
        const servidor = new FakeServer()
            .respond("GET", `/admin/v1/orgs/${ORG}/departments`, { status: 200, corpo: { items: [], next_cursor: null } })
            .respond("GET", `/agents/${AGENTE}`, { status: 200, corpo: agentJson() })
            .respond("GET", `/admin/v1/orgs/${ORG}`, { status: 200, corpo: { id: ORG, name: "Clínica", status: "active", version: 1 } });
        servidor.install();
        build();

        expect(await screen.findByText("Nenhum departamento na organização")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Ir para Estrutura" })).toBeEnabled();
    });

    it("usa o cabeçalho comum do agente e seções planas, sem cartão", async () => {
        serverWith();
        const { container } = build();

        expect(await screen.findByRole("heading", { level: 1, name: "Beatriz" })).toBeInTheDocument();
        expect(screen.getByRole("region", { name: "Departamentos do agente" })).toBeInTheDocument();
        expect(screen.getByRole("region", { name: "Motivos de transferência" })).toBeInTheDocument();
        expect(container.querySelector(".sofa-card")).toBeNull();
    });

    it("a barra de salvar usa o ponto de status do re-layout nos dois estados (RL-20)", async () => {
        serverWith();
        const { container } = build();
        await screen.findByLabelText("Departamento padrão");

        const barra = container.querySelector(".poph-routing__bar");
        expect(barra).not.toBeNull();
        expect(barra!.querySelector(".sofa-status-indicator")).toBeNull();

        const saved = barra!.querySelector(".poph-status");
        expect(saved).toHaveClass("poph-status--inactive");
        expect(saved!.querySelector(".poph-status__point")).not.toBeNull();
        expect(saved).toHaveTextContent("Salvo (versão 1)");

        const user = userEvent.setup();
        await user.selectOptions(screen.getByLabelText("Departamento padrão"), "dep_2");

        const dirty = container.querySelector(".poph-routing__bar .poph-status");
        expect(dirty).toHaveClass("poph-status--attention");
        expect(dirty!.querySelector(".poph-status__point")).not.toBeNull();
        expect(dirty).toHaveTextContent("Alterações não salvas");
    });
});

describe("I18N-07: aba de Roteamento em en-US", () => {
    afterEach(() => {
        act(() => setLocale("pt-BR"));
    });

    it("os campos, as seções e a barra de salvar falam inglês", async () => {
        setLocale("en-US");
        serverWith();
        build();

        expect(await screen.findByLabelText("Default department")).toHaveValue("dep_1");
        expect(
            screen.getByRole("switch", { name: /Transfer to a human agent/ }),
        ).not.toBeChecked();
        expect(screen.getByLabelText("Human destination department")).toBeInTheDocument();
        expect(
            within(screen.getByLabelText("Default department")).getByRole("option", {
                name: "Financeiro (inactive)",
            }),
        ).toBeInTheDocument();
        expect(screen.getByRole("region", { name: "Agent departments" })).toBeInTheDocument();
        expect(screen.getByRole("region", { name: "Transfer reasons" })).toBeInTheDocument();
        expect(screen.getByLabelText("Reason 1")).toHaveValue("cliente pede segunda via");
        expect(screen.getByRole("button", { name: "Add reason" })).toBeEnabled();
        expect(screen.getByRole("button", { name: "Save routing" })).toBeDisabled();
        expect(screen.getByRole("button", { name: "Discard changes" })).toBeDisabled();
        expect(screen.getByText("Saved (version 1)")).toBeInTheDocument();
    });

    it("a troca para inglês reescreve a tela aberta sem recarregar", async () => {
        serverWith();
        build();
        await screen.findByLabelText("Departamento padrão");

        expect(screen.getByRole("region", { name: "Departamentos do agente" })).toBeInTheDocument();

        act(() => setLocale("en-US"));

        expect(screen.getByLabelText("Default department")).toHaveValue("dep_1");
        expect(screen.getByRole("region", { name: "Agent departments" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Save routing" })).toBeInTheDocument();
        expect(screen.getByText("Saved (version 1)")).toBeInTheDocument();
    });

    it("a validação local responde em inglês e não chama a API", async () => {
        const user = userEvent.setup();
        setLocale("en-US");
        const servidor = serverWith();
        build();
        await screen.findByLabelText("Default department");

        await user.click(screen.getByRole("switch", { name: /Transfer to a human agent/ }));
        await user.click(screen.getByRole("button", { name: "Save routing" }));

        expect(
            await screen.findByText(/choose the human destination department to turn on the transfer/),
        ).toBeInTheDocument();
        expect(servidor.callsFor("PATCH", `/agents/${AGENTE}`)).toHaveLength(0);
    });

    it("o aviso de conflito e a barra de status falam inglês depois de gravar", async () => {
        const user = userEvent.setup();
        setLocale("en-US");
        const servidor = serverWith({ patches: [{ status: 200, corpo: agentJson({ version: 2 }) }] });
        build();
        await screen.findByLabelText("Default department");

        await user.selectOptions(screen.getByLabelText("Default department"), "dep_2");
        await user.click(screen.getByRole("button", { name: "Save routing" }));

        expect(await screen.findByRole("status")).toHaveTextContent("Routing saved (version 2).");
        expect(servidor.callsFor("PATCH", `/agents/${AGENTE}`)).toHaveLength(1);
    });
});
