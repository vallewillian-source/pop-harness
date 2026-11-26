/**
 * Testes da aba Modelo e Jev do agente (HR-45).
 *
 * O que importa: o formulário carrega o que o motor lê; grava só o que mudou com `If-Match`; o
 * `jev_json` sai com as chaves que a tela não edita intactas; validação de faixa não chama a API;
 * `412` relê, mantém o que foi digitado e o próximo salvamento usa a versão atual; erro de campo
 * da API aparece no campo.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { GuardaAlteracoesProvider } from "../componentes/GuardaAlteracoes";
import { I18nProvider, setLocale } from "../i18n";

afterEach(() => {
    setLocale("pt-BR");
});
import { FakeServer, apiError } from "../testing/server";
import { AgenteModeloJev } from "./AgenteModeloJev";

const ORG = "org_1";
const AGENTE = "ag_1";

function agenteJson(parcial: Record<string, unknown> = {}) {
    return {
        id: AGENTE,
        org_id: ORG,
        name: "Beatriz",
        slug: "beatriz",
        description: "",
        status: "active",
        default_department_id: null,
        human_handoff_enabled: false,
        human_handoff_department_id: null,
        provider: "antigravity2api",
        model: "gemini-3.8-flash-tiered",
        temperature: null,
        max_output_tokens: null,
        max_context_tokens: 64000,
        grouping_window_ms: 2000,
        max_tool_rounds: 4,
        max_tool_calls_per_message: 10,
        rag_top_k: 3,
        rag_min_score: 0,
        jev: {
            despedida: "Até logo!",
            situacoes_de_handoff: ["cliente pede segunda via"],
            gates: { g8: false },
        },
        version: 1,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
        ...parcial,
    };
}

function montar() {
    return render(
        <I18nProvider>
            <MemoryRouter initialEntries={[`/orgs/${ORG}/agents/${AGENTE}/model-jev`]}>
                <GuardaAlteracoesProvider>
                    <Routes>
                        <Route path="/orgs/:orgId/agents/:agentId/model-jev" element={<AgenteModeloJev />} />
                        <Route path="/orgs/:orgId/agents" element={<div>Tela de agentes</div>} />
                        <Route path="/orgs/:orgId/agents/:agentId/routing" element={<div>Tela de roteamento</div>} />
                    </Routes>
                </GuardaAlteracoesProvider>
            </MemoryRouter>
        </I18nProvider>,
    );
}

/**
 * Servidor falso. A ordem importa (o casamento é pelo primeiro trecho que bate): a rota do agente
 * vem antes da rota da org, que é o prefixo de todas.
 */
function servidorCom(opcoes: {
    agente?: Record<string, unknown>;
    patches?: Array<{ status: number; corpo?: unknown }>;
    relido?: Record<string, unknown>;
    leituraFalha?: boolean;
} = {}) {
    let leituras = 0;
    let patches = 0;
    const agente = opcoes.agente ?? agenteJson();
    const servidor = new FakeServer()
        .responder("PATCH", `/agents/${AGENTE}`, () => {
            const resposta = opcoes.patches?.[patches] ?? { status: 200, corpo: agenteJson({ version: 2 }) };
            patches += 1;
            return resposta;
        })
        .responder("GET", `/agents/${AGENTE}`, () => {
            leituras += 1;
            if (opcoes.leituraFalha) return apiError(500, "internal", "falha simulada");
            if (leituras > 1 && opcoes.relido) return { status: 200, corpo: opcoes.relido };
            return { status: 200, corpo: agente };
        })
        .responder("GET", `/admin/v1/orgs/${ORG}`, {
            status: 200,
            corpo: { id: ORG, name: "Clínica Teste", status: "active", version: 1 },
        });
    return servidor.instalar();
}

function corpoDoPatch(servidor: FakeServer, indice = 0): Record<string, unknown> {
    return servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)[indice]!.corpo as Record<string, unknown>;
}

