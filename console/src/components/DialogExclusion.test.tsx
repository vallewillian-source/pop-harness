
import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { deleteOrg } from "../api/orgs";
import { DialogExclusion } from "../components/DialogExclusion";
import { waitForModalFocus } from "../testing/modal";
import { FakeServer, apiError } from "../testing/server";
import { setLocale } from "../i18n";

function build(onConfirm: (forcar: boolean) => Promise<void>, onDelete = vi.fn(), onClose = vi.fn()) {
    return render(
        <DialogExclusion
            aberto
            onClose={onClose}
            titulo="Excluir organização?"
            descricao="A organização e tudo que está dentro dela saem do sistema."
            resource="loja-antiga"
            labelResource="slug da organização"
            labelConfirm="Excluir organização"
            permiteForcar
            labelForce="Apagar também o conteúdo"
            aoConfirmar={onConfirm}
            aoExcluir={onDelete}
        />,
    );
}

describe("DialogoExclusao", () => {
    it("só habilita o botão quando o slug digitado é exato", async () => {
        const user = userEvent.setup();
        build(vi.fn().mockResolvedValue(undefined));
        await waitForModalFocus();

        const confirm = screen.getByRole("button", { name: "Excluir organização" });
        expect(confirm).toBeDisabled();

        await user.type(screen.getByLabelText(/Digite/i), "loja");
        expect(confirm).toBeDisabled();

        await user.clear(screen.getByLabelText(/Digite/i));
        await user.type(screen.getByLabelText(/Digite/i), "loja-antiga");
        expect(confirm).toBeEnabled();
    });

    it("mostra a recusa por conteúdo (409) da API e libera o force", async () => {
        const user = userEvent.setup();
        const onConfirm = vi
            .fn()
            .mockRejectedValueOnce(
                Object.assign(new Error("a organização tem agentes; use force"), {
                    name: "ErroApi",
                }),
            )
            .mockResolvedValueOnce(undefined);
        build(onConfirm);
        await waitForModalFocus();

        await user.type(screen.getByLabelText(/Digite/i), "loja-antiga");
        await user.click(screen.getByRole("button", { name: "Excluir organização" }));

        expect(await screen.findByText("a organização tem agentes; use force")).toBeInTheDocument();
        expect(screen.getByRole("checkbox")).toBeInTheDocument();
    });

    it("confirma sem force, recebe 409 da API real e depois exclui com ?force=true", async () => {
        const user = userEvent.setup();
        const onDelete = vi.fn();
        const onClose = vi.fn();
        const servidor = new FakeServer()
            .responder("DELETE", "/admin/v1/orgs/org_1", (call) =>
                call.url.includes("force=true")
                    ? { status: 204 }
                    : apiError(409, "conflict", "a organização tem agentes; confirme com force"),
            )
            .instalar();

        build((forcar) => deleteOrg("org_1", forcar), onDelete, onClose);
        await waitForModalFocus();
        await user.type(screen.getByLabelText(/Digite/i), "loja-antiga");
        await user.click(screen.getByRole("button", { name: "Excluir organização" }));

        expect(
            await screen.findByText("a organização tem agentes; confirme com force"),
        ).toBeInTheDocument();

        await user.click(screen.getByRole("checkbox"));
        await user.click(screen.getByRole("button", { name: "Excluir organização" }));

        await waitFor(() => expect(onDelete).toHaveBeenCalled());
        expect(onClose).toHaveBeenCalled();
        const requests = servidor.chamadasPara("DELETE", "/admin/v1/orgs/org_1");
        expect(requests).toHaveLength(2);
        expect(requests[0]?.url).not.toContain("force=true");
        expect(requests[1]?.url).toContain("force=true");
    });
});

describe("DialogoExclusao em en-US", () => {
    afterEach(() => setLocale("pt-BR"));

    it("traduz rótulos, trava de digitação e recusa", async () => {
        const user = userEvent.setup();
        setLocale("en-US");
        const onConfirm = vi.fn().mockRejectedValueOnce(new Error("the organization has agents"));

        render(
            <DialogExclusion
                aberto
                onClose={vi.fn()}
                titulo="Delete organization?"
                descricao="The organization leaves the system."
                resource="loja-antiga"
                labelResource="organization slug"
                labelConfirm="Delete organization"
                permiteForcar
                aoConfirmar={onConfirm}
            />,
        );
        await waitForModalFocus();

        expect(screen.getByRole("dialog")).toHaveAccessibleName("Delete organization?");
        expect(screen.getByText("organization slug", { exact: false })).toBeInTheDocument();
        expect(screen.getByRole("checkbox")).toHaveAccessibleName(
            /Also delete this organization's content/,
        );
        expect(screen.getByRole("button", { name: "Delete organization" })).toBeDisabled();

        await user.type(screen.getByLabelText(/Type/i), "loja-antiga");
        expect(screen.getByRole("button", { name: "Delete organization" })).toBeEnabled();

        await user.click(screen.getByRole("button", { name: "Delete organization" }));

        expect(await screen.findByText("The deletion was refused")).toBeInTheDocument();
        expect(screen.getByText("the organization has agents")).toBeInTheDocument();
    });
});
