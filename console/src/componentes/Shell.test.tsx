/**
 * Testes da casca (HR-61): botão de ajuda no header abre o modal de atalhos, e o contador do item
 * Agentes do menu usa uma consulta só por organização, guardada na sessão. Sem organização aberta
 * não há contador nem consulta; se a consulta falha, o item fica sem contador e a tela segue.
 */

import { afterEach, describe, expect, it } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { SessaoProvider } from "../sessao/Sessao";
import { ServidorFalso, erroApi } from "../testes/servidor";
import { GuardaAlteracoesProvider } from "./GuardaAlteracoes";
import { Shell } from "./Shell";

const CHAVE_DA_SESSAO = "poph-console.sessao";

const SAUDE = {
    status: "ok",
    version: "0.4.2",
    uptime_seconds: 10,
    databases: { harness: 1, rag: 1 },
};

const DUAS_ORGS = [
    { id: "org_1", name: "Loja Antiga", slug: "loja-antiga", status: "active" },
    { id: "org_2", name: "Clínica Nova", slug: "clinica-nova", status: "active" },
];

const TRES_AGENTES = {
    items: [
        { id: "agt_1", name: "Atendente", org_id: "org_1" },
        { id: "agt_2", name: "Triagem", org_id: "org_1" },
        { id: "agt_3", name: "Vendas", org_id: "org_1" },
    ],
    next_cursor: null,
};

function prepararSessao() {
    window.sessionStorage.setItem(
        CHAVE_DA_SESSAO,
        JSON.stringify({ token: "pophg_teste", escopo: "global", orgs: DUAS_ORGS }),
    );
}

function montar(caminho: string) {
    return render(
        <MemoryRouter initialEntries={[caminho]}>
            <SessaoProvider>
                <GuardaAlteracoesProvider>
                    <Shell>
                        <p>Conteúdo da tela</p>
                    </Shell>
                </GuardaAlteracoesProvider>
            </SessaoProvider>
        </MemoryRouter>,
    );
}

afterEach(() => {
    window.sessionStorage.removeItem(CHAVE_DA_SESSAO);
});

describe("Casca do Console (HR-61)", () => {
    it("botão de ajuda no header abre o modal de atalhos", async () => {
        const usuario = userEvent.setup();
        prepararSessao();
        new ServidorFalso().responder("GET", "/healthz", { status: 200, corpo: SAUDE }).instalar();
        montar("/orgs");

        await usuario.click(screen.getByRole("button", { name: "Atalhos do teclado" }));

        expect(await screen.findByRole("dialog", { name: "Atalhos do Console" })).toBeInTheDocument();
    });

    it("mostra o total de agentes da organização aberta, com uma consulta só ao navegar", async () => {
        const usuario = userEvent.setup();
        prepararSessao();
        const servidor = new ServidorFalso()
            .responder("GET", "/healthz", { status: 200, corpo: SAUDE })
            .responder("GET", "/admin/v1/orgs/org_1/agents", { status: 200, corpo: TRES_AGENTES })
            .instalar();
        montar("/orgs/org_1/structure");

        const agentes = screen.getByRole("link", { name: /Agentes/ });
        await waitFor(() => expect(agentes).toHaveTextContent("3"));

        // Trocar de tela dentro da mesma organização não repete a consulta.
        await usuario.click(screen.getByRole("link", { name: /Produtos e catálogo/ }));
        await usuario.click(screen.getByRole("link", { name: /Agentes/ }));
        expect(screen.getByRole("link", { name: /Agentes/ })).toHaveTextContent("3");
        expect(servidor.chamadasPara("GET", "/admin/v1/orgs/org_1/agents")).toHaveLength(1);
    });

    it("sem organização aberta, o item Agentes não tem contador nem faz consulta", () => {
        prepararSessao();
        const servidor = new ServidorFalso()
            .responder("GET", "/healthz", { status: 200, corpo: SAUDE })
            .instalar();
        montar("/orgs");

        const agentes = screen.getByRole("button", { name: /Agentes/ });
        expect(agentes).not.toHaveTextContent(/\d/);
        expect(servidor.chamadasPara("GET", "/agents")).toHaveLength(0);
    });

    it("se a consulta de agentes falha, o item fica sem contador e a tela segue", async () => {
        prepararSessao();
        const servidor = new ServidorFalso()
            .responder("GET", "/healthz", { status: 200, corpo: SAUDE })
            .responder("GET", "/admin/v1/orgs/org_1/agents", erroApi(500, "interno", "falha de teste"))
            .instalar();
        montar("/orgs/org_1/structure");

        // Espera a consulta sair e a falha voltar (a promessa rejeitada precisa ser processada).
        await waitFor(() =>
            expect(servidor.chamadasPara("GET", "/admin/v1/orgs/org_1/agents")).toHaveLength(1),
        );
        await new Promise((resolver) => setTimeout(resolver, 20));

        expect(screen.getByRole("link", { name: /Agentes/ })).not.toHaveTextContent(/\d/);
        expect(screen.getByText("Conteúdo da tela")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Atalhos do teclado" })).toBeInTheDocument();
    });
});
