
import { afterEach, describe, expect, it } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Link, MemoryRouter, Route, Routes } from "react-router-dom";
import { GuardChangesProvider } from "../components/GuardChanges";
import { waitForModalFocus } from "../testing/modal";
import { FakeServer, apiError } from "../testing/server";
import { AgentInstructions } from "./AgentInstructions";
import { setLocale } from "../i18n";

const ORG = "org_1";
const AGENTE = "ag_1";

function agente(partial: Record<string, unknown> = {}) {
    return {
        id: AGENTE,
        org_id: ORG,
        name: "Beatriz",
        slug: "beatriz",
        description: "Atendente da clínica",
        status: "active",
        default_department_id: null,
        human_handoff_department_id: null,
        version: 1,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
        ...partial,
    };
}

const STRUCTURED = {
    identity: {
        name: "Beatriz",
        role: "Atendente",
        company: "Exemplo Odontologia",
        company_description: "Clínica odontológica em São Paulo.",
    },
    principles: [
        { rule: "nunca informar preço fora do catálogo", fallback_action: "say_dont_know" },
    ],
    scope: {
        does: "agenda consultas e tira dúvidas de horário",
        does_not: "fala de preço ou de convênio",
        handoff: { department_id: "", reasons: [] },
    },
    flow_steps: [],
    tone: {
        language: "pt-BR",
        style: "professional",
        emoji_policy: "moderate",
        format: "simple_lists",
        extra_restrictions: "",
    },
    tools: [
        { name: "consultar_base", enabled: true, when_to_use: "dúvida de horário" },
        { name: "buscar_produto", enabled: false, when_to_use: "" },
    ],
    extra_instructions: "",
};

function instructions(partial: Record<string, unknown> = {}) {
    return {
        agent_id: AGENTE,
        org_id: ORG,
        structured: STRUCTURED,
        markdown:
            "# Beatriz, Atendente da Exemplo Odontologia\n\n## Principios inegociaveis\n\n1. **nunca informar preço fora do catálogo**",
        derived_from_json: true,
        version: 2,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
        ...partial,
    };
}

function Shell() {
    return (
        <div>
            <Link to={`/orgs/${ORG}/agents`}>Agentes</Link>
            <AgentInstructions />
        </div>
    );
}

function build() {
    return render(
        <MemoryRouter initialEntries={[`/orgs/${ORG}/agents/${AGENTE}/instructions`]}>
            <GuardChangesProvider>
                <Routes>
                    <Route path="/orgs/:orgId/agents" element={<div>Tela de agentes</div>} />
                    <Route
                        path="/orgs/:orgId/agents/:agentId/instructions"
                        element={<Shell />}
                    />
                </Routes>
            </GuardChangesProvider>
        </MemoryRouter>,
    );
}

function serverWith(instructionsCurrent: unknown = instructions()) {
    return new FakeServer()
        .respond("GET", `/agents/${AGENTE}/instructions/current`, {
            status: 200,
            corpo: instructionsCurrent,
        })
        .respond("GET", `/agents/${AGENTE}`, { status: 200, corpo: agente() })
        .respond("GET", `/admin/v1/orgs/${ORG}`, {
            status: 200,
            corpo: { id: ORG, name: "Exemplo Odontologia" },
        })
        .install();
}

