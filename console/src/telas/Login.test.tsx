/**
 * Testes da tela de login no layout novo (RL-13).
 *
 * Confere a marca, o campo do token com rótulo e foco inicial, o aviso de uso interno, a entrada
 * com token válido (leva a /orgs), o erro de token recusado (fica na tela) e a falha de conexão.
 * A chamada de validação é `GET /admin/v1/orgs`, a mesma que a sessão já faz.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { SessaoProvider } from "../sessao/Sessao";
import { ServidorFalso } from "../testes/servidor";
import { Login } from "./Login";

function montar() {
    return render(
        <MemoryRouter initialEntries={["/login"]}>
            <SessaoProvider>
                <Routes>
                    <Route path="/login" element={<Login />} />
                    <Route path="/orgs" element={<p>lista de organizações</p>} />
                </Routes>
            </SessaoProvider>
        </MemoryRouter>,
    );
}

function campoDoToken() {
    // O rótulo tem o "*" de obrigatório junto, então a busca é por trecho.
    return screen.getByLabelText(/Token administrativo/) as HTMLInputElement;
}

afterEach(() => {
    vi.unstubAllGlobals();
    window.sessionStorage.clear();
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

    it("token válido entra e leva para /orgs, guardando a sessão na aba", async () => {
        const servidor = new ServidorFalso();
        servidor
            .responder("GET", "/admin/v1/orgs", { status: 200, corpo: { items: [], next_cursor: null } })
            .instalar();
        montar();

        await userEvent.type(campoDoToken(), "pophg_teste");
        await userEvent.click(screen.getByRole("button", { name: "Entrar no Console" }));

        expect(await screen.findByText("lista de organizações")).toBeInTheDocument();
        const gravada = JSON.parse(window.sessionStorage.getItem("poph-console.sessao") ?? "{}");
        expect(gravada.token).toBe("pophg_teste");
        expect(gravada.escopo).toBe("global");
    });

    it("token recusado mostra o erro no campo e não navega", async () => {
        const servidor = new ServidorFalso();
        servidor
            .responder("GET", "/admin/v1/orgs", { status: 401, corpo: { error: "unauthorized" } })
            .instalar();
        montar();

        await userEvent.type(campoDoToken(), "pophg_errado");
        await userEvent.click(screen.getByRole("button", { name: "Entrar no Console" }));

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

        await userEvent.type(campoDoToken(), "pophg_teste");
        await userEvent.click(screen.getByRole("button", { name: "Entrar no Console" }));

        expect(await screen.findByText("Sem conexão")).toBeInTheDocument();
        expect(screen.getByText(/Não foi possível alcançar a instância/)).toBeInTheDocument();
        await waitFor(() => expect(campoDoToken()).toBeInTheDocument());
        expect(screen.queryByText("lista de organizações")).toBeNull();
    });
});
