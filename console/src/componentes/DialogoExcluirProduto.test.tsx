/**
 * Testes da exclusão de produto (HR-42): a consequência escrita antes do botão, a trava por nome
 * digitado, o `DELETE` sem `force` (a API não pede) e o motivo real quando a API recusa.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Product } from "../api/products";
import { setLocale } from "../i18n";
import { waitForModalFocus } from "../testing/modal";
import { FakeServer, apiError } from "../testing/server";
import { DialogoExcluirProduto } from "./DialogoExcluirProduto";

const ORG = "org_1";
const PRODUTO: Product = {
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

function montar(agentesQueVeem: number | null, aoExcluir = vi.fn(), aoFechar = vi.fn()) {
    render(
        <DialogoExcluirProduto
            aberto
            aoFechar={aoFechar}
            org={ORG}
            produto={PRODUTO}
            agentesQueVeem={agentesQueVeem}
            aoExcluir={aoExcluir}
        />,
    );
    return { aoExcluir, aoFechar };
}

/** Recusa `409` da API quando o produto está em itens de carrinho (HR-42f), com a contagem. */
function recusaDeCarrinho(itens: number, carrinhos: number) {
    return apiError(
        409,
        "conflict",
        `o produto está em ${itens} item(ns) de ${carrinhos} carrinho(s); excluir faz esses itens ficarem como "produto removido do catálogo" (o preço gravado continua). Para confirmar, repita com force=true`,
        { itens_de_carrinho: itens, carrinhos },
    );
}

