/**
 * Testes da tela de login no layout novo (RL-13) e da porta de entrada do Console (CN-01).
 *
 * Confere a marca, o campo do token com rótulo e foco inicial, o aviso de uso interno, a entrada
 * com token válido (leva à organização padrão), o erro de token recusado (fica na tela) e a falha
 * de conexão. A chamada de validação é `GET /admin/v1/orgs`, a mesma que a sessão já faz.
 *
 * CN-01: o pós-login cai na última organização aberta (`localStorage`, chave
 * `poph-console.lastOrgId`), na primeira da sessão quando não há registro válido e em `/orgs`
 * quando a sessão não tem nenhuma organização.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useParams } from "react-router-dom";
import { setLocale } from "../i18n";
import { LAST_ORG_STORAGE_KEY, SessionProvider } from "../session/Session";
import { FakeServer } from "../testing/server";
import { Login } from "./Login";

const ORGS = [
    { id: "org_1", name: "Loja Antiga", slug: "loja-antiga", status: "active" },
    { id: "org_2", name: "Clínica Nova", slug: "clinica-nova", status: "active" },
];

function montar() {
    return render(
        <MemoryRouter initialEntries={["/login"]}>
            <SessionProvider>
                <Routes>
                    <Route path="/login" element={<Login />} />
                    <Route path="/orgs" element={<p>lista de organizações</p>} />
                    <Route path="/orgs/:orgId" element={<PainelDaOrg />} />
                </Routes>
            </SessionProvider>
        </MemoryRouter>,
    );
}

function PainelDaOrg() {
    const { orgId = "" } = useParams();
    return <p>painel da organização {orgId}</p>;
}

function campoDoToken() {
    // O rótulo tem o "*" de obrigatório junto, então a busca é por trecho.
    return screen.getByLabelText(/Token administrativo/) as HTMLInputElement;
}

function servirOrgs(itens: { id: string; name: string; slug: string; status: string }[]) {
    return new FakeServer()
        .responder("GET", "/admin/v1/orgs", {
            status: 200,
            corpo: { items: itens, next_cursor: null },
        })
        .instalar();
}

async function entrarCom(token: string) {
    await userEvent.type(campoDoToken(), token);
    await userEvent.click(screen.getByRole("button", { name: "Entrar no Console" }));
}

afterEach(() => {
    vi.unstubAllGlobals();
    window.sessionStorage.clear();
    window.localStorage.clear();
    setLocale("pt-BR");
});

describe("Login", () => {
    it("mostra a marca POP HARNESS e não usa cartão", () => {
        montar();

        expect(screen.getByText("POP HARNESS")).toBeInTheDocument();
        expect(screen.getByRole("heading", { level: 1, name: "Console do POP Harness" })).toBeInTheDocument();
        expect(document.querySelector(".sofa-card")).toBeNull();
    });

    it("o campo do token tem rótulo visível e é o foco inicial", () => {
        montar();

        expect(campoDoToken()).toHaveFocus();
        expect(campoDoToken().type).toBe("password");
    });

    it("mostra o aviso de uso interno", () => {
        montar();

        expect(screen.getByText("Acesso administrativo por token. Uso interno.")).toBeInTheDocument();
    });

    it("o botão de mostrar token alterna a visibilidade do campo", async () => {
        montar();

        await userEvent.click(screen.getByRole("button", { name: "Mostrar token" }));

        expect(campoDoToken().type).toBe("text");
        expect(screen.getByRole("button", { name: "Ocultar token" })).toBeInTheDocument();
    });

    it("sem organização na sessão, token válido entra e leva para /orgs, guardando a sessão na aba", async () => {
        servirOrgs([]);
        montar();

        await entrarCom("pophg_teste");

        expect(await screen.findByText("lista de organizações")).toBeInTheDocument();
        const gravada = JSON.parse(window.sessionStorage.getItem("poph-console.sessao") ?? "{}");
        expect(gravada.token).toBe("pophg_teste");
        expect(gravada.escopo).toBe("global");
    });

    it("com organizações e nenhuma lembrada, token válido abre a primeira da lista", async () => {
        servirOrgs(ORGS);
        montar();

        await entrarCom("pophg_teste");

        expect(await screen.findByText("painel da organização org_1")).toBeInTheDocument();
        expect(window.localStorage.getItem(LAST_ORG_STORAGE_KEY)).toBeNull();
    });

    it("token válido abre a última organização lembrada em localStorage", async () => {
        window.localStorage.setItem(LAST_ORG_STORAGE_KEY, "org_2");
        servirOrgs(ORGS);
        montar();

        await entrarCom("pophg_teste");

        expect(await screen.findByText("painel da organização org_2")).toBeInTheDocument();
        expect(window.localStorage.getItem(LAST_ORG_STORAGE_KEY)).toBe("org_2");
    });

    it("registro lembrado que não está mais na sessão cai na primeira organização", async () => {
        window.localStorage.setItem(LAST_ORG_STORAGE_KEY, "org_que_sumiu");
        servirOrgs(ORGS);
        montar();

        await entrarCom("pophg_teste");

        expect(await screen.findByText("painel da organização org_1")).toBeInTheDocument();
    });

    it("token recusado mostra o erro no campo e não navega", async () => {
        const servidor = new FakeServer();
        servidor
            .responder("GET", "/admin/v1/orgs", { status: 401, corpo: { error: "unauthorized" } })
            .instalar();
        montar();

        await entrarCom("pophg_errado");

        expect(
            await screen.findByText(/Token administrativo inválido ou revogado/),
        ).toBeInTheDocument();
        expect(screen.queryByText("lista de organizações")).toBeNull();
        expect(campoDoToken()).toBeInTheDocument();
        expect(window.sessionStorage.getItem("poph-console.sessao")).toBeNull();
    });

    it("falha de conexão mostra o aviso 'Sem conexão' e fica na tela", async () => {
        vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));
        montar();

        await entrarCom("pophg_teste");

        expect(await screen.findByText("Sem conexão")).toBeInTheDocument();
        expect(screen.getByText(/Não foi possível alcançar a instância/)).toBeInTheDocument();
        await waitFor(() => expect(campoDoToken()).toBeInTheDocument());
        expect(screen.queryByText("lista de organizações")).toBeNull();
    });

    it("switches to en-US when clicking EN and persists in localStorage", async () => {
        montar();

        const enButton = screen.getByRole("button", { name: "Inglês (Estados Unidos)" });
        await userEvent.click(enButton);

        expect(
            screen.getByRole("heading", { level: 1, name: "POP Harness Console" }),
        ).toBeInTheDocument();
        expect(
            screen.getByText("Token-based administrative access. Internal use."),
        ).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Sign in to Console" })).toBeInTheDocument();
        expect(screen.getByLabelText(/Administrative token/)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "English (United States)" })).toHaveAttribute(
            "aria-pressed",
            "true",
        );
        expect(document.documentElement.lang).toBe("en-US");
        expect(window.localStorage.getItem("poph-console.locale")).toBe("en-US");

        await userEvent.click(screen.getByRole("button", { name: "Portuguese (Brazil)" }));
        expect(
            screen.getByRole("heading", { level: 1, name: "Console do POP Harness" }),
        ).toBeInTheDocument();
    });
});
