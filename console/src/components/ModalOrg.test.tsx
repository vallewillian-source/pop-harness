
import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Org } from "../api/client";
import { waitForModalFocus } from "../testing/modal";
import { FakeServer, apiError } from "../testing/server";
import { ModalOrg } from "../components/ModalOrg";
import { setLocale } from "../i18n";

function orgFalse(partial: Partial<Org> = {}): Org {
    return {
        id: "org_1",
        name: "Loja Antiga",
        slug: "loja-antiga",
        status: "active",
        version: 3,
        config: { retention_days: 30 } as unknown as Org["config"],
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
        ...partial,
    };
}

function fieldName(): HTMLElement {
    return screen.getByRole("textbox", { name: "Nome" });
}

function fieldSlug(): HTMLElement {
    return screen.getByRole("textbox", { name: "Slug" });
}

describe("ModalOrg", () => {
    it("cria a organização enviando nome, slug e retenção", async () => {
        const user = userEvent.setup();
        const onSave = vi.fn();
        const onClose = vi.fn();
        const created = orgFalse({ id: "org_9", name: "Loja Nova", slug: "loja-nova" });
        const servidor = new FakeServer()
            .respond("POST", "/admin/v1/orgs", { status: 201, body: created })
            .install();

        render(<ModalOrg opened org={null} onClose={onClose} onSave={onSave} />);
        await waitForModalFocus();

        await user.type(fieldName(), "Loja Nova");
        await user.type(fieldSlug(), "loja-nova");
        await user.type(screen.getByRole("spinbutton", { name: /Retenção/ }), "7");
        await user.click(screen.getByRole("button", { name: "Criar organização" }));

        await waitFor(() => expect(onSave).toHaveBeenCalledWith(created));
        expect(onClose).toHaveBeenCalled();
        const [request] = servidor.callsFor("POST", "/admin/v1/orgs");
        expect(request?.body).toEqual({
            name: "Loja Nova",
            slug: "loja-nova",
            config: { retention_days: 7 },
        });
    });

    it("recusa nome vazio sem chamar a API", async () => {
        const user = userEvent.setup();
        const servidor = new FakeServer().install();

        render(
            <ModalOrg opened org={null} onClose={vi.fn()} onSave={vi.fn()} />,
        );
        await waitForModalFocus();

        await user.click(screen.getByRole("button", { name: "Criar organização" }));

        expect(await screen.findByText("informe o nome da organização")).toBeInTheDocument();
        expect(servidor.callsFor("POST", "/admin/v1/orgs")).toHaveLength(0);
    });

    it("mostra o conflito de versão, relê a organização e mantém o formulário preenchido", async () => {
        const user = userEvent.setup();
        const onSave = vi.fn();
        let patches = 0;
        const servidor = new FakeServer()
            .respond("PATCH", "/admin/v1/orgs/org_1", () => {
                patches += 1;
                return patches === 1
                    ? apiError(412, "version_conflict", "a versão informada não é a atual", {
                          expected_version: 3,
                          current_version: 4,
                      })
                    : { status: 200, body: orgFalse({ name: "Loja Editada", version: 5 }) };
            })
            .respond("GET", "/admin/v1/orgs/org_1", {
                status: 200,
                body: orgFalse({ name: "Loja do Outro", version: 4 }),
            })
            .install();

        render(
            <ModalOrg opened org={orgFalse()} onClose={vi.fn()} onSave={onSave} />,
        );
        await waitForModalFocus();

        const nome = fieldName();
        await user.clear(nome);
        await user.type(nome, "Loja Editada");
        await user.click(screen.getByRole("button", { name: "Salvar alterações" }));

        expect(await screen.findByText("Versão desatualizada")).toBeInTheDocument();
        expect(screen.getByText(/agora na versão 4/)).toBeInTheDocument();
        expect(nome).toHaveValue("Loja Editada");
        expect(servidor.callsFor("GET", "/admin/v1/orgs/org_1")).toHaveLength(1);
        expect(onSave).not.toHaveBeenCalled();

        await user.click(screen.getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() => expect(onSave).toHaveBeenCalled());

        const [first, second] = servidor.callsFor("PATCH", "/admin/v1/orgs/org_1");
        expect(first?.headers["if-match"]).toBe("3");
        expect(second?.headers["if-match"]).toBe("4");
        expect(second?.body).toMatchObject({ name: "Loja Editada", status: "active" });
    });

    it("mostra o erro de validação da API no campo que ela aponta", async () => {
        const user = userEvent.setup();
        new FakeServer()
            .respond(
                "POST",
                "/admin/v1/orgs",
                apiError(400, "validation_error", "campo `name` inválido", {
                    field: "name",
                    reason: "curto demais",
                }),
            )
            .install();

        render(
            <ModalOrg opened org={null} onClose={vi.fn()} onSave={vi.fn()} />,
        );
        await waitForModalFocus();

        await user.type(fieldName(), "X");
        await user.click(screen.getByRole("button", { name: "Criar organização" }));

        expect(await screen.findByText("campo `name` inválido")).toBeInTheDocument();
        expect(fieldName()).toHaveAttribute("aria-invalid", "true");
    });
});

describe("ModalOrg em en-US", () => {
    afterEach(() => setLocale("pt-BR"));

    it("mostra o formulário e a validação em inglês", async () => {
        const user = userEvent.setup();
        setLocale("en-US");
        new FakeServer().install();

        render(<ModalOrg opened org={null} onClose={vi.fn()} onSave={vi.fn()} />);
        await waitForModalFocus();

        expect(screen.getByRole("dialog")).toHaveAccessibleName("New organization");
        expect(screen.getByRole("textbox", { name: "Name" })).toBeInTheDocument();
        expect(screen.getByRole("textbox", { name: "Slug" })).toBeInTheDocument();
        expect(screen.getByRole("spinbutton", { name: /retention/i })).toBeInTheDocument();

        await user.click(screen.getByRole("button", { name: "Create organization" }));

        expect(await screen.findByText("enter the organization name")).toBeInTheDocument();
    });

    it("troca o idioma do formulário aberto sem recarregar", async () => {
        setLocale("pt-BR");
        new FakeServer().install();

        render(<ModalOrg opened org={orgFalse()} onClose={vi.fn()} onSave={vi.fn()} />);
        await waitForModalFocus();

        expect(screen.getByRole("dialog")).toHaveAccessibleName("Editar organização");

        setLocale("en-US");

        expect(await screen.findByRole("dialog", { name: "Edit organization" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Save changes" })).toBeInTheDocument();
    });
});