describe("Aba Modelo e Jev", () => {
    it("carrega modelo, RAG, gates e mostra o que é herdado do servidor", async () => {
        servidorCom();
        montar();

        expect(await screen.findByLabelText("Provedor")).toHaveValue("antigravity2api");
        expect(screen.getByLabelText("Modelo")).toHaveValue("gemini-3.8-flash-tiered");
        expect(screen.getByLabelText("Temperatura")).toHaveValue("");
        expect(screen.getByLabelText("Temperatura")).toHaveAttribute("placeholder", "herda o padrão do servidor");
        expect(screen.getByLabelText("Máximo de tokens de saída")).toHaveAttribute(
            "placeholder",
            "herda o padrão do servidor",
        );
        expect(screen.getByLabelText("Trechos recuperados (top-k)")).toHaveValue("3");
        expect(screen.getByLabelText("G8 · A conversa foi encerrada pelo cliente?")).toHaveValue("desligado");
        expect(screen.getByLabelText("G3 · Precisa de base, de produto ou de nada?")).toHaveValue("herdar");
        expect(screen.getByText(/situacoes_de_handoff/)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Salvar modelo e Jev" })).toBeDisabled();

        // RL-12: seções planas com o cabeçalho comum do agente, sem cartão.
        expect(screen.getByRole("heading", { level: 1, name: "Beatriz" })).toBeInTheDocument();
        expect(screen.getByRole("region", { name: "Modelo de linguagem" })).toBeInTheDocument();
        expect(screen.getByRole("region", { name: "Jev: portões de decisão" })).toBeInTheDocument();
        expect(document.querySelector(".sofa-card")).not.toBeInTheDocument();
    });

    it("grava só o que mudou no modelo e no RAG, com If-Match da versão", async () => {
        const servidor = servidorCom();
        montar();
        await screen.findByLabelText("Modelo");
        const usuario = userEvent.setup();

        await usuario.clear(screen.getByLabelText("Modelo"));
        await usuario.type(screen.getByLabelText("Modelo"), "gemini-3.8-flash-high");
        await usuario.clear(screen.getByLabelText("Trechos recuperados (top-k)"));
        await usuario.type(screen.getByLabelText("Trechos recuperados (top-k)"), "5");
        await usuario.click(screen.getByRole("button", { name: "Salvar modelo e Jev" }));

        await waitFor(() => expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)).toHaveLength(1));
        const envio = servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)[0]!;
        expect(envio.cabecalhos["if-match"]).toBe("1");
        expect(envio.corpo).toEqual({ model: "gemini-3.8-flash-high", rag_top_k: 5 });
        expect(await screen.findByRole("status")).toHaveTextContent("Modelo e Jev gravados (versão 2)");
    });

    it("ligar um gate manda o jev_json com despedida, motivos e chaves desconhecidas preservadas", async () => {
        const servidor = servidorCom({
            agente: agenteJson({
                jev: {
                    despedida: "Até logo!",
                    situacoes_de_handoff: ["cliente pede segunda via"],
                    resposta_segura: "Vou confirmar e já te respondo.",
                    g7_acao: "registrar",
                    chave_desconhecida: 7,
                    gates: { g8: false },
                },
            }),
        });
        montar();
        await screen.findByLabelText("G2 · Esperar mais mensagens ou responder agora?");
        const usuario = userEvent.setup();

        await usuario.selectOptions(screen.getByLabelText("G2 · Esperar mais mensagens ou responder agora?"), "ligado");
        await usuario.click(screen.getByRole("button", { name: "Salvar modelo e Jev" }));

        await waitFor(() => expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)).toHaveLength(1));
        const corpo = corpoDoPatch(servidor);
        expect(Object.keys(corpo)).toEqual(["jev_json"]);
        expect(JSON.parse(corpo.jev_json as string)).toEqual({
            despedida: "Até logo!",
            situacoes_de_handoff: ["cliente pede segunda via"],
            resposta_segura: "Vou confirmar e já te respondo.",
            g7_acao: "registrar",
            chave_desconhecida: 7,
            gates: { g8: false, g2: true },
        });
    });

    it("herdar um gate tira a chave dele, mas não mexe no resto", async () => {
        const servidor = servidorCom();
        montar();
        await screen.findByLabelText("G8 · A conversa foi encerrada pelo cliente?");
        const usuario = userEvent.setup();

        await usuario.selectOptions(screen.getByLabelText("G8 · A conversa foi encerrada pelo cliente?"), "herdar");
        await usuario.click(screen.getByRole("button", { name: "Salvar modelo e Jev" }));

        await waitFor(() => expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)).toHaveLength(1));
        expect(JSON.parse(corpoDoPatch(servidor).jev_json as string)).toEqual({
            despedida: "Até logo!",
            situacoes_de_handoff: ["cliente pede segunda via"],
        });
    });

    it("desligar o Jev no agente manda ativo: false e preserva o resto", async () => {
        const servidor = servidorCom();
        montar();
        await screen.findByLabelText("Modelo");
        const usuario = userEvent.setup();

        await usuario.click(screen.getByRole("switch", { name: /Desligar o Jev neste agente/ }));
        await usuario.click(screen.getByRole("button", { name: "Salvar modelo e Jev" }));

        await waitFor(() => expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)).toHaveLength(1));
        expect(JSON.parse(corpoDoPatch(servidor).jev_json as string)).toEqual({
            despedida: "Até logo!",
            situacoes_de_handoff: ["cliente pede segunda via"],
            gates: { g8: false },
            ativo: false,
        });
    });

    it("validação de faixa recusa a janela de 9000 ms sem chamar a API", async () => {
        const servidor = servidorCom();
        montar();
        await screen.findByLabelText("Janela de agrupamento (ms)");
        const usuario = userEvent.setup();

        await usuario.clear(screen.getByLabelText("Janela de agrupamento (ms)"));
        await usuario.type(screen.getByLabelText("Janela de agrupamento (ms)"), "9000");
        await usuario.click(screen.getByRole("button", { name: "Salvar modelo e Jev" }));

        expect(await screen.findByText(/use um número de 0 a 8000 ms/)).toBeInTheDocument();
        expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)).toHaveLength(0);
    });

    it("temperatura já definida não volta a vazio pela tela: avisa, sem chamar a API", async () => {
        const servidor = servidorCom({ agente: agenteJson({ temperature: 0.7 }) });
        montar();
        const temperatura = await screen.findByLabelText("Temperatura");
        expect(temperatura).toHaveValue("0.7");
        const usuario = userEvent.setup();

        await usuario.clear(temperatura);
        await usuario.click(screen.getByRole("button", { name: "Salvar modelo e Jev" }));

        expect(await screen.findByText(/a tela não volta ao padrão do servidor/)).toBeInTheDocument();
        expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)).toHaveLength(0);
    });

    it("412 relê, adota a versão atual e mantém o que foi digitado; o próximo salvamento usa a nova versão", async () => {
        const servidor = servidorCom({
            patches: [
                { status: 412, corpo: { code: "version_conflict", message: "versão desatualizada", details: { current_version: 2, expected_version: 1 } } },
                { status: 200, corpo: agenteJson({ version: 3, model: "gemini-3.8-flash-high" }) },
            ],
            relido: agenteJson({ version: 2, model: "outro-modelo" }),
        });
        montar();
        await screen.findByLabelText("Modelo");
        const usuario = userEvent.setup();

        await usuario.clear(screen.getByLabelText("Modelo"));
        await usuario.type(screen.getByLabelText("Modelo"), "gemini-3.8-flash-high");
        await usuario.click(screen.getByRole("button", { name: "Salvar modelo e Jev" }));

        expect(await screen.findByText(/Outra pessoa salvou este agente antes de você/)).toBeInTheDocument();
        expect(screen.getByLabelText("Modelo")).toHaveValue("gemini-3.8-flash-high");

        await usuario.click(screen.getByRole("button", { name: "Salvar modelo e Jev" }));
        await waitFor(() => expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)).toHaveLength(2));
        expect(servidor.chamadasPara("PATCH", `/agents/${AGENTE}`)[1]!.cabecalhos["if-match"]).toBe("2");
        expect(corpoDoPatch(servidor, 1)).toEqual({ model: "gemini-3.8-flash-high" });
    });

    it("erro de campo da API aparece no campo, com a mensagem da API", async () => {
        servidorCom({
            patches: [
                {
                    status: 400,
                    corpo: {
                        code: "validation_error",
                        message: "dados inválidos",
                        details: { field: "max_tool_calls", reason: "use no mínimo 1 chamada" },
                    },
                },
            ],
        });
        montar();
        await screen.findByLabelText("Ferramentas por mensagem");
        const usuario = userEvent.setup();

        await usuario.clear(screen.getByLabelText("Ferramentas por mensagem"));
        await usuario.type(screen.getByLabelText("Ferramentas por mensagem"), "1");
        await usuario.click(screen.getByRole("button", { name: "Salvar modelo e Jev" }));

        expect(await screen.findByText("use no mínimo 1 chamada")).toBeInTheDocument();
    });

    it("erro ao carregar mostra o estado de erro com nova tentativa", async () => {
        servidorCom({ leituraFalha: true });
        montar();
        expect(await screen.findByText("Não consegui carregar o modelo e o Jev do agente")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
    });

    it("reage à troca para en-US com a tela aberta e mantém o formulário em inglês", async () => {
        servidorCom();
        montar();
        expect(await screen.findByLabelText("Provedor")).toBeInTheDocument();

        act(() => {
            setLocale("en-US");
        });

        expect(await screen.findByLabelText("Provider")).toHaveValue("antigravity2api");
        expect(screen.getByLabelText("Model")).toHaveValue("gemini-3.8-flash-tiered");
        expect(screen.getByLabelText("Temperature")).toHaveAttribute("placeholder", "inherits server default");
        expect(screen.getByRole("region", { name: "Language model" })).toBeInTheDocument();
        expect(screen.getByRole("region", { name: "Jev: decision gates" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Save model and Jev" })).toBeDisabled();
        setLocale("pt-BR");
    });
});
