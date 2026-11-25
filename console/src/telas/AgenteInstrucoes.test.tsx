/**
 * Testes da aba Instruções do agente (HR-40).
 *
 * O que importa aqui: carregar o que está salvo, salvar com `If-Match` da versão lida, o `412` de
 * concorrência **sem perder o que o operador digitou**, a criação quando o agente ainda não tem
 * instruções, a validação local sem chamar a API, o erro de validação que a API devolve e o aviso
 * de alterações não salvas ao trocar de tela.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Link, MemoryRouter, Route, Routes } from "react-router-dom";
import { GuardaAlteracoesProvider } from "../componentes/GuardaAlteracoes";
import { esperarFocoDoModal } from "../testes/modal";
import { ServidorFalso, erroApi } from "../testes/servidor";
import { AgenteInstrucoes } from "./AgenteInstrucoes";
import { setLocale } from "../i18n";

const ORG = "org_1";
const AGENTE = "ag_1";

function agente(parcial: Record<string, unknown> = {}) {
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
        ...parcial,
    };
}

const ESTRUTURADO = {
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

function instrucoes(parcial: Record<string, unknown> = {}) {
    return {
        agent_id: AGENTE,
        org_id: ORG,
        structured: ESTRUTURADO,
        markdown:
            "# Beatriz, Atendente da Exemplo Odontologia\n\n## Principios inegociaveis\n\n1. **nunca informar preço fora do catálogo**",
        derived_from_json: true,
        version: 2,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
        ...parcial,
    };
}

/** Casca mínima com o link que a guarda precisa interceptar (o rail, no app de verdade). */
function Casca() {
    return (
        <div>
            <Link to={`/orgs/${ORG}/agents`}>Agentes</Link>
            <AgenteInstrucoes />
        </div>
    );
}

function montar() {
    return render(
        <MemoryRouter initialEntries={[`/orgs/${ORG}/agents/${AGENTE}/instructions`]}>
            <GuardaAlteracoesProvider>
                <Routes>
                    <Route path="/orgs/:orgId/agents" element={<div>Tela de agentes</div>} />
                    <Route
                        path="/orgs/:orgId/agents/:agentId/instructions"
                        element={<Casca />}
                    />
                </Routes>
            </GuardaAlteracoesProvider>
        </MemoryRouter>,
    );
}

/** Servidor com as rotas da tela, na ordem em que o `ServidorFalso` casa (mais específica antes). */
function servidorCom(instrucoesAtuais: unknown = instrucoes()) {
    return new ServidorFalso()
        .responder("GET", `/agents/${AGENTE}/instructions/current`, {
            status: 200,
            corpo: instrucoesAtuais,
        })
        .responder("GET", `/agents/${AGENTE}`, { status: 200, corpo: agente() })
        .responder("GET", `/admin/v1/orgs/${ORG}`, {
            status: 200,
            corpo: { id: ORG, name: "Exemplo Odontologia" },
        })
        .instalar();
}

