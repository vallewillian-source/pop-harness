/**
 * Testes do formulário de produto (HR-42).
 *
 * Cobre criar (com o corpo que sai para a API: preço em centavos, listas separadas por vírgula e
 * sem `status`), validação antes da chamada, erro de campo vindo da API, e o `412` de concorrência,
 * que precisa manter o que foi digitado e usar a versão nova no salvamento seguinte.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Product } from "../api/products";
import { setLocale } from "../i18n";
import { waitForModalFocus } from "../testing/modal";
import { FakeServer, apiError } from "../testing/server";
import { ModalProduto, centavosDe, formatarReais, listaDe, reaisDe } from "./ModalProduto";

const ORG = "org_1";
const CATEGORIAS = [
    { id: "cat_1", org_id: ORG, name: "Ortodontia", description: "", status: "active", version: 1, created_at: 0, updated_at: 0 },
    { id: "cat_2", org_id: ORG, name: "Implantes", description: "", status: "active", version: 1, created_at: 0, updated_at: 0 },
];

function produtoBase(sobrescrito: Partial<Product> = {}): Product {
    return {
        id: "prod_1",
        org_id: ORG,
        category_id: "cat_1",
        name: "Clareamento dental",
        slug: "clareamento-dental",
        aliases: ["clareamento"],
        description: "",
        price_text: "R$ 450,00",
        price_cents: 45000,
        unit: "por sessão",
        includes: "",
        excludes: "",
        conditions: "",
        tags: ["estetica"],
        status: "active",
        version: 2,
        created_at: 1_700_000_000_000,
        updated_at: 1_700_000_000_000,
        ...sobrescrito,
    };
}

function campo(nome: string): HTMLElement {
    return screen.getByRole("textbox", { name: nome });
}

function montar(props: Partial<React.ComponentProps<typeof ModalProduto>> = {}) {
    const aoSalvar = vi.fn();
    const aoFechar = vi.fn();
    render(
        <ModalProduto
            org={ORG}
            aberto
            produto={null}
            categorias={CATEGORIAS}
            aoFechar={aoFechar}
            aoSalvar={aoSalvar}
            {...props}
        />,
    );
    return { aoSalvar, aoFechar };
}

describe("conversão de preço e listas", () => {
    it("aceita 450, 450,00, 1.234,50 e R$ 450 e devolve centavos", () => {
        expect(centavosDe("450")).toBe(45000);
        expect(centavosDe("450,00")).toBe(45000);
        expect(centavosDe("1.234,50")).toBe(123450);
        expect(centavosDe("R$ 450")).toBe(45000);
        expect(centavosDe("")).toBe(0);
        expect(centavosDe("abc")).toBeNull();
        expect(centavosDe("1,234")).toBeNull();
    });

    it("mostra centavos em reais com vírgula e separa listas por vírgula", () => {
        expect(reaisDe(45000)).toBe("450,00");
        expect(listaDe(" a, ,b ,, c ")).toEqual(["a", "b", "c"]);
    });

    it("formata o texto calculado no padrão do servidor (com separador de milhar)", () => {
        expect(formatarReais(890000)).toBe("R$ 8.900,00");
        expect(formatarReais(245000)).toBe("R$ 2.450,00");
        expect(formatarReais(18000)).toBe("R$ 180,00");
        expect(formatarReais(0)).toBe("R$ 0,00");
        expect(formatarReais(99)).toBe("R$ 0,99");
        expect(formatarReais(100000000)).toBe("R$ 1.000.000,00");
    });
});

describe("ModalProduto", () => {
    it("cria produto com o corpo da API (centavos, listas, sem status e sem slug vazio)", async () => {
        const usuario = userEvent.setup();
        const criado = produtoBase({ id: "prod_9", name: "Implante unitário", price_cents: 320000 });
        const servidor = new FakeServer()
            .responder("POST", `/admin/v1/orgs/${ORG}/products`, { status: 201, corpo: criado })
            .instalar();
        const { aoSalvar, aoFechar } = montar();
        await waitForModalFocus();

        await usuario.selectOptions(screen.getByRole("combobox", { name: "Categoria" }), "cat_2");
        await usuario.type(campo("Nome"), "Implante unitário");
        await usuario.type(campo("Outros nomes"), "implante, pino, implante simples");
        await usuario.type(campo("Preço (R$)"), "3.200,00");
        await usuario.type(campo("Etiquetas de busca"), "cirurgia");
        await usuario.click(screen.getByRole("button", { name: "Criar produto" }));

        await waitFor(() => expect(aoSalvar).toHaveBeenCalledWith(criado));
        expect(aoFechar).toHaveBeenCalled();
        const [pedido] = servidor.chamadasPara("POST", "/products");
        expect(pedido?.corpo).toEqual({
            category_id: "cat_2",
            name: "Implante unitário",
            aliases: ["implante", "pino", "implante simples"],
            description: "",
            price_text: "",
            price_cents: 320000,
            unit: "",
            includes: "",
            excludes: "",
            conditions: "",
            tags: ["cirurgia"],
        });
        expect(pedido?.corpo).not.toHaveProperty("status");
        expect(pedido?.corpo).not.toHaveProperty("slug");
    });

    it("na criação não existe o campo Situação, que a API recusa", async () => {
        new FakeServer().instalar();
        montar();
        expect(await screen.findByRole("dialog", { name: "Novo produto" })).toBeInTheDocument();
        expect(screen.queryByRole("combobox", { name: "Situação" })).not.toBeInTheDocument();
    });

    it("sem categoria, nome vazio e preço inválido não chamam a API e marcam os campos", async () => {
        const usuario = userEvent.setup();
        const servidor = new FakeServer().instalar();
        montar();
        await waitForModalFocus();

        await usuario.type(campo("Preço (R$)"), "abc");
        await usuario.click(screen.getByRole("button", { name: "Criar produto" }));

        expect(await screen.findByText("escolha a categoria do produto")).toBeInTheDocument();
        expect(screen.getByText("informe o nome do produto")).toBeInTheDocument();
        expect(screen.getByText("use o formato 450 ou 450,00 (valor em reais)")).toBeInTheDocument();
        expect(servidor.chamadas).toHaveLength(0);
    });

    it("erro de campo vindo da API (slug repetido) aparece no campo Identificador", async () => {
        const usuario = userEvent.setup();
        new FakeServer()
            .responder(
                "POST",
                `/admin/v1/orgs/${ORG}/products`,
                apiError(400, "validation_error", "campo `slug` inválido", {
                    field: "slug",
                    reason: "já existe um produto com esse identificador nesta org",
                }),
            )
            .instalar();
        montar();
        await waitForModalFocus();

        await usuario.selectOptions(screen.getByRole("combobox", { name: "Categoria" }), "cat_1");
        await usuario.type(campo("Nome"), "Clareamento");
        await usuario.type(campo("Identificador (slug)"), "clareamento-dental");
        await usuario.click(screen.getByRole("button", { name: "Criar produto" }));

        expect(
            await screen.findByText("já existe um produto com esse identificador nesta org"),
        ).toBeInTheDocument();
    });

    it("editar envia If-Match e a situação; no 412 relê, mantém o que foi digitado e salva com a versão nova", async () => {
        const usuario = userEvent.setup();
        const aoSalvar = vi.fn();
        let tentativa = 0;
        const servidor = new FakeServer()
            .responder("PATCH", `/admin/v1/orgs/${ORG}/products/prod_1`, () => {
                tentativa += 1;
                if (tentativa === 1) {
                    return apiError(412, "version_conflict", "produto mudou desde a leitura", {
                        current_version: 3,
                        expected_version: 2,
                    });
                }
                return { status: 200, corpo: produtoBase({ version: 4, name: "Clareamento caseiro" }) };
            })
            .responder("GET", `/admin/v1/orgs/${ORG}/products/prod_1`, {
                status: 200,
                corpo: produtoBase({ version: 3, price_cents: 50000, price_text: "R$ 500,00" }),
            })
            .instalar();
        montar({ produto: produtoBase(), aoSalvar });
        await waitForModalFocus();

        expect(screen.queryByRole("combobox", { name: "Situação" })).toBeInTheDocument();
        await usuario.clear(campo("Nome"));
        await usuario.type(campo("Nome"), "Clareamento caseiro");
        await usuario.click(screen.getByRole("button", { name: "Salvar alterações" }));

        expect(await screen.findByText(/Outra pessoa salvou este produto/)).toBeInTheDocument();
        expect(screen.getByText(/agora na versão 3/)).toBeInTheDocument();
        expect(campo("Nome")).toHaveValue("Clareamento caseiro");

        await usuario.click(screen.getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() => expect(aoSalvar).toHaveBeenCalled());

        const patches = servidor.chamadasPara("PATCH", "/products/prod_1");
        expect(patches).toHaveLength(2);
        expect(patches[0]?.cabecalhos["if-match"]).toBe("2");
        expect(patches[1]?.cabecalhos["if-match"]).toBe("3");
        expect(patches[1]?.corpo).toMatchObject({
            name: "Clareamento caseiro",
            price_cents: 45000,
            status: "active",
        });
        expect(patches[1]?.corpo).not.toHaveProperty("slug");
        // O 412 trouxe o preço gravado por outra pessoa (R$ 500,00). O operador manteve R$ 450,00,
        // que é diferente do atual: o texto volta a ser calculado, e o cliente não vê o valor antigo.
        expect(patches[1]?.corpo).toMatchObject({ price_text: "" });
    });
});

describe("preço como o cliente vê", () => {
    const URL_PRODUTO = `/admin/v1/orgs/${ORG}/products/prod_1`;

    it("editar sem mexer no preço não envia o texto e mostra o que o cliente vê", async () => {
        const usuario = userEvent.setup();
        const servidor = new FakeServer()
            .responder("PATCH", URL_PRODUTO, { status: 200, corpo: produtoBase({ version: 3 }) })
            .instalar();
        montar({ produto: produtoBase() });
        await waitForModalFocus();

        expect(campo("Preço como o cliente vê")).toHaveValue("");
        expect(screen.getByText(/Como o cliente vê: R\$ 450,00\./)).toBeInTheDocument();

        await usuario.click(screen.getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() => expect(servidor.chamadasPara("PATCH", "/products/prod_1")).toHaveLength(1));

        const [patch] = servidor.chamadasPara("PATCH", "/products/prod_1");
        expect(patch?.corpo).not.toHaveProperty("price_text");
        expect(patch?.corpo).toMatchObject({ price_cents: 45000 });
    });

    it("mudar só o valor em centavos envia o texto vazio, a prévia acompanha e a lista recebe o preço novo", async () => {
        const usuario = userEvent.setup();
        const aoSalvar = vi.fn();
        const servidor = new FakeServer()
            .responder("PATCH", URL_PRODUTO, {
                status: 200,
                corpo: produtoBase({ version: 3, price_cents: 50000, price_text: "R$ 500,00" }),
            })
            .instalar();
        montar({ produto: produtoBase(), aoSalvar });
        await waitForModalFocus();

        await usuario.clear(campo("Preço (R$)"));
        await usuario.type(campo("Preço (R$)"), "500");
        expect(screen.getByText(/Como o cliente vê: R\$ 500,00\./)).toBeInTheDocument();
        expect(screen.getByText(/O preço mudou/)).toBeInTheDocument();

        await usuario.click(screen.getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() => expect(aoSalvar).toHaveBeenCalled());

        const [patch] = servidor.chamadasPara("PATCH", "/products/prod_1");
        expect(patch?.corpo).toMatchObject({ price_cents: 50000, price_text: "" });
        expect(aoSalvar).toHaveBeenCalledWith(expect.objectContaining({ price_text: "R$ 500,00" }));
    });

    it("texto digitado pelo operador vai no corpo como foi digitado", async () => {
        const usuario = userEvent.setup();
        const servidor = new FakeServer()
            .responder("PATCH", URL_PRODUTO, { status: 200, corpo: produtoBase({ version: 3 }) })
            .instalar();
        montar({ produto: produtoBase({ price_cents: 32000, price_text: "R$ 320,00" }) });
        await waitForModalFocus();

        await usuario.type(campo("Preço como o cliente vê"), "R$ 320,00 por visita");
        expect(screen.getByText(/Como o cliente vê: R\$ 320,00 por visita\./)).toBeInTheDocument();

        await usuario.click(screen.getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() => expect(servidor.chamadasPara("PATCH", "/products/prod_1")).toHaveLength(1));

        const [patch] = servidor.chamadasPara("PATCH", "/products/prod_1");
        expect(patch?.corpo).toMatchObject({ price_cents: 32000, price_text: "R$ 320,00 por visita" });
    });

    it("apagar o texto digitado envia vazio e a prévia volta ao valor calculado", async () => {
        const usuario = userEvent.setup();
        const servidor = new FakeServer()
            .responder("PATCH", URL_PRODUTO, { status: 200, corpo: produtoBase({ version: 3 }) })
            .instalar();
        montar({ produto: produtoBase({ price_cents: 32000, price_text: "R$ 320,00" }) });
        await waitForModalFocus();

        await usuario.type(campo("Preço como o cliente vê"), "x");
        await usuario.clear(campo("Preço como o cliente vê"));
        expect(screen.getByText(/Como o cliente vê: R\$ 320,00\./)).toBeInTheDocument();

        await usuario.click(screen.getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() => expect(servidor.chamadasPara("PATCH", "/products/prod_1")).toHaveLength(1));

        const [patch] = servidor.chamadasPara("PATCH", "/products/prod_1");
        expect(patch?.corpo).toMatchObject({ price_text: "" });
    });

    it("com texto próprio gravado, o botão volta o texto a ser calculado pelo valor", async () => {
        const usuario = userEvent.setup();
        const servidor = new FakeServer()
            .responder("PATCH", URL_PRODUTO, { status: 200, corpo: produtoBase({ version: 3 }) })
            .instalar();
        montar({ produto: produtoBase({ price_cents: 32000, price_text: "R$ 320,00 por visita" }) });
        await waitForModalFocus();

        expect(screen.getByText(/Como o cliente vê: R\$ 320,00 por visita\./)).toBeInTheDocument();
        await usuario.click(screen.getByRole("button", { name: "Voltar a calcular o texto pelo valor" }));
        expect(screen.getByText(/Como o cliente vê: R\$ 320,00\./)).toBeInTheDocument();
        expect(
            screen.queryByRole("button", { name: "Voltar a calcular o texto pelo valor" }),
        ).not.toBeInTheDocument();

        await usuario.click(screen.getByRole("button", { name: "Salvar alterações" }));
        await waitFor(() => expect(servidor.chamadasPara("PATCH", "/products/prod_1")).toHaveLength(1));
        expect(servidor.chamadasPara("PATCH", "/products/prod_1")[0]?.corpo).toMatchObject({
            price_text: "",
        });
    });

    it("renders fields, statuses, and validation in en-US", async () => {
        const usuario = userEvent.setup();
        setLocale("en-US");
        montar();
        await waitForModalFocus();

        expect(screen.getByRole("heading", { name: "New product" })).toBeInTheDocument();
        expect(screen.getByRole("option", { name: "Choose a category" })).toBeInTheDocument();
        await usuario.click(screen.getByRole("button", { name: "Create product" }));
        expect(await screen.findByText("choose the product category")).toBeInTheDocument();
        expect(screen.getByText("enter the product name")).toBeInTheDocument();

        act(() => {
            setLocale("pt-BR");
        });
        expect(await screen.findByRole("heading", { name: "Novo produto" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Criar produto" })).toBeInTheDocument();
    });
});

afterEach(() => {
    setLocale("pt-BR");
});
