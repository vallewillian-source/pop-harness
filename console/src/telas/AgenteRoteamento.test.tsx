/**
 * Testes da aba Roteamento do agente (HR-43).
 *
 * O que importa: carregar o roteamento como o motor o lê, gravar só o que mudou com `If-Match` da
 * versão do agente, preservar as outras chaves do `jev_json` ao mudar os motivos, o `412` que
 * relê o agente e **mantém o que foi digitado**, as validações locais (sem chamar a API) e o erro
 * da API mostrado com código e motivo reais.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { GuardaAlteracoesProvider } from "../componentes/GuardaAlteracoes";
import { ServidorFalso, erroApi } from "../testes/servidor";
import { AgenteRoteamento } from "./AgenteRoteamento";
import { setLocale } from "../i18n";

const ORG = "org_1";
const AGENTE = "ag_1";

const DEPARTAMENTOS = [
    { id: "dep_1", org_id: ORG, name: "Atendimento geral", description: "", status: "active", version: 1, created_at: 1, updated_at: 1 },
    { id: "dep_2", org_id: ORG, name: "Suporte clínico", description: "", status: "active", version: 1, created_at: 1, updated_at: 1 },
    { id: "dep_3", org_id: ORG, name: "Financeiro", description: "", status: "inactive", version: 1, created_at: 1, updated_at: 1 },
];

function agenteJson(parcial: Record<string, unknown> = {}) {
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
        ...parcial,
    };
}

function montar() {
    return render(
        <MemoryRouter initialEntries={[`/orgs/${ORG}/agents/${AGENTE}/routing`]}>
            <GuardaAlteracoesProvider>
                <Routes>
                    <Route path="/orgs/:orgId/agents/:agentId/routing" element={<AgenteRoteamento />} />
                    <Route path="/orgs/:orgId/structure" element={<div>Tela de estrutura</div>} />
                    <Route path="/orgs/:orgId/agents" element={<div>Tela de agentes</div>} />
                </Routes>
            </GuardaAlteracoesProvider>
        </MemoryRouter>,
    );
}

/**
 * Servidor falso. A ordem importa (o casamento é pelo primeiro trecho que bate): departamentos
 * antes da rota da org, e a rota do agente por último.
 */
function servidorCom(opcoes: {
    agente?: Record<string, unknown>;
    /** Respostas do `PATCH` do agente, na ordem das chamadas. */
    patches?: Array<{ status: number; corpo?: unknown }>;
    /** Agente relido depois de um 412. */
    relido?: Record<string, unknown>;
} = {}) {
    let leituras = 0;
    let patches = 0;
    const agente = opcoes.agente ?? agenteJson();
    const servidor = new ServidorFalso()
        .responder("GET", `/admin/v1/orgs/${ORG}/departments`, {
            status: 200,
            corpo: { items: DEPARTAMENTOS, next_cursor: null },
        })
        .responder("PATCH", `/agents/${AGENTE}`, () => {
            const resposta = opcoes.patches?.[patches] ?? { status: 200, corpo: agenteJson({ version: 2 }) };
            patches += 1;
            return resposta;
        })
        .responder("GET", `/agents/${AGENTE}`, () => {
            leituras += 1;
            if (leituras > 1 && opcoes.relido) return { status: 200, corpo: opcoes.relido };
            return { status: 200, corpo: agente };
        })
        .responder("GET", `/admin/v1/orgs/${ORG}`, {
            status: 200,
            corpo: { id: ORG, name: "Clínica Teste", status: "active", version: 1 },
        });
    return servidor.instalar();
}

