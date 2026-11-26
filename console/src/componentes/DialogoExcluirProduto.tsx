/**
 * Exclusão de produto do catálogo (HR-42), com a consequência escrita antes do botão.
 *
 * Dois passos (HR-42g, mesmo desenho da categoria):
 * - a primeira tentativa vai sem `force`. Se o produto estiver em itens de carrinho, a API recusa
 *   com `409`, nada é apagado, e a tela mostra a contagem real (itens e carrinhos) em linguagem
 *   comum;
 * - a recusa libera a confirmação extra ("Excluir mesmo assim…"), e a tentativa seguinte vai com
 *   `?force=true`. Sem carrinho, a exclusão sai direto, como antes.
 *
 * O que sai junto, pelo esquema do banco: o produto some do catálogo e da busca dos agentes; itens
 * de carrinho que citam o produto perdem a referência a ele (o preço já gravado continua).
 *
 * A contagem de agentes vem da tela: agente sem categoria vinculada enxerga o catálogo inteiro, então
 * entra na conta. A confirmação é digitar o nome exato (`DialogoExclusao`).
 */

import { useEffect, useRef, useState } from "react";
import { ApiError } from "../api/client";
import {
    refusalCount,
    deleteProduct,
    type CartCount,
    type Product,
} from "../api/products";
import { useI18n } from "../i18n";
import { DialogoExclusao } from "./DialogoExclusao";

export type DialogoExcluirProdutoProps = {
    aberto: boolean;
    aoFechar: () => void;
    org: string;
    produto: Product | null;
    /** Agentes que oferecem o produto hoje; `null` = a leitura dos agentes falhou. */
    agentesQueVeem: number | null;
    /** Chamado depois da exclusão, com o aviso do índice quando ele não atualizou. */
    aoExcluir: (avisoDoIndice: string | null) => void;
};

export function DialogoExcluirProduto({
    aberto,
    aoFechar,
    org,
    produto,
    agentesQueVeem,
    aoExcluir,
}: DialogoExcluirProdutoProps) {
    const { translate } = useI18n();
    // Aviso do índice da exclusão em curso: vai de `aoConfirmar` até `aoExcluir`.
    const avisoRef = useRef<string | null>(null);
    // Contagem que a recusa 409 trouxe nesta abertura; some quando o diálogo fecha.
    const [contagem, setContagem] = useState<CartCount | null>(null);

    useEffect(() => {
        if (aberto) {
            setContagem(null);
        }
    }, [aberto]);

    if (!produto) return null;

    function consequencia() {
        const texto =
            agentesQueVeem === null ? (
                translate("products.deleteAgentsUnknown")
            ) : agentesQueVeem === 0 ? (
                translate("products.deleteAgentsNone")
            ) : (
                <>
                    {agentesQueVeem === 1
                        ? translate("products.deleteAgentsOne")
                        : translate("products.deleteAgentsMany", { count: agentesQueVeem })}{" "}
                    {translate("products.deleteAgentsSuffix")}
                </>
            );
        const itemsText = contagem
            ? contagem.itens_de_carrinho === 1
                ? translate("products.deleteCartItemOne")
                : translate("products.deleteCartItemMany", { count: contagem.itens_de_carrinho })
            : "";
        const cartsText = contagem
            ? contagem.carrinhos === 1
                ? translate("products.deleteCartOne")
                : translate("products.deleteCartMany", { count: contagem.carrinhos })
            : "";
        return (
            <div className="poph-pilha" style={{ gap: "var(--pop-spacing-sm, 6px)" }}>
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {texto}
                </p>
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {translate("products.deleteCartsNote")}
                </p>
                {contagem && (
                    <p style={{ margin: 0, color: "var(--pop-text-primary)", lineHeight: 1.6 }}>
                        {translate("products.deleteCartsConflict", {
                            items: itemsText,
                            carts: cartsText,
                        })}
                    </p>
                )}
            </div>
        );
    }

    const forceItemsText = contagem
        ? contagem.itens_de_carrinho === 1
            ? translate("products.deleteForceItemOne")
            : translate("products.deleteForceItemMany", { count: contagem.itens_de_carrinho })
        : "";
    const rotuloForcar = contagem
        ? translate("products.deleteForceLabelWithCount", { items: forceItemsText })
        : translate("products.deleteForceLabelDefault");

    return (
        <DialogoExclusao
            aberto={aberto}
            aoFechar={aoFechar}
            titulo={translate("products.deleteTitle")}
            descricao={translate("products.deleteDescription", { name: produto.name })}
            recurso={produto.name}
            rotuloRecurso={translate("products.deleteResourceLabel")}
            rotuloConfirmar={translate("products.deleteConfirm")}
            consequencia={consequencia()}
            permiteForcar
            forcarSoAposRecusa
            rotuloForcar={rotuloForcar}
            descricaoForcar={translate("products.deleteForceDescription")}
            aoConfirmar={async (forcar) => {
                try {
                    const resultado = await deleteProduct(org, produto.id, forcar);
                    avisoRef.current = resultado.index_warning ?? null;
                } catch (falha) {
                    // 409: a recusa traz a contagem; a tela mostra e a confirmação extra libera o envio.
                    if (falha instanceof ApiError && falha.status === 409) {
                        setContagem(refusalCount(falha.detalhes));
                    }
                    throw falha;
                }
            }}
            aoExcluir={() => aoExcluir(avisoRef.current)}
        />
    );
}
