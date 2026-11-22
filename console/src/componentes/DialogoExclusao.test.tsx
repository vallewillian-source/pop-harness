/**
 * Testes do diálogo de exclusão: alta fricção (digitar o slug), a recusa por conteúdo (`409
 * conflict`) mostrada como veio e a confirmação extra que libera o `?force=true`.
 */

import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { excluirOrg } from "../api/orgs";
import { DialogoExclusao } from "./DialogoExclusao";
import { esperarFocoDoModal } from "../testes/modal";
import { ServidorFalso, erroApi } from "../testes/servidor";

function montar(aoConfirmar: (forcar: boolean) => Promise<void>, aoExcluir = vi.fn(), aoFechar = vi.fn()) {
    return render(
        <DialogoExclusao
            aberto
            aoFechar={aoFechar}
            titulo="Excluir organização?"
            descricao="A organização e tudo que está dentro dela saem do sistema."
            recurso="loja-antiga"
            rotuloRecurso="slug da organização"
            rotuloConfirmar="Excluir organização"
            permiteForcar
            rotuloForcar="Apagar também o conteúdo"
            aoConfirmar={aoConfirmar}
            aoExcluir={aoExcluir}
        />,
    );
}

describe("DialogoExclusao", () => {
    it("só habilita o botão quando o slug digitado é exato", async () => {
        const usuario = userEvent.setup();
        montar(vi.fn().mockResolvedValue(undefined));
        await esperarFocoDoModal();

        const confirmar = screen.getByRole("button", { name: "Excluir organização" });
        expect(confirmar).toBeDisabled();

        await usuario.type(screen.getByLabelText(/Digite/i), "loja");
        expect(confirmar).toBeDisabled();

        await usuario.clear(screen.getByLabelText(/Digite/i));
        await usuario.type(screen.getByLabelText(/Digite/i), "loja-antiga");
        expect(confirmar).toBeEnabled();
    });

    it("mostra a recusa por conteúdo (409) da API e libera o force", async () => {
        const usuario = userEvent.setup();
        const aoConfirmar = vi
            .fn()
            .mockRejectedValueOnce(
                Object.assign(new Error("a organização tem agentes; use force"), {
                    name: "ErroApi",
                }),
            )
            .mockResolvedValueOnce(undefined);
        montar(aoConfirmar);
        await esperarFocoDoModal();

        await usuario.type(screen.getByLabelText(/Digite/i), "loja-antiga");
        await usuario.click(screen.getByRole("button", { name: "Excluir organização" }));

        expect(await screen.findByText("a organização tem agentes; use force")).toBeInTheDocument();
        // O diálogo continua aberto: nada foi excluído.
        expect(screen.getByRole("checkbox")).toBeInTheDocument();
    });

    it("confirma sem force, recebe 409 da API real e depois exclui com ?force=true", async () => {
        const usuario = userEvent.setup();
        const aoExcluir = vi.fn();
        const aoFechar = vi.fn();
        const servidor = new ServidorFalso()
            .responder("DELETE", "/admin/v1/orgs/org_1", (chamada) =>
                chamada.url.includes("force=true")
                    ? { status: 204 }
                    : erroApi(409, "conflict", "a organização tem agentes; confirme com force"),
            )
            .instalar();

        montar((forcar) => excluirOrg("org_1", forcar), aoExcluir, aoFechar);
        await esperarFocoDoModal();
        await usuario.type(screen.getByLabelText(/Digite/i), "loja-antiga");
        await usuario.click(screen.getByRole("button", { name: "Excluir organização" }));

        expect(
            await screen.findByText("a organização tem agentes; confirme com force"),
        ).toBeInTheDocument();

        await usuario.click(screen.getByRole("checkbox"));
        await usuario.click(screen.getByRole("button", { name: "Excluir organização" }));

        await waitFor(() => expect(aoExcluir).toHaveBeenCalled());
        expect(aoFechar).toHaveBeenCalled();
        const pedidos = servidor.chamadasPara("DELETE", "/admin/v1/orgs/org_1");
        expect(pedidos).toHaveLength(2);
        expect(pedidos[0]?.url).not.toContain("force=true");
        expect(pedidos[1]?.url).toContain("force=true");
    });
});
