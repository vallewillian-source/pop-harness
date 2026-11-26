
import { afterEach, describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import type { Org } from "../api/client";
import { SessionProvider } from "../session/Session";
import { FakeServer } from "../testing/server";
import { OrgPanel } from "./OrgPanel";
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

function prepareSession(token: string, escopo: "global" | "org") {
    window.sessionStorage.setItem(
        "poph-console.sessao",
        JSON.stringify({ token, escopo, orgs: [ORG] }),
    );
}

function serverWithOrg(org: Org = ORG) {
    const servidor = new FakeServer();
    servidor
        .respond("GET", "/admin/v1/orgs/org_1", { status: 200, body: org })
        .respond("GET", "/admin/v1/orgs", { status: 200, body: { items: [org], next_cursor: null } })
        .install();
    return servidor;
}

function build() {
    return render(
        <MemoryRouter initialEntries={["/orgs/org_1"]}>
            <SessionProvider>
                <Routes>
                    <Route path="/orgs/:orgId" element={<OrgPanel />} />
                    <Route path="/orgs" element={<p>lista de organizações</p>} />
                </Routes>
            </SessionProvider>
        </MemoryRouter>,
    );
}

describe("OrgPainel em en-US", () => {
    afterEach(() => setLocale("pt-BR"));

    it("traduz cabeçalho, resumo, atalhos e a confirmação de exclusão", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global");
        serverWithOrg();
        setLocale("en-US");

        build();

        expect(await screen.findByRole("heading", { level: 1, name: "Loja Antiga" })).toBeInTheDocument();
        const cabeca = document.querySelector(".poph-org-panel__title-line") as HTMLElement;
        expect(within(cabeca).getByText("Active")).toBeInTheDocument();
        const summary = screen.getByRole("heading", { name: "Summary" }).closest("section");
        expect(within(summary!).getByText("Identifier")).toBeInTheDocument();
        expect(within(summary!).getByText("30 days")).toBeInTheDocument();
        expect(screen.getByRole("link", { name: "Open structure" })).toHaveAttribute(
            "href",
            "/orgs/org_1/structure",
        );

        await user.click(screen.getByRole("button", { name: "Delete" }));
        const dialog = await screen.findByRole("dialog");
        expect(dialog).toHaveAccessibleName("Delete organization?");
        expect(within(dialog).getByText(/There is no undo/)).toBeInTheDocument();
    });
});

describe("OrgPainel", () => {
    it("mostra o nome e a situação da organização e o resumo de configuração", async () => {
        prepareSession("pophg_teste", "global");
        serverWithOrg();

        build();

        expect(await screen.findByRole("heading", { level: 1, name: "Loja Antiga" })).toBeInTheDocument();
        const cabeca = document.querySelector(".poph-org-panel__title-line") as HTMLElement;
        expect(within(cabeca).getByText("Ativa")).toBeInTheDocument();
        const summary = screen.getByRole("heading", { name: "Resumo" }).closest("section");
        expect(summary).not.toBeNull();
        expect(within(summary!).getByText("loja-antiga")).toBeInTheDocument();
        expect(within(summary!).getByText("30 dias")).toBeInTheDocument();
        expect(document.querySelector(".sofa-card")).toBeNull();
    });

    it("Editar abre o formulário de edição com os dados da organização", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global");
        serverWithOrg();

        build();
        await screen.findByRole("heading", { level: 1, name: "Loja Antiga" });

        await user.click(screen.getByRole("button", { name: "Editar" }));
        const dialog = await screen.findByRole("dialog");
        expect(dialog).toHaveAccessibleName("Editar organização");
        expect(within(dialog).getByRole("textbox", { name: "Nome" })).toHaveValue("Loja Antiga");
        expect(within(dialog).getByRole("textbox", { name: "Slug" })).toHaveValue("loja-antiga");
    });

    it("Excluir chega à confirmação com o slug da organização", async () => {
        const user = userEvent.setup();
        prepareSession("pophg_teste", "global");
        const servidor = serverWithOrg();

        build();
        await screen.findByRole("heading", { level: 1, name: "Loja Antiga" });

        await user.click(screen.getByRole("button", { name: "Excluir" }));
        const dialog = await screen.findByRole("dialog");
        expect(dialog).toHaveAccessibleName("Excluir organização?");
        expect(within(dialog).getByRole("textbox")).toBeInTheDocument();
        expect(servidor.callsFor("DELETE", "/admin/v1/orgs/org_1")).toHaveLength(0);
    });

    it("token de org não vê Editar nem Excluir, mas vê os atalhos", async () => {
        prepareSession("poph_org_teste", "org");
        serverWithOrg();

        build();
        await screen.findByRole("heading", { level: 1, name: "Loja Antiga" });

        expect(screen.queryByRole("button", { name: "Editar" })).not.toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Excluir" })).not.toBeInTheDocument();
        expect(screen.getByRole("link", { name: "Abrir estrutura" })).toHaveAttribute(
            "href",
            "/orgs/org_1/structure",
        );
    });

    it("os atalhos de estrutura, produtos e agentes são links, sem cartão", async () => {
        prepareSession("pophg_teste", "global");
        serverWithOrg();

        build();
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