describe("Aba Roteamento", () => {
    it("carrega o departamento padrão, a transferência e os motivos do agente", async () => {
        const servidor = servidorCom();
        montar();

        const padrao = await screen.findByLabelText("Departamento padrão");
        expect(padrao).toHaveValue("dep_1");
        expect(screen.getByRole("switch", { name: /Transferir para atendente humano/ })).not.toBeChecked();
        expect(screen.getByLabelText("Motivo 1")).toHaveValue("cliente pede segunda via");
        expect(servidor.chamadasPara("GET", "/departments")[0]?.url).toContain("limit=200");
    });

    it("grava só o departamento padrão e a transferência, com If-Match da versão do agente", async () => {
        const servidor = servidorCom();
        montar();
        await screen.findByLabelText("Departamento padrão");
        const usuario = userEvent.setup();

        await usuario.selectOptions(screen.getByLabelText("Departamento padrão"), "dep_2");
        await usuario.click(screen.getByRole("switch", { name: /Transferir para atendente humano/ }));
        await usuario.selectOptions(screen.getByLabelText("Departamento humano de destino"), "dep_2");
        await usuario.click(screen.getByRole("button", { name: "Salvar roteamento" }));

        await waitFor(() => expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)).toHaveLength(1));
        const envio = servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)[0]!;
        expect(envio.cabecalhos["if-match"]).toBe("1");
        expect(envio.corpo).toEqual({ department: "dep_2", handoff: true, handoff_department: "dep_2" });
        expect(await screen.findByRole("status")).toHaveTextContent("Roteamento gravado (versão 2)");
    });

    it("ao mudar os motivos, manda o jev_json com as outras chaves do agente preservadas", async () => {
        const servidor = servidorCom();
        montar();
        await screen.findByLabelText("Motivo 1");
        const usuario = userEvent.setup();

        await usuario.click(screen.getByRole("button", { name: "Adicionar motivo" }));
        await usuario.type(screen.getByLabelText("Motivo 2"), "reclamação de atraso");
        await usuario.click(screen.getByRole("button", { name: "Salvar roteamento" }));

        await waitFor(() => expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)).toHaveLength(1));
        const corpo = servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)[0]!.corpo as { jev_json: string };
        expect(JSON.parse(corpo.jev_json)).toEqual({
            despedida: "Até logo!",
            situacoes_de_handoff: ["cliente pede segunda via", "reclamação de atraso"],
        });
        expect(Object.keys(corpo)).toEqual(["jev_json"]);
    });

    it("no 412 relê o agente, mantém o que foi digitado e grava de novo na versão atual", async () => {
        const servidor = servidorCom({
            relido: agenteJson({ version: 2, default_department_id: "dep_1", jev: { despedida: "Até logo!", situacoes_de_handoff: ["cliente pede segunda via"] } }),
            patches: [
                { status: 412, corpo: { code: "version_conflict", message: "agente mudou desde a leitura", details: { current_version: 2, expected_version: 1 } } },
                { status: 200, corpo: agenteJson({ version: 3, default_department_id: "dep_2" }) },
            ],
        });
        montar();
        await screen.findByLabelText("Departamento padrão");
        const usuario = userEvent.setup();

        await usuario.selectOptions(screen.getByLabelText("Departamento padrão"), "dep_2");
        await usuario.click(screen.getByRole("button", { name: "Salvar roteamento" }));

        expect(
            await screen.findByText(/Outra pessoa salvou este agente antes de você \(agora na versão 2\)/),
        ).toBeInTheDocument();
        // O que foi digitado continua na tela.
        expect(screen.getByLabelText("Departamento padrão")).toHaveValue("dep_2");

        await usuario.click(screen.getByRole("button", { name: "Salvar roteamento" }));
        await waitFor(() => expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)).toHaveLength(2));
        expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)[1]!.cabecalhos["if-match"]).toBe("2");
        expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)[1]!.corpo).toEqual({ department: "dep_2" });
    });

    it("exige o departamento humano para ligar a transferência, sem chamar a API", async () => {
        const servidor = servidorCom();
        montar();
        await screen.findByLabelText("Departamento padrão");
        const usuario = userEvent.setup();

        await usuario.click(screen.getByRole("switch", { name: /Transferir para atendente humano/ }));
        await usuario.click(screen.getByRole("button", { name: "Salvar roteamento" }));

        expect(await screen.findByText(/escolha o departamento humano de destino/)).toBeInTheDocument();
        expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)).toHaveLength(0);
    });

    it("não grava motivo acima de 300 caracteres nem deixa passar de 15 motivos", async () => {
        const motivos = Array.from({ length: 15 }, (_, i) => `motivo ${i + 1}`);
        const servidor = servidorCom({ agente: agenteJson({ jev: { situacoes_de_handoff: motivos } }) });
        montar();
        await screen.findByLabelText("Motivo 15");
        const usuario = userEvent.setup();

        // Com 15 motivos, a lista não aceita mais um.
        expect(screen.getByRole("button", { name: "Adicionar motivo" })).toBeDisabled();

        // Um motivo de 301 caracteres fica marcado no campo e a gravação não sai.
        fireEvent.change(screen.getByLabelText("Motivo 1"), { target: { value: "a".repeat(301) } });
        await usuario.click(screen.getByRole("button", { name: "Salvar roteamento" }));

        expect(await screen.findByText("o motivo aceita até 300 caracteres")).toBeInTheDocument();
        expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)).toHaveLength(0);
    });

    it("mostra o código e o motivo reais quando a API recusa a gravação", async () => {
        const servidor = servidorCom({
            patches: [
                erroApi(404, "not_found", "departamento `dep_2` não encontrado", null),
            ],
        });
        montar();
        await screen.findByLabelText("Departamento padrão");
        const usuario = userEvent.setup();

        await usuario.selectOptions(screen.getByLabelText("Departamento padrão"), "dep_2");
        await usuario.click(screen.getByRole("button", { name: "Salvar roteamento" }));

        expect(await screen.findByText(/departamento `dep_2` não encontrado \(código not_found\)/)).toBeInTheDocument();
        expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)).toHaveLength(1);
    });

    it("sem alteração, o botão de gravar fica desabilitado", async () => {
        servidorCom();
        montar();
        await screen.findByLabelText("Departamento padrão");

        expect(screen.getByRole("button", { name: "Salvar roteamento" })).toBeDisabled();
        expect(screen.getByText(/Salvo \(versão 1\)/)).toBeInTheDocument();
    });

    it("sem departamentos na org, mostra o estado vazio com caminho para Estrutura", async () => {
        const servidor = new ServidorFalso()
            .responder("GET", `/admin/v1/orgs/${ORG}/departments`, { status: 200, corpo: { items: [], next_cursor: null } })
            .responder("GET", `/agents/${AGENTE}`, { status: 200, corpo: agenteJson() })
            .responder("GET", `/admin/v1/orgs/${ORG}`, { status: 200, corpo: { id: ORG, name: "Clínica", status: "active", version: 1 } });
        servidor.instalar();
        montar();

        expect(await screen.findByText("Nenhum departamento na organização")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Ir para Estrutura" })).toBeEnabled();
    });

    it("usa o cabeçalho comum do agente e seções planas, sem cartão", async () => {
        servidorCom();
        const { container } = montar();

        expect(await screen.findByRole("heading", { level: 1, name: "Beatriz" })).toBeInTheDocument();
        expect(screen.getByRole("region", { name: "Departamentos do agente" })).toBeInTheDocument();
        expect(screen.getByRole("region", { name: "Motivos de transferência" })).toBeInTheDocument();
        expect(container.querySelector(".sofa-card")).toBeNull();
    });

    it("a barra de salvar usa o ponto de status do re-layout nos dois estados (RL-20)", async () => {
        servidorCom();
        const { container } = montar();
        await screen.findByLabelText("Departamento padrão");

        const barra = container.querySelector(".poph-roteamento__barra");
        expect(barra).not.toBeNull();
        // A pílula da biblioteca não é mais usada nesta barra.
        expect(barra!.querySelector(".sofa-status-indicator")).toBeNull();

        const salvo = barra!.querySelector(".poph-status");
        expect(salvo).toHaveClass("poph-status--inativo");
        expect(salvo!.querySelector(".poph-status__ponto")).not.toBeNull();
        expect(salvo).toHaveTextContent("Salvo (versão 1)");

        const usuario = userEvent.setup();
        await usuario.selectOptions(screen.getByLabelText("Departamento padrão"), "dep_2");

        const sujo = container.querySelector(".poph-roteamento__barra .poph-status");
        expect(sujo).toHaveClass("poph-status--atencao");
        expect(sujo!.querySelector(".poph-status__ponto")).not.toBeNull();
        expect(sujo).toHaveTextContent("Alterações não salvas");
    });
});

