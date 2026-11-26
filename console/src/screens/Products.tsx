
import { useCallback, useEffect, useId, useMemo, useState, type ReactNode } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { createColumnHelper } from "@tanstack/react-table";
import {
    IconPlus,
    IconRefresh,
    IconTag,
    SofaBanner,
    SofaButton,
    SofaEmptyState,
    SofaErrorState,
    SofaField,
    SofaInput,
    SofaSelect,
    SofaSkeleton,
    SofaTable,
} from "@sofacoding/sofa-ui";
import type { Agente } from "../api/agents";
import type { Category } from "../api/structure";
import { getOrg } from "../api/orgs";
import {
    agentsSeeingProduct,
    loadAgentsWithCategories,
    listCategories,
    listProducts,
    type Product,
    type ProductStatus,
} from "../api/products";
import { DialogDeleteProduct } from "../components/DialogDeleteProduct";
import { ModalProduct } from "../components/ModalProduct";
import { useI18n } from "../i18n";
import "../styles/products.css";

type Situacao = "todas" | ProductStatus;

type Filters = {
    search: string;
    situacao: Situacao;
    categoria: string;
};

const INITIAL_FILTERS: Filters = { search: "", situacao: "todas", categoria: "todas" };

function classStatus(situacao: string): string {
    return situacao === "active" ? "poph-status--active" : "poph-status--inactive";
}

function labelStatus(
    situacao: string,
    translate: ReturnType<typeof useI18n>["translate"],
): string {
    switch (situacao) {
        case "active":
            return translate("products.statusActive");
        case "inactive":
            return translate("products.statusInactive");
        case "archived":
            return translate("products.statusArchived");
        default:
            return situacao;
    }
}

function normalize(texto: string): string {
    return texto.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLowerCase();
}

function matchesSearch(product: Product, search: string): boolean {
    if (!search) return true;
    const target = normalize(
        [product.name, product.slug, ...(product.aliases ?? [])].join(" "),
    );
    return target.includes(normalize(search.trim()));
}

function textAgents(
    total: number | null,
    translate: ReturnType<typeof useI18n>["translate"],
): string {
    if (total === null) return "—";
    if (total === 0) return translate("products.agentsCountNone");
    return total === 1
        ? translate("products.agentsCountOne")
        : translate("products.agentsCountMany", { count: total });
}

const column = createColumnHelper<Product>();