describe("DialogoExcluirProduto", () => {
    it("diz quantos agentes deixam de oferecer o produto e só libera com o nome exato", async () => {
        const usuario = userEvent.setup();
        montar(2);
        await waitForModalFocus();

        expect(screen.getByText(/2 agentes oferecem este produto/)).toBeInTheDocument();
        const confirmar = screen.getByRole("button", { name: "Excluir produto" });
        expect(confirmar).toBeDisabled();

        await usuario.type(screen.getByLabelText(/Digite/i), "Clareamento");
        expect(confirmar).toBeDisabled();

        await usuario.clear(screen.getByLabelText(/Digite/i));
        await usuario.type(screen.getByLabelText(/Digite/i), "Clareamento dental");
        expect(confirmar).toBeEnabled();
    });

    it("sem agentes que o oferecem, diz isso em vez de contar", async () => {
        montar(0);
        expect(await screen.findByText("Nenhum agente oferece este produto hoje.")).toBeInTheDocument();
    });

    it("se a leitura dos agentes falhou, avisa em vez de ficar carregando", async () => {
        montar(null);
        expect(await screen.findByText(/Não consegui contar os agentes/)).toBeInTheDocument();
    });

    it("manda o DELETE sem force, chama aoExcluir com o aviso do índice e fecha", async () => {
        const usuario = userEvent.setup();
        const servidor = new FakeServer()
            .responder("DELETE", `/admin/v1/orgs/${ORG}/products/prod_1`, {
                status: 200,
                corpo: {
                    deleted: true,
                    index_warning: "índice de produtos não atualizado; execute product reindex",
                },
            })
            .instalar();
        const { aoExcluir } = montar(1);
        await waitForModalFocus();

        await usuario.type(screen.getByLabelText(/Digite/i), "Clareamento dental");
        await usuario.click(screen.getByRole("button", { name: "Excluir produto" }));

        await waitFor(() =>
            expect(aoExcluir).toHaveBeenCalledWith(
                "índice de produtos não atualizado; execute product reindex",
            ),
        );
        const [pedido] = servidor.chamadasPara("DELETE", "/products/prod_1");
        expect(pedido?.url).not.toContain("force");
    });

    it("quando a API recusa, mostra o motivo real e não fecha", async () => {
        const usuario = userEvent.setup();
        new FakeServer()
            .responder(
                "DELETE",
                `/admin/v1/orgs/${ORG}/products/prod_1`,
                apiError(404, "not_found", "produto `prod_1` não encontrado"),
            )
            .instalar();
        const { aoExcluir } = montar(0);
        await waitForModalFocus();

        await usuario.type(screen.getByLabelText(/Digite/i), "Clareamento dental");
        await usuario.click(screen.getByRole("button", { name: "Excluir produto" }));

        expect(await screen.findByText("produto `prod_1` não encontrado")).toBeInTheDocument();
        expect(screen.getByText("código: not_found")).toBeInTheDocument();
        expect(aoExcluir).not.toHaveBeenCalled();
    });

    it("com itens de carrinho, a 1ª tentativa sai sem force; o 409 mostra a contagem, não apaga e libera a confirmação extra", async () => {
        const usuario = userEvent.setup();
        const servidor = new FakeServer()
            .responder("DELETE", `/admin/v1/orgs/${ORG}/products/prod_1`, (chamada) =>
                chamada.url.includes("force=true")
                    ? { status: 200, corpo: { deleted: true, index_warning: null } }
                    : recusaDeCarrinho(3, 2),
            )
            .instalar();
        const { aoExcluir, aoFechar } = montar(1);
        const dialogo = await screen.findByRole("dialog", { name: "Excluir produto?" });
        await waitForModalFocus();
        // Antes de qualquer recusa, a confirmação extra não aparece.
        expect(within(dialogo).queryByRole("checkbox")).toBeNull();

        await usuario.type(screen.getByLabelText(/Digite/i), "Clareamento dental");
        await usuario.click(screen.getByRole("button", { name: "Excluir produto" }));

        // A recusa aparece com o motivo real e com a contagem em linguagem comum.
        expect(await within(dialogo).findByText(/o produto está em 3 item\(ns\) de 2 carrinho\(s\)/)).toBeInTheDocument();
        expect(within(dialogo).getByText("código: conflict")).toBeInTheDocument();
        expect(
            within(dialogo).getByText(/Este produto está em 3 itens de 2 carrinhos/),
        ).toBeInTheDocument();
        expect(aoExcluir).not.toHaveBeenCalled();
        expect(aoFechar).not.toHaveBeenCalled();
        expect(dialogo).toBeInTheDocument();
        expect(servidor.chamadasPara("DELETE", "/products/prod_1")[0]?.url).not.toContain("force");

        // A confirmação extra aparece com a contagem; marcada, o próximo envio vai com force=true.
        const extra = within(dialogo).getByRole("checkbox", {
            name: /Excluir mesmo assim: 3 itens de carrinho/,
        });
        await usuario.click(extra);
        await usuario.click(screen.getByRole("button", { name: "Excluir produto" }));

        await waitFor(() => expect(aoExcluir).toHaveBeenCalledWith(null));
        expect(aoFechar).toHaveBeenCalled();
        const pedidos = servidor.chamadasPara("DELETE", "/products/prod_1");
        expect(pedidos).toHaveLength(2);
        expect(pedidos.at(-1)?.url).toContain("force=true");
    });

    it("no segundo passo, Esc fecha sem excluir", async () => {
        const usuario = userEvent.setup();
        const servidor = new FakeServer()
            .responder("DELETE", `/admin/v1/orgs/${ORG}/products/prod_1`, recusaDeCarrinho(1, 1))
            .instalar();
        const { aoExcluir, aoFechar } = montar(0);
        await waitForModalFocus();

        await usuario.type(screen.getByLabelText(/Digite/i), "Clareamento dental");
        await usuario.click(screen.getByRole("button", { name: "Excluir produto" }));
        expect(await screen.findByText(/Este produto está em 1 item de 1 carrinho/)).toBeInTheDocument();
        expect(screen.getByRole("checkbox", { name: /Excluir mesmo assim/ })).toBeInTheDocument();

        await usuario.keyboard("{Escape}");
        await waitFor(() => expect(aoFechar).toHaveBeenCalled());
        expect(aoExcluir).not.toHaveBeenCalled();
        expect(servidor.chamadasPara("DELETE", "/products/prod_1")).toHaveLength(1);
    });

    it("renders consequences and 409 cart count in en-US", async () => {
        const usuario = userEvent.setup();
        setLocale("en-US");
        new FakeServer()
            .responder("DELETE", `/admin/v1/orgs/${ORG}/products/prod_1`, recusaDeCarrinho(3, 2))
            .instalar();
        montar(2);
        await waitForModalFocus();

        expect(screen.getByRole("dialog", { name: "Delete product?" })).toBeInTheDocument();
        expect(screen.getByText(/2 agents offer this product/)).toBeInTheDocument();

        await usuario.type(screen.getByLabelText(/Clareamento dental|product name/i), "Clareamento dental");
        await usuario.click(screen.getByRole("button", { name: "Delete product" }));
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
