/**
 * Testes do formulário de organização (HR-39a): criar, editar, erro de validação e o `412` de
 * concorrência — o caso que importa de verdade, porque nele a tela não pode perder o que o
 * operador digitou.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Org } from "../api/cliente";
import { esperarFocoDoModal } from "../testes/modal";
import { ServidorFalso, erroApi } from "../testes/servidor";
import { ModalOrg } from "./ModalOrg";
import { setLocale } from "../i18n";

function orgFalsa(parcial: Partial<Org> = {}): Org {
    return {
        id: "org_1",
        name: "Loja Antiga",
        slug: "loja-antiga",
        status: "active",
        version: 3,
        config: { retention_days: 30 } as unknown as Org["config"],
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
        ...parcial,
    };
}

/** Nome acessível sem o asterisco de obrigatório (ele é `aria-hidden`). */
function campoNome(): HTMLElement {
    return screen.getByRole("textbox", { name: "Nome" });
}

function campoSlug(): HTMLElement {
    return screen.getByRole("textbox", { name: "Slug" });
}

describe("ModalOrg", () => {
    it("cria a organização enviando nome, slug e retenção", async () => {
        const usuario = userEvent.setup();
        const aoSalvar = vi.fn();
        const aoFechar = vi.fn();
        const criada = orgFalsa({ id: "org_9", name: "Loja Nova", slug: "loja-nova" });
        const servidor = new ServidorFalso()
            .responder("POST", "/admin/v1/orgs", { status: 201, corpo: criada })
            .instalar();

        render(<ModalOrg aberto org={null} aoFechar={aoFechar} aoSalvar={aoSalvar} />);
        await esperarFocoDoModal();

        await usuario.type(campoNome(), "Loja Nova");
        await usuario.type(campoSlug(), "loja-nova");
        await usuario.type(screen.getByRole("spinbutton", { name: /Retenção/ }), "7");
        await usuario.click(screen.getByRole("button", { name: "Criar organização" }));

        await waitFor(() => expect(aoSalvar).toHaveBeenCalledWith(criada));
        expect(aoFechar).toHaveBeenCalled();
        const [pedido] = servidor.chamadasPara("POST", "/admin/v1/orgs");
        expect(pedido?.corpo).toEqual({
            name: "Loja Nova",
            slug: "loja-nova",
            config: { retention_days: 7 },
        });
    });

    it("recusa nome vazio sem chamar a API", async () => {
        const usuario = userEvent.setup();
        const servidor = new ServidorFalso().instalar();

        render(
            <ModalOrg aberto org={null} aoFechar={vi.fn()} aoSalvar={vi.fn()} />,
        );
        await esperarFocoDoModal();

        await usuario.click(screen.getByRole("button", { name: "Criar organização" }));

        expect(await screen.findByText("informe o nome da organização")).toBeInTheDocument();
        expect(servidor.chamadasPara("POST", "/admin/v1/orgs")).toHaveLength(0);
    });

    it("mostra o conflito de versão, relê a organização e mantém o formulário preenchido", async () => {
        const usuario = userEvent.setup();
        const aoSalvar = vi.fn();
        let patches = 0;
        const servidor = new ServidorFalso()
            .responder("PATCH", "/admin/v1/orgs/org_1", () => {
                patches += 1;
                return patches === 1
                    ? erroApi(412, "version_conflict", "a versão informada não é a atual", {
                          expected_version: 3,
                          current_version: 4,
                      })
                    : { status: 200, corpo: orgFalsa({ name: "Loja Editada", version: 5 }) };
            })
            .responder("GET", "/admin/v1/orgs/org_1", {
                status: 200,
                corpo: orgFalsa({ name: "Loja do Outro", version: 4 }),
            })
            .instalar();

        render(
            <ModalOrg aberto org={orgFalsa()} aoFechar={vi.fn()} aoSalvar={aoSalvar} />,
        );
        await esperarFocoDoModal();

        const nome = campoNome();
        await usuario.clear(nome);
        await usuario.type(nome, "Loja Editada");
        await usuario.click(screen.getByRole("button", { name: "Salvar alterações" }));

        expect(await screen.findByText("Versão desatualizada")).toBeInTheDocument();
        expect(screen.getByText(/agora na versão 4/)).toBeInTheDocument();
        // O que o operador digitou continua lá depois da releitura.
        expect(nome).toHaveValue("Loja Editada");
        expect(servidor.chamadasPara("GET", "/admin/v1/orgs/org_1")).toHaveLength(1);
        expect(aoSalvar).not.toHaveBeenCalled();

        // O segundo salvamento já usa a versão relida (4), não a velha (3).
        await usuario.click(screen.getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() => expect(aoSalvar).toHaveBeenCalled());

        const [primeiro, segundo] = servidor.chamadasPara("PATCH", "/admin/v1/orgs/org_1");
        expect(primeiro?.cabecalhos["if-match"]).toBe("3");
        expect(segundo?.cabecalhos["if-match"]).toBe("4");
        expect(segundo?.corpo).toMatchObject({ name: "Loja Editada", status: "active" });
    });

    it("mostra o erro de validação da API no campo que ela aponta", async () => {
        const usuario = userEvent.setup();
        new ServidorFalso()
            .responder(
                "POST",
                "/admin/v1/orgs",
                erroApi(400, "validation_error", "campo `name` inválido", {
                    field: "name",
                    reason: "curto demais",
                }),
            )
            .instalar();

        render(
            <ModalOrg aberto org={null} aoFechar={vi.fn()} aoSalvar={vi.fn()} />,
        );
        await esperarFocoDoModal();

        await usuario.type(campoNome(), "X");
        await usuario.click(screen.getByRole("button", { name: "Criar organização" }));

        expect(await screen.findByText("campo `name` inválido")).toBeInTheDocument();
        expect(campoNome()).toHaveAttribute("aria-invalid", "true");
    });
});

describe("ModalOrg em en-US", () => {
    afterEach(() => setLocale("pt-BR"));

    it("mostra o formulário e a validação em inglês", async () => {
        const usuario = userEvent.setup();
        setLocale("en-US");
        new ServidorFalso().instalar();

        render(<ModalOrg aberto org={null} aoFechar={vi.fn()} aoSalvar={vi.fn()} />);
        await esperarFocoDoModal();

        expect(screen.getByRole("dialog")).toHaveAccessibleName("New organization");
        expect(screen.getByRole("textbox", { name: "Name" })).toBeInTheDocument();
        expect(screen.getByRole("textbox", { name: "Slug" })).toBeInTheDocument();
        expect(screen.getByRole("spinbutton", { name: /retention/i })).toBeInTheDocument();

        await usuario.click(screen.getByRole("button", { name: "Create organization" }));

        expect(await screen.findByText("enter the organization name")).toBeInTheDocument();
    });

    it("troca o idioma do formulário aberto sem recarregar", async () => {
        setLocale("pt-BR");
        new ServidorFalso().instalar();

        render(<ModalOrg aberto org={orgFalsa()} aoFechar={vi.fn()} aoSalvar={vi.fn()} />);
        await esperarFocoDoModal();

        expect(screen.getByRole("dialog")).toHaveAccessibleName("Editar organização");

        setLocale("en-US");

        expect(await screen.findByRole("dialog", { name: "Edit organization" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Save changes" })).toBeInTheDocument();
    });
});
