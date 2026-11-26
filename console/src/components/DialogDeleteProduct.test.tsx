
import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Product } from "../api/products";
import { setLocale } from "../i18n";
import { waitForModalFocus } from "../testing/modal";
import { FakeServer, apiError } from "../testing/server";
import { DialogDeleteProduct } from "../components/DialogDeleteProduct";

const ORG = "org_1";
const PRODUCT: Product = {
    id: "prod_1",
    org_id: ORG,
    category_id: "cat_1",
    name: "Clareamento dental",
    slug: "clareamento-dental",
    aliases: [],
    description: "",
    price_text: "R$ 450,00",
    price_cents: 45000,
    unit: "",
    includes: "",
    excludes: "",
    conditions: "",
    tags: [],
    status: "active",
    version: 2,
    created_at: 0,
    updated_at: 0,
};

function build(agentsSee: number | null, onDelete = vi.fn(), onClose = vi.fn()) {
    render(
        <DialogDeleteProduct
            aberto
            onClose={onClose}
            org={ORG}
            produto={PRODUCT}
            agentsSee={agentsSee}
            aoExcluir={onDelete}
        />,
    );
    return { onDelete, onClose };
}

function refusalCart(items: number, carts: number) {
    return apiError(
        409,
        "conflict",
        `o produto está em ${items} item(ns) de ${carts} carrinho(s); excluir faz esses itens ficarem como "produto removido do catálogo" (o preço gravado continua). Para confirmar, repita com force=true`,
        { itens_de_carrinho: items, carrinhos: carts },
    );
}