describe("AgenteInstrucoes", () => {
    it("carrega o que está salvo e mostra a prévia composta pelo servidor", async () => {
        servidorCom();
        montar();

        expect(await screen.findByDisplayValue("Beatriz")).toBeInTheDocument();
        expect(screen.getByText(/If-Match: v2/)).toBeInTheDocument();
        expect(screen.getByText(/Beatriz, Atendente da Exemplo Odontologia/)).toBeInTheDocument();
        // A prévia é a do servidor: o texto salvo aparece inteiro, não um rascunho remontado.
        expect(screen.getByText(/Principios inegociaveis/)).toBeInTheDocument();
        // O botão de salvar só habilita com edição pendente.
        expect(screen.getByRole("button", { name: "Salvar (v2)" })).toBeDisabled();
    });

    it("salva com o If-Match da versão lida e atualiza a prévia com o texto recomposto", async () => {
        const usuario = userEvent.setup();
        const servidor = new ServidorFalso()
            .responder("GET", `/agents/${AGENTE}/instructions/current`, {
                status: 200,
                corpo: instrucoes(),
            })
            .responder("GET", `/agents/${AGENTE}`, { status: 200, corpo: agente() })
            .responder("GET", `/admin/v1/orgs/${ORG}`, {
                status: 200,
                corpo: { id: ORG, name: "Exemplo Odontologia" },
            })
            .responder("PATCH", `/agents/${AGENTE}/instructions/current`, {
                status: 200,
                corpo: instrucoes({
                    version: 3,
                    structured: { ...ESTRUTURADO, identity: { ...ESTRUTURADO.identity, name: "Beatriz Lima" } },
                    markdown: "# Beatriz Lima, Atendente da Exemplo Odontologia",
                }),
            })
            .instalar();
        montar();

        const campo = await screen.findByDisplayValue("Beatriz");
        await usuario.clear(campo);
        await usuario.type(campo, "Beatriz Lima");
        await usuario.click(screen.getByRole("button", { name: "Salvar (v2)" }));

        await waitFor(() => expect(screen.getByText(/If-Match: v3/)).toBeInTheDocument());
        const [pedido] = servidor.chamadasPara("PATCH", `/agents/${AGENTE}/instructions/current`);
        expect(pedido?.cabecalhos["if-match"]).toBe("2");
        const corpo = pedido?.corpo as { structured: typeof ESTRUTURADO };
        expect(corpo.structured.identity.name).toBe("Beatriz Lima");
        // O servidor devolveu o markdown novo; a tela mostra o dele, não remonta o texto.
        expect(screen.getByText(/# Beatriz Lima/)).toBeInTheDocument();
    });

    it("no 412 relê as instruções, adota a versão nova e mantém o que foi digitado", async () => {
        const usuario = userEvent.setup();
        // A releitura depois do 412 devolve a versão 3, como se outra pessoa tivesse salvado.
        let versaoAtual = 2;
        new ServidorFalso()
            .responder("GET", `/agents/${AGENTE}/instructions/current`, () => ({
                status: 200,
                corpo:
                    versaoAtual === 2
                        ? instrucoes()
                        : instrucoes({
                              version: 3,
                              markdown: "# Beatriz (salva por outra pessoa)",
                          }),
            }))
            .responder("GET", `/agents/${AGENTE}`, { status: 200, corpo: agente() })
            .responder("GET", `/admin/v1/orgs/${ORG}`, {
                status: 200,
                corpo: { id: ORG, name: "Exemplo Odontologia" },
            })
            .responder("PATCH", `/agents/${AGENTE}/instructions/current`, () => {
                versaoAtual = 3;
                return erroApi(412, "version_conflict", "versão desatualizada", {
                    expected_version: 2,
                    current_version: 3,
                });
            })
            .instalar();
        montar();

        const campo = await screen.findByDisplayValue("Beatriz");
        await usuario.clear(campo);
        await usuario.type(campo, "Beatriz Lima");
        await usuario.click(screen.getByRole("button", { name: "Salvar (v2)" }));

        expect(
            await screen.findByText(/Outra pessoa salvou as instruções antes de você \(agora na versão 3\)/),
        ).toBeInTheDocument();
        // O rascunho não é descartado...
        expect(screen.getByDisplayValue("Beatriz Lima")).toBeInTheDocument();
        // ...e a tela passa a salvar sobre a versão atual.
        expect(screen.getByText(/If-Match: v3/)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Salvar (v3)" })).toBeInTheDocument();
    });

    it("nome vazio não chama a API e o erro volta no campo", async () => {
        const usuario = userEvent.setup();
        const servidor = servidorCom();
        montar();

        const campo = await screen.findByDisplayValue("Beatriz");
        await usuario.clear(campo);
        await usuario.click(screen.getByRole("button", { name: "Salvar (v2)" }));

        expect(await screen.findByText("informe o nome do assistente")).toBeInTheDocument();
        expect(servidor.chamadasPara("PATCH", "/instructions/current")).toHaveLength(0);
    });

    it("cria as instruções quando o agente ainda não tem nenhuma", async () => {
        const usuario = userEvent.setup();
        const servidor = new ServidorFalso()
            .responder(
                "GET",
                `/agents/${AGENTE}/instructions/current`,
                erroApi(404, "not_found", "instruções não encontradas"),
            )
            .responder("GET", `/agents/${AGENTE}`, { status: 200, corpo: agente() })
            .responder("GET", `/admin/v1/orgs/${ORG}`, {
                status: 200,
                corpo: { id: ORG, name: "Exemplo Odontologia" },
            })
            .responder("POST", `/agents/${AGENTE}/instructions`, {
                status: 201,
                corpo: instrucoes({ version: 1, markdown: "# Beatriz, Atendente" }),
            })
            .instalar();
        montar();

        // Sem instruções, o nome do agente já entra no formulário.
        expect(await screen.findByDisplayValue("Beatriz")).toBeInTheDocument();
        expect(screen.getByText(/sem instruções salvas/)).toBeInTheDocument();
        await usuario.click(screen.getByRole("button", { name: "Salvar instruções" }));

        await waitFor(() => expect(screen.getByText(/If-Match: v1/)).toBeInTheDocument());
        const [pedido] = servidor.chamadasPara("POST", `/agents/${AGENTE}/instructions`);
        expect(pedido?.cabecalhos["if-match"]).toBeUndefined();
        expect((pedido?.corpo as { structured: typeof ESTRUTURADO }).structured.identity.name).toBe(
            "Beatriz",
        );
    });

    it("erro de validação da API volta na seção de ferramentas, com o motivo real", async () => {
        const usuario = userEvent.setup();
        const servidor = new ServidorFalso()
            .responder("GET", `/agents/${AGENTE}/instructions/current`, {
                status: 200,
                corpo: instrucoes(),
            })
            .responder("GET", `/agents/${AGENTE}`, { status: 200, corpo: agente() })
            .responder("GET", `/admin/v1/orgs/${ORG}`, {
                status: 200,
                corpo: { id: ORG, name: "Exemplo Odontologia" },
            })
            .responder(
                "PATCH",
                `/agents/${AGENTE}/instructions/current`,
                erroApi(400, "validation_error", "campo `structured` inválido", {
                    field: "structured",
                    reason: "`search_knowledge_base` não é ferramenta da V1",
                }),
            )
            .instalar();
        montar();

        const campo = await screen.findByDisplayValue("Beatriz");
        await usuario.clear(campo);
        await usuario.type(campo, "Beatriz Lima");
        await usuario.click(screen.getByRole("button", { name: "Salvar (v2)" }));

        expect(
            await screen.findByText(/`search_knowledge_base` não é ferramenta da V1/),
        ).toBeInTheDocument();
        expect(servidor.chamadasPara("PATCH", "/instructions/current")).toHaveLength(1);
    });

    it("avisa alterações não salvas ao trocar de tela e deixa cancelar ou descartar", async () => {
        const usuario = userEvent.setup();
        servidorCom();
        montar();

        const campo = await screen.findByDisplayValue("Beatriz");
        await usuario.clear(campo);
        await usuario.type(campo, "Beatriz Lima");

        await usuario.click(screen.getByRole("link", { name: "Agentes" }));
        const dialogo = await screen.findByRole("dialog", { name: "Alterações não salvas" });
        expect(dialogo).toBeInTheDocument();
        // Nada navegou ainda.
        expect(screen.queryByText("Tela de agentes")).toBeNull();

        await esperarFocoDoModal();
        await usuario.click(within(dialogo).getByRole("button", { name: "Cancelar" }));
        expect(screen.queryByRole("dialog", { name: "Alterações não salvas" })).toBeNull();
        expect(screen.getByDisplayValue("Beatriz Lima")).toBeInTheDocument();

        await usuario.click(screen.getByRole("link", { name: "Agentes" }));
        const segundo = await screen.findByRole("dialog", { name: "Alterações não salvas" });
        await esperarFocoDoModal();
        await usuario.click(within(segundo).getByRole("button", { name: "Descartar alterações" }));

        expect(await screen.findByText("Tela de agentes")).toBeInTheDocument();
    });

    it("usa o cabeçalho comum do agente (nome, situação, slug, organização) e seções planas, sem cartão", async () => {
        servidorCom();
        const { container } = montar();

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
        servidorCom();
        montar();

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
        servidorCom();
        montar();
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
        const usuario = userEvent.setup();
        setLocale("en-US");
        const servidor = servidorCom();
        montar();

        const campo = await screen.findByDisplayValue("Beatriz");
        await usuario.clear(campo);
        await usuario.click(screen.getByRole("button", { name: "Save (v2)" }));

        expect(await screen.findByText("enter the assistant name")).toBeInTheDocument();
        expect(servidor.chamadasPara("PATCH", "/instructions/current")).toHaveLength(0);
    });
});
