
import { useEffect, useRef, useState } from "react";
import { ApiError } from "../api/client";
import {
    refusalCount,
    deleteProduct,
    type CartCount,
    type Product,
} from "../api/products";
import { useI18n } from "../i18n";
import { DialogExclusion } from "../components/DialogExclusion";

export type DialogDeleteProductProps = {
    opened: boolean;
    onClose: () => void;
    org: string;
    product: Product | null;
    agentsSee: number | null;
    onDelete: (noticeIndex: string | null) => void;
};

export function DialogDeleteProduct({
    opened: isOpen,
    onClose,
    org,
    product,
    agentsSee,
    onDelete,
}: DialogDeleteProductProps) {
    const { translate } = useI18n();
    const noticeRef = useRef<string | null>(null);
    const [count, setCount] = useState<CartCount | null>(null);

    useEffect(() => {
        if (isOpen) {
            setCount(null);
        }
    }, [isOpen]);

    if (!product) return null;

    function consequencia() {
        const texto =
            agentsSee === null ? (
                translate("products.deleteAgentsUnknown")
            ) : agentsSee === 0 ? (
                translate("products.deleteAgentsNone")
            ) : (
                <>
                    {agentsSee === 1
                        ? translate("products.deleteAgentsOne")
                        : translate("products.deleteAgentsMany", { count: agentsSee })}{" "}
                    {translate("products.deleteAgentsSuffix")}
                </>
            );
        const itemsText = count
            ? count.itens_de_carrinho === 1
                ? translate("products.deleteCartItemOne")
                : translate("products.deleteCartItemMany", { count: count.itens_de_carrinho })
            : "";
        const cartsText = count
            ? count.carrinhos === 1
                ? translate("products.deleteCartOne")
                : translate("products.deleteCartMany", { count: count.carrinhos })
            : "";
        return (
            <div className="poph-stack" style={{ gap: "var(--pop-spacing-sm, 6px)" }}>
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {texto}
                </p>
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {translate("products.deleteCartsNote")}
                </p>
                {count && (
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

    const forceItemsText = count
        ? count.itens_de_carrinho === 1
            ? translate("products.deleteForceItemOne")
            : translate("products.deleteForceItemMany", { count: count.itens_de_carrinho })
        : "";
    const labelForce = count
        ? translate("products.deleteForceLabelWithCount", { items: forceItemsText })
        : translate("products.deleteForceLabelDefault");

    return (
        <DialogExclusion
            opened={isOpen}
            onClose={onClose}
            title={translate("products.deleteTitle")}
            descricao={translate("products.deleteDescription", { name: product.name })}
            resource={product.name}
            labelResource={translate("products.deleteResourceLabel")}
            labelConfirm={translate("products.deleteConfirm")}
            consequencia={consequencia()}
            allowForce
            forceOnlyAfterRefusal
            labelForce={labelForce}
            descriptionForce={translate("products.deleteForceDescription")}
            onConfirm={async (forcar) => {
                try {
                    const resultado = await deleteProduct(org, product.id, forcar);
                    noticeRef.current = resultado.index_warning ?? null;
                } catch (failure) {
                    if (failure instanceof ApiError && failure.status === 409) {
                        setCount(refusalCount(failure.detalhes));
                    }
                    throw failure;
                }
            }}
            onDelete={() => onDelete(noticeRef.current)}
        />
    );
}