describe("AgenteInstrucoes", () => {
    it("carrega o que está salvo e mostra a prévia composta pelo servidor", async () => {
        serverWith();
        build();

        expect(await screen.findByDisplayValue("Beatriz")).toBeInTheDocument();
        expect(screen.getByText(/If-Match: v2/)).toBeInTheDocument();
        expect(screen.getByText(/Beatriz, Atendente da Exemplo Odontologia/)).toBeInTheDocument();
        expect(screen.getByText(/Principios inegociaveis/)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Salvar (v2)" })).toBeDisabled();
    });

    it("salva com o If-Match da versão lida e atualiza a prévia com o texto recomposto", async () => {
        const user = userEvent.setup();
        const servidor = new FakeServer()
            .respond("GET", `/agents/${AGENTE}/instructions/current`, {
                status: 200,
                corpo: instructions(),
            })
            .respond("GET", `/agents/${AGENTE}`, { status: 200, corpo: agente() })
            .respond("GET", `/admin/v1/orgs/${ORG}`, {
                status: 200,
                corpo: { id: ORG, name: "Exemplo Odontologia" },
            })
            .respond("PATCH", `/agents/${AGENTE}/instructions/current`, {
                status: 200,
                corpo: instructions({
                    version: 3,
                    structured: { ...STRUCTURED, identity: { ...STRUCTURED.identity, name: "Beatriz Lima" } },
                    markdown: "# Beatriz Lima, Atendente da Exemplo Odontologia",
                }),
            })
            .install();
        build();

        const campo = await screen.findByDisplayValue("Beatriz");
        await user.clear(campo);
        await user.type(campo, "Beatriz Lima");
        await user.click(screen.getByRole("button", { name: "Salvar (v2)" }));

        await waitFor(() => expect(screen.getByText(/If-Match: v3/)).toBeInTheDocument());
        const [order] = servidor.callsFor("PATCH", `/agents/${AGENTE}/instructions/current`);
        expect(order?.headers["if-match"]).toBe("2");
        const corpo = order?.corpo as { structured: typeof STRUCTURED };
        expect(corpo.structured.identity.name).toBe("Beatriz Lima");
        expect(screen.getByText(/# Beatriz Lima/)).toBeInTheDocument();
    });

    it("no 412 relê as instruções, adota a versão nova e mantém o que foi digitado", async () => {
        const user = userEvent.setup();
        let versionCurrent = 2;
        new FakeServer()
            .respond("GET", `/agents/${AGENTE}/instructions/current`, () => ({
                status: 200,
                corpo:
                    versionCurrent === 2
                        ? instructions()
                        : instructions({
                              version: 3,
                              markdown: "# Beatriz (salva por outra pessoa)",
                          }),
            }))
            .respond("GET", `/agents/${AGENTE}`, { status: 200, corpo: agente() })
            .respond("GET", `/admin/v1/orgs/${ORG}`, {
                status: 200,
                corpo: { id: ORG, name: "Exemplo Odontologia" },
            })
            .respond("PATCH", `/agents/${AGENTE}/instructions/current`, () => {
                versionCurrent = 3;
                return apiError(412, "version_conflict", "versão desatualizada", {
                    expected_version: 2,
                    current_version: 3,
                });
            })
            .install();
        build();

        const campo = await screen.findByDisplayValue("Beatriz");
        await user.clear(campo);
        await user.type(campo, "Beatriz Lima");
        await user.click(screen.getByRole("button", { name: "Salvar (v2)" }));

        expect(
            await screen.findByText(/Outra pessoa salvou as instruções antes de você \(agora na versão 3\)/),
        ).toBeInTheDocument();
        expect(screen.getByDisplayValue("Beatriz Lima")).toBeInTheDocument();
        expect(screen.getByText(/If-Match: v3/)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Salvar (v3)" })).toBeInTheDocument();
    });

    it("nome vazio não chama a API e o erro volta no campo", async () => {
        const user = userEvent.setup();
        const servidor = serverWith();
        build();

        const campo = await screen.findByDisplayValue("Beatriz");
        await user.clear(campo);
        await user.click(screen.getByRole("button", { name: "Salvar (v2)" }));

        expect(await screen.findByText("informe o nome do assistente")).toBeInTheDocument();
        expect(servidor.callsFor("PATCH", "/instructions/current")).toHaveLength(0);
    });

    it("cria as instruções quando o agente ainda não tem nenhuma", async () => {
        const user = userEvent.setup();
        const servidor = new FakeServer()
            .respond(
                "GET",
                `/agents/${AGENTE}/instructions/current`,
                apiError(404, "not_found", "instruções não encontradas"),
            )
            .respond("GET", `/agents/${AGENTE}`, { status: 200, corpo: agente() })
            .respond("GET", `/admin/v1/orgs/${ORG}`, {
                status: 200,
                corpo: { id: ORG, name: "Exemplo Odontologia" },
            })
            .respond("POST", `/agents/${AGENTE}/instructions`, {
                status: 201,
                corpo: instructions({ version: 1, markdown: "# Beatriz, Atendente" }),
            })
            .install();
        build();

        expect(await screen.findByDisplayValue("Beatriz")).toBeInTheDocument();
        expect(screen.getByText(/sem instruções salvas/)).toBeInTheDocument();
        await user.click(screen.getByRole("button", { name: "Salvar instruções" }));

        await waitFor(() => expect(screen.getByText(/If-Match: v1/)).toBeInTheDocument());
        const [order] = servidor.callsFor("POST", `/agents/${AGENTE}/instructions`);
        expect(order?.headers["if-match"]).toBeUndefined();
        expect((order?.corpo as { structured: typeof STRUCTURED }).structured.identity.name).toBe(
            "Beatriz",
        );
    });

    it("erro de validação da API volta na seção de ferramentas, com o motivo real", async () => {
        const user = userEvent.setup();
        const servidor = new FakeServer()
            .respond("GET", `/agents/${AGENTE}/instructions/current`, {
                status: 200,
                corpo: instructions(),
            })
            .respond("GET", `/agents/${AGENTE}`, { status: 200, corpo: agente() })
            .respond("GET", `/admin/v1/orgs/${ORG}`, {
                status: 200,
                corpo: { id: ORG, name: "Exemplo Odontologia" },
            })
            .respond(
                "PATCH",
                `/agents/${AGENTE}/instructions/current`,
                apiError(400, "validation_error", "campo `structured` inválido", {
                    field: "structured",
                    reason: "`search_knowledge_base` não é ferramenta da V1",
                }),
            )
            .install();
        build();

        const campo = await screen.findByDisplayValue("Beatriz");
        await user.clear(campo);
        await user.type(campo, "Beatriz Lima");
        await user.click(screen.getByRole("button", { name: "Salvar (v2)" }));

        expect(
            await screen.findByText(/`search_knowledge_base` não é ferramenta da V1/),
        ).toBeInTheDocument();
        expect(servidor.callsFor("PATCH", "/instructions/current")).toHaveLength(1);
    });

    it("avisa alterações não salvas ao trocar de tela e deixa cancelar ou descartar", async () => {
        const user = userEvent.setup();
        serverWith();
        build();

        const campo = await screen.findByDisplayValue("Beatriz");
        await user.clear(campo);
        await user.type(campo, "Beatriz Lima");

        await user.click(screen.getByRole("link", { name: "Agentes" }));
        const dialog = await screen.findByRole("dialog", { name: "Alterações não salvas" });
        expect(dialog).toBeInTheDocument();
        expect(screen.queryByText("Tela de agentes")).toBeNull();

        await waitForModalFocus();
        await user.click(within(dialog).getByRole("button", { name: "Cancelar" }));
        expect(screen.queryByRole("dialog", { name: "Alterações não salvas" })).toBeNull();
        expect(screen.getByDisplayValue("Beatriz Lima")).toBeInTheDocument();

        await user.click(screen.getByRole("link", { name: "Agentes" }));
        const second = await screen.findByRole("dialog", { name: "Alterações não salvas" });
        await waitForModalFocus();
        await user.click(within(second).getByRole("button", { name: "Descartar alterações" }));

        expect(await screen.findByText("Tela de agentes")).toBeInTheDocument();
    });

    it("usa o cabeçalho comum do agente (nome, situação, slug, organização) e seções planas, sem cartão", async () => {
        serverWith();
        const { container } = build();

        expect(await screen.findByRole("heading", { level: 1, name: "Beatriz" })).toBeInTheDocument();
        expect(await screen.findByText("Agente · Exemplo Odontologia")).toBeInTheDocument();
        expect(screen.getByText("beatriz")).toBeInTheDocument();
        expect(screen.getByText("Ativo")).toBeInTheDocument();
        expect(screen.getByRole("region", { name: "Identidade" })).toBeInTheDocument();
        expect(screen.getByRole("region", { name: "Prévia do texto composto" })).toBeInTheDocument();
        expect(container.querySelector(".sofa-card")).toBeNull();
    });
});

describe("I18N-07: aba de Instruções em en-US", () => {
    afterEach(() => {
        act(() => setLocale("pt-BR"));
    });

    it("o formulário, a prévia e as ações falam inglês", async () => {
        setLocale("en-US");
        serverWith();
        build();

        expect(await screen.findByDisplayValue("Beatriz")).toBeInTheDocument();
        expect(screen.getByLabelText(/Assistant name/)).toHaveValue("Beatriz");
        expect(screen.getByLabelText("Role or function")).toHaveValue("Atendente");
        expect(screen.getByRole("region", { name: "Identity" })).toBeInTheDocument();
        expect(
            screen.getByRole("region", { name: "Principles and non-negotiable rules" }),
        ).toBeInTheDocument();
        expect(screen.getByLabelText("Rule 1")).toBeInTheDocument();
        expect(screen.getByLabelText("When it cannot comply")).toBeInTheDocument();
        expect(screen.getByLabelText("Language")).toHaveValue("pt-BR");
        expect(screen.getByRole("option", { name: "Portuguese (Brazil)" })).toBeInTheDocument();
        expect(screen.getByRole("option", { name: "Friendly" })).toBeInTheDocument();
        expect(screen.getByRole("switch", { name: /Search the knowledge base/ })).toBeChecked();
        expect(screen.getByRole("region", { name: "Composed text preview" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Save (v2)" })).toBeDisabled();
        expect(screen.getByRole("button", { name: "Refresh preview" })).toBeInTheDocument();
        expect(screen.getByText(/If-Match: v2/)).toBeInTheDocument();
    });

    it("a troca para inglês reescreve a tela aberta sem recarregar", async () => {
        serverWith();
        build();
        await screen.findByDisplayValue("Beatriz");

        expect(screen.getByLabelText(/Nome do assistente/)).toBeInTheDocument();
        expect(screen.getByRole("region", { name: "Identidade" })).toBeInTheDocument();

        act(() => setLocale("en-US"));

        expect(screen.getByLabelText(/Assistant name/)).toHaveValue("Beatriz");
        expect(screen.getByRole("region", { name: "Identity" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Save (v2)" })).toBeInTheDocument();
        expect(screen.getByRole("region", { name: "Composed text preview" })).toBeInTheDocument();
    });

    it("a validação local responde em inglês e não chama a API", async () => {
        const user = userEvent.setup();
        setLocale("en-US");
        const servidor = serverWith();
        build();

        const campo = await screen.findByDisplayValue("Beatriz");
        await user.clear(campo);
        await user.click(screen.getByRole("button", { name: "Save (v2)" }));

        expect(await screen.findByText("enter the assistant name")).toBeInTheDocument();
        expect(servidor.callsFor("PATCH", "/instructions/current")).toHaveLength(0);
    });
});