describe("DialogoExcluirProduto", () => {
    it("diz quantos agentes deixam de oferecer o produto e só libera com o nome exato", async () => {
        const user = userEvent.setup();
        build(2);
        await waitForModalFocus();

        expect(screen.getByText(/2 agentes oferecem este produto/)).toBeInTheDocument();
        const confirm = screen.getByRole("button", { name: "Excluir produto" });
        expect(confirm).toBeDisabled();

        await user.type(screen.getByLabelText(/Digite/i), "Clareamento");
        expect(confirm).toBeDisabled();

        await user.clear(screen.getByLabelText(/Digite/i));
        await user.type(screen.getByLabelText(/Digite/i), "Clareamento dental");
        expect(confirm).toBeEnabled();
    });

    it("sem agentes que o oferecem, diz isso em vez de contar", async () => {
        build(0);
        expect(await screen.findByText("Nenhum agente oferece este produto hoje.")).toBeInTheDocument();
    });

    it("se a leitura dos agentes falhou, avisa em vez de ficar carregando", async () => {
        build(null);
        expect(await screen.findByText(/Não consegui contar os agentes/)).toBeInTheDocument();
    });

    it("manda o DELETE sem force, chama aoExcluir com o aviso do índice e fecha", async () => {
        const user = userEvent.setup();
        const servidor = new FakeServer()
            .responder("DELETE", `/admin/v1/orgs/${ORG}/products/prod_1`, {
                status: 200,
                corpo: {
                    deleted: true,
                    index_warning: "índice de produtos não atualizado; execute product reindex",
                },
            })
            .instalar();
        const { onDelete } = build(1);
        await waitForModalFocus();

        await user.type(screen.getByLabelText(/Digite/i), "Clareamento dental");
        await user.click(screen.getByRole("button", { name: "Excluir produto" }));

        await waitFor(() =>
            expect(onDelete).toHaveBeenCalledWith(
                "índice de produtos não atualizado; execute product reindex",
            ),
        );
        const [request] = servidor.chamadasPara("DELETE", "/products/prod_1");
        expect(request?.url).not.toContain("force");
    });

    it("quando a API recusa, mostra o motivo real e não fecha", async () => {
        const user = userEvent.setup();
        new FakeServer()
            .responder(
                "DELETE",
                `/admin/v1/orgs/${ORG}/products/prod_1`,
                apiError(404, "not_found", "produto `prod_1` não encontrado"),
            )
            .instalar();
        const { onDelete } = build(0);
        await waitForModalFocus();

        await user.type(screen.getByLabelText(/Digite/i), "Clareamento dental");
        await user.click(screen.getByRole("button", { name: "Excluir produto" }));

        expect(await screen.findByText("produto `prod_1` não encontrado")).toBeInTheDocument();
        expect(screen.getByText("código: not_found")).toBeInTheDocument();
        expect(onDelete).not.toHaveBeenCalled();
    });

    it("com itens de carrinho, a 1ª tentativa sai sem force; o 409 mostra a contagem, não apaga e libera a confirmação extra", async () => {
        const user = userEvent.setup();
        const servidor = new FakeServer()
            .responder("DELETE", `/admin/v1/orgs/${ORG}/products/prod_1`, (call) =>
                call.url.includes("force=true")
                    ? { status: 200, corpo: { deleted: true, index_warning: null } }
                    : refusalCart(3, 2),
            )
            .instalar();
        const { onDelete, onClose } = build(1);
        const dialog = await screen.findByRole("dialog", { name: "Excluir produto?" });
        await waitForModalFocus();
        expect(within(dialog).queryByRole("checkbox")).toBeNull();

        await user.type(screen.getByLabelText(/Digite/i), "Clareamento dental");
        await user.click(screen.getByRole("button", { name: "Excluir produto" }));

        expect(await within(dialog).findByText(/o produto está em 3 item\(ns\) de 2 carrinho\(s\)/)).toBeInTheDocument();
        expect(within(dialog).getByText("código: conflict")).toBeInTheDocument();
        expect(
            within(dialog).getByText(/Este produto está em 3 itens de 2 carrinhos/),
        ).toBeInTheDocument();
        expect(onDelete).not.toHaveBeenCalled();
        expect(onClose).not.toHaveBeenCalled();
        expect(dialog).toBeInTheDocument();
        expect(servidor.chamadasPara("DELETE", "/products/prod_1")[0]?.url).not.toContain("force");

        const extra = within(dialog).getByRole("checkbox", {
            name: /Excluir mesmo assim: 3 itens de carrinho/,
        });
        await user.click(extra);
        await user.click(screen.getByRole("button", { name: "Excluir produto" }));

        await waitFor(() => expect(onDelete).toHaveBeenCalledWith(null));
        expect(onClose).toHaveBeenCalled();
        const requests = servidor.chamadasPara("DELETE", "/products/prod_1");
        expect(requests).toHaveLength(2);
        expect(requests.at(-1)?.url).toContain("force=true");
    });

    it("no segundo passo, Esc fecha sem excluir", async () => {
        const user = userEvent.setup();
        const servidor = new FakeServer()
            .responder("DELETE", `/admin/v1/orgs/${ORG}/products/prod_1`, refusalCart(1, 1))
            .instalar();
        const { onDelete, onClose } = build(0);
        await waitForModalFocus();

        await user.type(screen.getByLabelText(/Digite/i), "Clareamento dental");
        await user.click(screen.getByRole("button", { name: "Excluir produto" }));
        expect(await screen.findByText(/Este produto está em 1 item de 1 carrinho/)).toBeInTheDocument();
        expect(screen.getByRole("checkbox", { name: /Excluir mesmo assim/ })).toBeInTheDocument();

        await user.keyboard("{Escape}");
        await waitFor(() => expect(onClose).toHaveBeenCalled());
        expect(onDelete).not.toHaveBeenCalled();
        expect(servidor.chamadasPara("DELETE", "/products/prod_1")).toHaveLength(1);
    });

    it("renders consequences and 409 cart count in en-US", async () => {
        const user = userEvent.setup();
        setLocale("en-US");
        new FakeServer()
            .responder("DELETE", `/admin/v1/orgs/${ORG}/products/prod_1`, refusalCart(3, 2))
            .instalar();
        build(2);
        await waitForModalFocus();

        expect(screen.getByRole("dialog", { name: "Delete product?" })).toBeInTheDocument();
        expect(screen.getByText(/2 agents offer this product/)).toBeInTheDocument();

        await user.type(screen.getByLabelText(/Clareamento dental|product name/i), "Clareamento dental");
        await user.click(screen.getByRole("button", { name: "Delete product" }));
        expect(
            await screen.findByText(/This product is in 3 items across 2 carts/),
        ).toBeInTheDocument();
        expect(
            screen.getByRole("checkbox", {
                name: /Delete anyway: 3 cart items will remain as "product removed from catalog"/,
            }),
        ).toBeInTheDocument();
    });
});

afterEach(() => {
    setLocale("pt-BR");
});
