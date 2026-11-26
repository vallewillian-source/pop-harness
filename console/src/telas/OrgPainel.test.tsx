/**
 * Testes do painel da organização no layout de D2 (RL-07).
 *
 * O painel mantém os fluxos de antes: Editar abre o modal com os dados da organização, Excluir
 * abre a confirmação, e token de org não vê as ações de gestão. A leitura da organização é
 * `GET /admin/v1/orgs/{id}`; a lista (`GET /admin/v1/orgs`) alimenta só o seletor da sessão.
 */

import { afterEach, describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import type { Org } from "../api/client";
import { SessionProvider } from "../session/Session";
import { FakeServer } from "../testing/server";
import { OrgPainel } from "./OrgPainel";
import { setLocale } from "../i18n";

const ORG: Org = {
    id: "org_1",
    name: "Loja Antiga",
    slug: "loja-antiga",
    status: "active",
    version: 3,
    config: { retention_days: 30 } as unknown as Org["config"],
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
};

function prepararSessao(token: string, escopo: "global" | "org") {
    window.sessionStorage.setItem(
        "poph-console.sessao",
        JSON.stringify({ token, escopo, orgs: [ORG] }),
    );
}

/**
 * As rotas específicas vêm antes: o roteador falso responde à primeira que casa, e
 * `/admin/v1/orgs` também casa com `/admin/v1/orgs/org_1`.
 */
function servidorComOrg(org: Org = ORG) {
    const servidor = new FakeServer();
    servidor
        .responder("GET", "/admin/v1/orgs/org_1", { status: 200, corpo: org })
        .responder("GET", "/admin/v1/orgs", { status: 200, corpo: { items: [org], next_cursor: null } })
        .instalar();
    return servidor;
}

function montar() {
    return render(
        <MemoryRouter initialEntries={["/orgs/org_1"]}>
            <SessionProvider>
                <Routes>
                    <Route path="/orgs/:orgId" element={<OrgPainel />} />
                    <Route path="/orgs" element={<p>lista de organizações</p>} />
                </Routes>
            </SessionProvider>
        </MemoryRouter>,
    );
}

describe("OrgPainel em en-US", () => {
    afterEach(() => setLocale("pt-BR"));

    it("traduz cabeçalho, resumo, atalhos e a confirmação de exclusão", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global");
        servidorComOrg();
        setLocale("en-US");

        montar();

        expect(await screen.findByRole("heading", { level: 1, name: "Loja Antiga" })).toBeInTheDocument();
        const cabeca = document.querySelector(".poph-orgpainel__titulo-linha") as HTMLElement;
        expect(within(cabeca).getByText("Active")).toBeInTheDocument();
        const resumo = screen.getByRole("heading", { name: "Summary" }).closest("section");
        expect(within(resumo!).getByText("Identifier")).toBeInTheDocument();
        expect(within(resumo!).getByText("30 days")).toBeInTheDocument();
        expect(screen.getByRole("link", { name: "Open structure" })).toHaveAttribute(
            "href",
            "/orgs/org_1/structure",
        );

        await usuario.click(screen.getByRole("button", { name: "Delete" }));
        const dialogo = await screen.findByRole("dialog");
        expect(dialogo).toHaveAccessibleName("Delete organization?");
        expect(within(dialogo).getByText(/There is no undo/)).toBeInTheDocument();
    });
});

describe("OrgPainel", () => {
    it("mostra o nome e a situação da organização e o resumo de configuração", async () => {
        prepararSessao("pophg_teste", "global");
        servidorComOrg();

        montar();

        expect(await screen.findByRole("heading", { level: 1, name: "Loja Antiga" })).toBeInTheDocument();
        // "Ativa" aparece no cabeçalho e de novo no resumo: confere a do cabeçalho.
        const cabeca = document.querySelector(".poph-orgpainel__titulo-linha") as HTMLElement;
        expect(within(cabeca).getByText("Ativa")).toBeInTheDocument();
        const resumo = screen.getByRole("heading", { name: "Resumo" }).closest("section");
        expect(resumo).not.toBeNull();
        expect(within(resumo!).getByText("loja-antiga")).toBeInTheDocument();
        expect(within(resumo!).getByText("30 dias")).toBeInTheDocument();
        // A tela não usa cartão: nenhuma seção fica dentro de outra.
        expect(document.querySelector(".sofa-card")).toBeNull();
    });

    it("Editar abre o formulário de edição com os dados da organização", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global");
        servidorComOrg();

        montar();
        await screen.findByRole("heading", { level: 1, name: "Loja Antiga" });

        await usuario.click(screen.getByRole("button", { name: "Editar" }));
        const dialogo = await screen.findByRole("dialog");
        expect(dialogo).toHaveAccessibleName("Editar organização");
        expect(within(dialogo).getByRole("textbox", { name: "Nome" })).toHaveValue("Loja Antiga");
        expect(within(dialogo).getByRole("textbox", { name: "Slug" })).toHaveValue("loja-antiga");
    });

    it("Excluir chega à confirmação com o slug da organização", async () => {
        const usuario = userEvent.setup();
        prepararSessao("pophg_teste", "global");
        const servidor = servidorComOrg();

        montar();
        await screen.findByRole("heading", { level: 1, name: "Loja Antiga" });

        await usuario.click(screen.getByRole("button", { name: "Excluir" }));
        const dialogo = await screen.findByRole("dialog");
        expect(dialogo).toHaveAccessibleName("Excluir organização?");
        expect(within(dialogo).getByRole("textbox")).toBeInTheDocument();
        // Confirmar a exclusão só acontece depois de digitar o slug: nada foi enviado ainda.
        expect(servidor.chamadasPara("DELETE", "/admin/v1/orgs/org_1")).toHaveLength(0);
    });

    it("token de org não vê Editar nem Excluir, mas vê os atalhos", async () => {
        prepararSessao("poph_org_teste", "org");
        servidorComOrg();

        montar();
        await screen.findByRole("heading", { level: 1, name: "Loja Antiga" });

        expect(screen.queryByRole("button", { name: "Editar" })).not.toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Excluir" })).not.toBeInTheDocument();
        expect(screen.getByRole("link", { name: "Abrir estrutura" })).toHaveAttribute(
            "href",
            "/orgs/org_1/structure",
        );
    });

    it("os atalhos de estrutura, produtos e agentes são links, sem cartão", async () => {
        prepararSessao("pophg_teste", "global");
        servidorComOrg();

        montar();
        await screen.findByRole("heading", { level: 1, name: "Loja Antiga" });

        expect(screen.getByRole("link", { name: "Abrir produtos" })).toHaveAttribute(
            "href",
            "/orgs/org_1/products",
        );
        expect(screen.getByRole("link", { name: "Abrir agentes" })).toHaveAttribute(
            "href",
            "/orgs/org_1/agents",
        );
    });
});