describe("I18N-07: aba de Roteamento em en-US", () => {
    afterEach(() => {
        act(() => setLocale("pt-BR"));
    });

    it("os campos, as seções e a barra de salvar falam inglês", async () => {
        setLocale("en-US");
        servidorCom();
        montar();

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
        servidorCom();
        montar();
        await screen.findByLabelText("Departamento padrão");

        expect(screen.getByRole("region", { name: "Departamentos do agente" })).toBeInTheDocument();

        act(() => setLocale("en-US"));

        expect(screen.getByLabelText("Default department")).toHaveValue("dep_1");
        expect(screen.getByRole("region", { name: "Agent departments" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Save routing" })).toBeInTheDocument();
        expect(screen.getByText("Saved (version 1)")).toBeInTheDocument();
    });

    it("a validação local responde em inglês e não chama a API", async () => {
        const usuario = userEvent.setup();
        setLocale("en-US");
        const servidor = servidorCom();
        montar();
        await screen.findByLabelText("Default department");

        await usuario.click(screen.getByRole("switch", { name: /Transfer to a human agent/ }));
        await usuario.click(screen.getByRole("button", { name: "Save routing" }));

        expect(
            await screen.findByText(/choose the human destination department to turn on the transfer/),
        ).toBeInTheDocument();
        expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)).toHaveLength(0);
    });

    it("o aviso de conflito e a barra de status falam inglês depois de gravar", async () => {
        const usuario = userEvent.setup();
        setLocale("en-US");
        const servidor = servidorCom({ patches: [{ status: 200, corpo: agenteJson({ version: 2 }) }] });
        montar();
        await screen.findByLabelText("Default department");

        await usuario.selectOptions(screen.getByLabelText("Default department"), "dep_2");
        await usuario.click(screen.getByRole("button", { name: "Save routing" }));

        expect(await screen.findByRole("status")).toHaveTextContent("Routing saved (version 2).");
        expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)).toHaveLength(1);
    });
});
