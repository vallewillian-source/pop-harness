
import { useEffect, useState } from "react";
import { SofaSkeleton } from "@sofacoding/sofa-ui";
import {
    agentsUsingCategory,
    deleteCategory,
    categoryProducts,
    type AgentSummary,
    type Category,
    type CategoryProduct,
} from "../api/structure";
import { useI18n } from "../i18n";
import { DialogExclusion } from "../components/DialogExclusion";

export type DialogDeleteCategoryProps = {
    opened: boolean;
    onClose: () => void;
    org: string;
    category: Category | null;
    onDelete: () => void;
};

type Links = {
    products: CategoryProduct[];
    agents: AgentSummary[];
};

export function DialogDeleteCategory({
    opened: isOpen,
    onClose,
    org,
    category,
    onDelete,
}: DialogDeleteCategoryProps) {
    const { translate } = useI18n();
    const [loading, setLoading] = useState(false);
    const [links, setLinks] = useState<Links | null>(null);
    const [failureVerify, setFailureVerify] = useState(false);

    useEffect(() => {
        if (!isOpen || !category) return;
        let cancelled = false;
        setLoading(true);
        setLinks(null);
        setFailureVerify(false);
        Promise.all([
            categoryProducts(org, category.id),
            agentsUsingCategory(org, category.id),
        ])
            .then(([products, agents]) => {
                if (!cancelled) setLinks({ products, agents });
            })
            .catch(() => {
                if (!cancelled) setFailureVerify(true);
            })
            .finally(() => {
                if (!cancelled) setLoading(false);
            });
        return () => {
            cancelled = true;
        };
    }, [isOpen, category, org]);

    if (!category) return null;

    const withoutLink = links !== null && links.products.length === 0 && links.agents.length === 0;
    const quantityProducts = links?.products.length ?? 0;

    function consequencia() {
        if (loading) {
            return <SofaSkeleton lines={2} />;
        }
        if (failureVerify || links === null) {
            return (
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {translate("structure.deleteCatCheckFailed")}
                </p>
            );
        }
        if (withoutLink) {
            return (
                <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                    {translate("structure.deleteCatNoLinks")}
                </p>
            );
        }
        const names = links.agents.map((agente) => agente.name).join(", ");
        const productsText =
            links.products.length === 1
                ? translate("structure.deleteCatProductOne")
                : translate("structure.deleteCatProductMany", { count: links.products.length });
        const productsPronoun =
            links.products.length === 1
                ? translate("structure.deleteCatProductsPronounOne")
                : translate("structure.deleteCatProductsPronounMany");
        const agentsText =
            links.agents.length === 1
                ? translate("structure.deleteCatAgentOne")
                : translate("structure.deleteCatAgentMany", { count: links.agents.length });
        return (
            <div className="poph-stack" style={{ gap: "var(--pop-spacing-sm, 8px)" }}>
                {links.products.length > 0 && (
                    <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                        {translate("structure.deleteCatProductsLine", {
                            products: productsText,
                            pronoun: productsPronoun,
                        })}{" "}
                        <strong>
                            {links.products.map((product) => product.name).join(", ")}
                        </strong>
                        .
                    </p>
                )}
                {links.agents.length > 0 && (
                    <p style={{ margin: 0, color: "var(--pop-text-secondary)", lineHeight: 1.6 }}>
                        {translate("structure.deleteCatAgentsLine", { agents: agentsText })}{" "}
                        <strong>{names}</strong>.
                    </p>
                )}
            </div>
        );
    }

    const labelForce =
        quantityProducts > 0
            ? quantityProducts === 1
                ? translate("structure.deleteCatForceWithProductsOne")
                : translate("structure.deleteCatForceWithProductsMany", { count: quantityProducts })
            : translate("structure.deleteCatForceAgentsOnly");

    return (
        <DialogExclusion
            opened={isOpen}
            onClose={onClose}
            title={translate("structure.deleteCatTitle")}
            descricao={translate("structure.deleteCatDescription", { name: category.name })}
            resource={category.name}
            labelResource={translate("structure.deleteCatResourceLabel")}
            labelConfirm={translate("structure.deleteCatConfirm")}
            consequencia={consequencia()}
            allowForce={!withoutLink}
            labelForce={labelForce}
            descriptionForce={translate("structure.deleteCatForceDescription")}
            onConfirm={async (forcar) => {
                await deleteCategory(org, category.id, forcar);
            }}
            onDelete={onDelete}
        />
    );
}