export function Products() {
    const { translate } = useI18n();
    const { orgId = "" } = useParams();
    const navigate = useNavigate();
    const catalogId = useId();

    const [nameOrg, setOrgName] = useState<string | null>(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);
    const [products, setProducts] = useState<Product[]>([]);
    const [categories, setCategories] = useState<Category[]>([]);
    const [agents, setAgents] = useState<Agente[] | null>(null);
    const [filters, setFilters] = useState<Filters>(INITIAL_FILTERS);
    const [noticeIndex, setIndexNotice] = useState<string | null>(null);
    const [edit, setEdit] = useState<{ opened: boolean; produto: Product | null }>({
        opened: false,
        produto: null,
    });
    const [toDelete, setToDelete] = useState<Product | null>(null);

    const load = useCallback(async () => {
        setLoading(true);
        setError(null);
        try {
            const [list, cats] = await Promise.all([
                listProducts(orgId),
                listCategories(orgId),
            ]);
            setProducts(list);
            setCategories(cats);
        } catch (failure) {
            setError(failure instanceof Error ? failure.message : translate("products.loadError"));
        } finally {
            setLoading(false);
        }
    }, [orgId, translate]);

    const loadAgents = useCallback(async () => {
        try {
            setAgents(await loadAgentsWithCategories(orgId));
        } catch {
            setAgents(null);
        }
    }, [orgId]);

    useEffect(() => {
        setOrgName(null);
        getOrg(orgId)
            .then((org) => setOrgName(org.name))
            .catch(() => setOrgName(null));
        void load();
        void loadAgents();
    }, [orgId, load, loadAgents]);

    const nameCategory = useMemo(() => {
        const map = new Map<string, string>();
        for (const category of categories) map.set(category.id, category.name);
        return map;
    }, [categories]);

    const countAgents = useMemo(() => {
        const map = new Map<string, number>();
        if (agents === null) return map;
        for (const product of products) {
            map.set(product.id, agentsSeeingProduct(agents, product).length);
        }
        return map;
    }, [agents, products]);

    const visible = useMemo(
        () =>
            products.filter(
                (product) =>
                    matchesSearch(product, filters.search) &&
                    (filters.situacao === "todas" || product.status === filters.situacao) &&
                    (filters.categoria === "todas" || product.category_id === filters.categoria),
            ),
        [products, filters],
    );

    const columns = useMemo(
        () => [
            column.accessor("name", {
                header: translate("products.colProduct"),
                cell: (info) => {
                    const product = info.row.original;
                    return (
                        <div className="poph-stack" style={{ gap: 2 }}>
                            <span style={{ color: "var(--pop-text-primary)" }}>{product.name}</span>
                            <span className="poph-mono-muted">{product.slug}</span>
                            {product.aliases?.length > 0 && (
                                <span className="poph-mono-muted">
                                    {translate("products.colProductAliases", {
                                        aliases: product.aliases.join(", "),
                                    })}
                                </span>
                            )}
                        </div>
                    );
                },
            }),
            column.accessor("category_id", {
                header: translate("products.colCategory"),
                cell: (info) =>
                    nameCategory.get(info.getValue()) ?? translate("products.colCategoryNone"),
            }),
            column.accessor("price_cents", {
                header: translate("products.colPrice"),
                cell: (info) => {
                    const product = info.row.original;
                    return (
                        <div className="poph-stack" style={{ gap: 2 }}>
                            <span>{product.price_text}</span>
                            {product.unit && <span className="poph-mono-muted">{product.unit}</span>}
                        </div>
                    );
                },
            }),
            column.accessor("status", {
                header: translate("products.colStatus"),
                cell: (info) => (
                    <span className={`poph-status ${classStatus(info.getValue())}`}>
                        <span className="poph-status__point" aria-hidden="true" />
                        {labelStatus(info.getValue(), translate)}
                    </span>
                ),
            }),
            column.display({
                id: "agentes",
                header: translate("products.colAgents"),
                cell: (info) => (
                    <span
                        title={
                            agents === null
                                ? translate("products.colAgentsLoadFailedTitle")
                                : undefined
                        }
                    >
                        {textAgents(countAgents.get(info.row.original.id) ?? null, translate)}
                    </span>
                ),
            }),
            column.display({
                id: "acoes",
                header: translate("products.colActions"),
                cell: (info) => {
                    const product = info.row.original;
                    return (
                        <div style={{ display: "flex", gap: 4, justifyContent: "flex-end" }}>
                            <SofaButton
                                variant="subtle"
                                size="sm"
                                aria-label={translate("products.editProductAria", {
                                    name: product.name,
                                })}
                                onClick={() => setEdit({ opened: true, produto: product })}
                            >
                                {translate("products.edit")}
                            </SofaButton>
                            <SofaButton
                                variant="subtle"
                                size="sm"
                                aria-label={translate("products.deleteProductAria", {
                                    name: product.name,
                                })}
                                onClick={() => setToDelete(product)}
                            >
                                {translate("products.delete")}
                            </SofaButton>
                        </div>
                    );
                },
            }),
        ],
        [agents, countAgents, nameCategory, translate],
    );

    function onSaveProduct(saved: Product) {
        setProducts((current) => {
            const exists = current.some((item) => item.id === saved.id);
            return exists
                ? current.map((item) => (item.id === saved.id ? saved : item))
                : [...current, saved];
        });
        setIndexNotice(saved.index_warning ?? null);
        void loadAgents();
    }

    function clearFilters() {
        setFilters(INITIAL_FILTERS);
    }

    const noProducts = !loading && !error && products.length === 0;
    const noResult = !loading && !error && products.length > 0 && visible.length === 0;

    let corpo: ReactNode;
    if (error && !loading) {
        corpo = (
            <SofaErrorState
                title={translate("products.errorTitle")}
                description={translate("products.errorDescription")}
                details={error}
                retry={{ label: translate("products.retry"), onClick: () => void load() }}
            />
        );
    } else if (loading && products.length === 0) {
        corpo = (
            <div aria-busy="true" aria-label={translate("products.loadingAria")}>
                <SofaSkeleton lines={4} />
            </div>
        );
    } else if (noProducts) {
        const noCategories = categories.length === 0;
        corpo = (
            <SofaEmptyState
                icon={<IconTag aria-hidden="true" />}
                title={translate("products.emptyTitle")}
                description={
                    noCategories
                        ? translate("products.emptyNoCategoriesDescription")
                        : translate("products.emptyDefaultDescription")
                }
                action={
                    noCategories ? (
                        <SofaButton
                            variant="primary"
                            onClick={() => navigate(`/orgs/${orgId}/structure`)}
                        >
                            {translate("products.goToStructure")}
                        </SofaButton>
                    ) : (
                        <SofaButton
                            variant="primary"
                            icon={<IconPlus aria-hidden="true" />}
                            onClick={() => setEdit({ opened: true, produto: null })}
                        >
                            {translate("products.createFirstProduct")}
                        </SofaButton>
                    )
                }
            />
        );
    } else if (noResult) {
        corpo = (
            <SofaEmptyState
                icon={<IconTag aria-hidden="true" />}
                title={translate("products.filteredEmptyTitle")}
                description={translate("products.filteredEmptyDescription")}
                action={
                    <SofaButton variant="secondary" onClick={clearFilters}>
                        {translate("products.clearFilters")}
                    </SofaButton>
                }
            />
        );
    } else {
        corpo = (
            <div className="poph-products__table">
                <SofaTable
                    label={translate("products.tableLabel")}
                    columns={columns}
                    data={visible}
                    getRowId={(product) => product.id}
                    isLoading={loading}
                    footerMeta={({ total }) =>
                        total === products.length
                            ? total === 1
                                ? translate("products.footerOne")
                                : translate("products.footerMany", { total })
                            : translate("products.footerFiltered", {
                                  total,
                                  all: products.length,
                              })
                    }
                />
            </div>
        );
    }

    const totalCategories = categories.length;

    return (
        <>
            <div className="poph-page poph-products">
            <div className="poph-page__top">
                <div>
                    <p className="poph-products__eyebrow">
                        {nameOrg
                            ? translate("products.eyebrowOrg", { name: nameOrg })
                            : translate("products.eyebrow")}
                    </p>
                    <h1 className="poph-page__title">{translate("products.pageTitle")}</h1>
                    <p className="poph-page__description">
                        {translate("products.pageDescription")}
                    </p>
                </div>
                <div className="poph-page__actions">
                    <SofaButton
                        variant="primary"
                        icon={<IconPlus aria-hidden="true" />}
                        disabled={totalCategories === 0}
                        onClick={() => setEdit({ opened: true, produto: null })}
                    >
                        {translate("products.newProduct")}
                    </SofaButton>
                    <SofaButton
                        variant="secondary"
                        icon={<IconRefresh aria-hidden="true" />}
                        onClick={() => {
                            void load();
                            void loadAgents();
                        }}
                        loading={loading}
                    >
                        {translate("products.refresh")}
                    </SofaButton>
                </div>
            </div>
                {noticeIndex && (
                    <SofaBanner
                        tone="warn"
                        title={translate("products.indexWarningTitle")}
                        onDismiss={() => setIndexNotice(null)}
                    >
                        {translate("products.indexWarningBody")}{" "}
                        <span className="poph-mono-muted">{noticeIndex}</span>
                    </SofaBanner>
                )}
                {agents === null && !loading && (
                    <SofaBanner tone="info" title={translate("products.agentsBannerTitle")}>
                        {translate("products.agentsBannerBody")}
                    </SofaBanner>
                )}
                <section className="poph-section poph-products__section" aria-labelledby={catalogId}>
                    <div className="poph-products__header">
                        <h2 id={catalogId} className="poph-section__title">
                            {translate("products.catalogHeading")}
                        </h2>
                        <div className="poph-products__filters">
                            <SofaField label={translate("products.filterSearchLabel")} htmlFor="poph-produtos-busca">
                                {(control) => (
                                    <SofaInput
                                        id={control.id}
                                        value={filters.search}
                                        placeholder={translate("products.filterSearchPlaceholder")}
                                        autoComplete="off"
                                        onChange={(evento) =>
                                            setFilters((current) => ({
                                                ...current,
                                                search: evento.target.value,
                                            }))
                                        }
                                    />
                                )}
                            </SofaField>
                            <SofaField label={translate("products.filterStatusLabel")} htmlFor="poph-produtos-situacao">
                                {(control) => (
                                    <SofaSelect
                                        id={control.id}
                                        value={filters.situacao}
                                        onChange={(evento) =>
                                            setFilters((current) => ({
                                                ...current,
                                                situacao: evento.target.value as Situacao,
                                            }))
                                        }
                                    >
                                        <option value="todas">{translate("products.filterAll")}</option>
                                        <option value="active">{translate("products.filterActive")}</option>
                                        <option value="inactive">{translate("products.filterInactive")}</option>
                                        <option value="archived">{translate("products.filterArchived")}</option>
                                    </SofaSelect>
                                )}
                            </SofaField>
                            <SofaField label={translate("products.filterCategoryLabel")} htmlFor="poph-produtos-categoria">
                                {(control) => (
                                    <SofaSelect
                                        id={control.id}
                                        value={filters.categoria}
                                        onChange={(evento) =>
                                            setFilters((current) => ({
                                                ...current,
                                                categoria: evento.target.value,
                                            }))
                                        }
                                    >
                                        <option value="todas">{translate("products.filterAll")}</option>
                                        {categories.map((category) => (
                                            <option key={category.id} value={category.id}>
                                                {category.name}
                                            </option>
                                        ))}
                                    </SofaSelect>
                                )}
                            </SofaField>
                        </div>
                    </div>
                    {corpo}
                </section>
            </div>

            <ModalProduct
                org={orgId}
                opened={edit.opened}
                product={edit.produto}
                categories={categories}
                onClose={() => setEdit({ opened: false, produto: null })}
                onSave={onSaveProduct}
            />

            <DialogDeleteProduct
                opened={toDelete !== null}
                onClose={() => setToDelete(null)}
                org={orgId}
                product={toDelete}
                agentsSee={
                    toDelete && agents !== null && countAgents.has(toDelete.id)
                        ? (countAgents.get(toDelete.id) ?? 0)
                        : null
                }
                onDelete={(notice) => {
                    if (toDelete) {
                        setProducts((current) => current.filter((item) => item.id !== toDelete.id));
                    }
                    setToDelete(null);
                    setIndexNotice(notice);
                    void loadAgents();
                }}
            />
        </>
    );
}
