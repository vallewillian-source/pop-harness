/**
 * Catálogo de produtos da organização (HR-42).
 *
 * Uma tabela com todos os produtos (de qualquer situação), filtros por nome, situação e categoria,
 * e a coluna "Agentes que oferecem", que conta quais agentes enxergam cada produto pela categoria
 * (ou por não ter categoria vinculada). Criar, editar e excluir passam pelos modais de produto.
 *
 * A tela fica no contexto da organização (`/orgs/:id/products`), não dentro do agente: o catálogo é
 * da organização e os agentes só ligam categorias (ver `docs/CONSOLE.md`).
 */

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

type Filtros = {
    busca: string;
    situacao: Situacao;
    categoria: string;
};

const FILTROS_INICIAIS: Filtros = { busca: "", situacao: "todas", categoria: "todas" };

/** Classe do ponto de status (6 px + texto), no padrão de Orgs e OrgPainel. */
function classeDaSituacao(situacao: string): string {
    return situacao === "active" ? "poph-status--active" : "poph-status--inactive";
}

function rotuloDaSituacao(
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

/** Texto para comparar busca: minúsculas e sem acento. */
function normalizar(texto: string): string {
    return texto.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLowerCase();
}

function casaComBusca(produto: Product, busca: string): boolean {
    if (!busca) return true;
    const alvo = normalizar(
        [produto.name, produto.slug, ...(produto.aliases ?? [])].join(" "),
    );
    return alvo.includes(normalizar(busca.trim()));
}

function textoDeAgentes(
    total: number | null,
    translate: ReturnType<typeof useI18n>["translate"],
): string {
    if (total === null) return "—";
    if (total === 0) return translate("products.agentsCountNone");
    return total === 1
        ? translate("products.agentsCountOne")
        : translate("products.agentsCountMany", { count: total });
}

const coluna = createColumnHelper<Product>();

export function Products() {
    const { translate } = useI18n();
    const { orgId = "" } = useParams();
    const navegar = useNavigate();
    const idDoCatalogo = useId();

    const [nomeDaOrg, setNomeDaOrg] = useState<string | null>(null);
    const [carregando, setCarregando] = useState(true);
    const [erro, setErro] = useState<string | null>(null);
    const [produtos, setProdutos] = useState<Product[]>([]);
    const [categorias, setCategorias] = useState<Category[]>([]);
    const [agentes, setAgentes] = useState<Agente[] | null>(null);
    const [filtros, setFiltros] = useState<Filtros>(FILTROS_INICIAIS);
    const [avisoDoIndice, setAvisoDoIndice] = useState<string | null>(null);
    const [edicao, setEdicao] = useState<{ aberto: boolean; produto: Product | null }>({
        aberto: false,
        produto: null,
    });
    const [paraExcluir, setParaExcluir] = useState<Product | null>(null);

    const carregar = useCallback(async () => {
        setCarregando(true);
        setErro(null);
        try {
            const [lista, cats] = await Promise.all([
                listProducts(orgId),
                listCategories(orgId),
            ]);
            setProdutos(lista);
            setCategorias(cats);
        } catch (falha) {
            setErro(falha instanceof Error ? falha.message : translate("products.loadError"));
        } finally {
            setCarregando(false);
        }
    }, [orgId, translate]);

    const carregarAgentes = useCallback(async () => {
        try {
            setAgentes(await loadAgentsWithCategories(orgId));
        } catch {
            // Sem os agentes, a coluna mostra "—"; o catálogo segue normal.
            setAgentes(null);
        }
    }, [orgId]);

    useEffect(() => {
        setNomeDaOrg(null);
        getOrg(orgId)
            .then((org) => setNomeDaOrg(org.name))
            .catch(() => setNomeDaOrg(null));
        void carregar();
        void carregarAgentes();
    }, [orgId, carregar, carregarAgentes]);

    const nomeDaCategoria = useMemo(() => {
        const mapa = new Map<string, string>();
        for (const categoria of categorias) mapa.set(categoria.id, categoria.name);
        return mapa;
    }, [categorias]);

    const contagemDeAgentes = useMemo(() => {
        const mapa = new Map<string, number>();
        if (agentes === null) return mapa;
        for (const produto of produtos) {
            mapa.set(produto.id, agentsSeeingProduct(agentes, produto).length);
        }
        return mapa;
    }, [agentes, produtos]);

    const visiveis = useMemo(
        () =>
            produtos.filter(
                (produto) =>
                    casaComBusca(produto, filtros.busca) &&
                    (filtros.situacao === "todas" || produto.status === filtros.situacao) &&
                    (filtros.categoria === "todas" || produto.category_id === filtros.categoria),
            ),
        [produtos, filtros],
    );

    const colunas = useMemo(
        () => [
            coluna.accessor("name", {
                header: translate("products.colProduct"),
                cell: (info) => {
                    const produto = info.row.original;
                    return (
                        <div className="poph-stack" style={{ gap: 2 }}>
                            <span style={{ color: "var(--pop-text-primary)" }}>{produto.name}</span>
                            <span className="poph-mono-muted">{produto.slug}</span>
                            {produto.aliases?.length > 0 && (
                                <span className="poph-mono-muted">
                                    {translate("products.colProductAliases", {
                                        aliases: produto.aliases.join(", "),
                                    })}
                                </span>
                            )}
                        </div>
                    );
                },
            }),
            coluna.accessor("category_id", {
                header: translate("products.colCategory"),
                cell: (info) =>
                    nomeDaCategoria.get(info.getValue()) ?? translate("products.colCategoryNone"),
            }),
            coluna.accessor("price_cents", {
                header: translate("products.colPrice"),
                cell: (info) => {
                    const produto = info.row.original;
                    return (
                        <div className="poph-stack" style={{ gap: 2 }}>
                            <span>{produto.price_text}</span>
                            {produto.unit && <span className="poph-mono-muted">{produto.unit}</span>}
                        </div>
                    );
                },
            }),
            coluna.accessor("status", {
                header: translate("products.colStatus"),
                cell: (info) => (
                    <span className={`poph-status ${classeDaSituacao(info.getValue())}`}>
                        <span className="poph-status__point" aria-hidden="true" />
                        {rotuloDaSituacao(info.getValue(), translate)}
                    </span>
                ),
            }),
            coluna.display({
                id: "agentes",
                header: translate("products.colAgents"),
                cell: (info) => (
                    <span
                        title={
                            agentes === null
                                ? translate("products.colAgentsLoadFailedTitle")
                                : undefined
                        }
                    >
                        {textoDeAgentes(contagemDeAgentes.get(info.row.original.id) ?? null, translate)}
                    </span>
                ),
            }),
            coluna.display({
                id: "acoes",
                header: translate("products.colActions"),
                cell: (info) => {
                    const produto = info.row.original;
                    return (
                        <div style={{ display: "flex", gap: 4, justifyContent: "flex-end" }}>
                            <SofaButton
                                variant="subtle"
                                size="sm"
                                aria-label={translate("products.editProductAria", {
                                    name: produto.name,
                                })}
                                onClick={() => setEdicao({ aberto: true, produto })}
                            >
                                {translate("products.edit")}
                            </SofaButton>
                            <SofaButton
                                variant="subtle"
                                size="sm"
                                aria-label={translate("products.deleteProductAria", {
                                    name: produto.name,
                                })}
                                onClick={() => setParaExcluir(produto)}
                            >
                                {translate("products.delete")}
                            </SofaButton>
                        </div>
                    );
                },
            }),
        ],
        [agentes, contagemDeAgentes, nomeDaCategoria, translate],
    );

    function aoSalvarProduto(salvo: Product) {
        setProdutos((atual) => {
            const existe = atual.some((item) => item.id === salvo.id);
            return existe
                ? atual.map((item) => (item.id === salvo.id ? salvo : item))
                : [...atual, salvo];
        });
        setAvisoDoIndice(salvo.index_warning ?? null);
        void carregarAgentes();
    }

    function limparFiltros() {
        setFiltros(FILTROS_INICIAIS);
    }

    const semProdutos = !carregando && !erro && produtos.length === 0;
    const semResultado = !carregando && !erro && produtos.length > 0 && visiveis.length === 0;

    let corpo: ReactNode;
    if (erro && !carregando) {
        corpo = (
            <SofaErrorState
                title={translate("products.errorTitle")}
                description={translate("products.errorDescription")}
                details={erro}
                retry={{ label: translate("products.retry"), onClick: () => void carregar() }}
            />
        );
    } else if (carregando && produtos.length === 0) {
        corpo = (
            <div aria-busy="true" aria-label={translate("products.loadingAria")}>
                <SofaSkeleton lines={4} />
            </div>
        );
    } else if (semProdutos) {
        const semCategorias = categorias.length === 0;
        corpo = (
            <SofaEmptyState
                icon={<IconTag aria-hidden="true" />}
                title={translate("products.emptyTitle")}
                description={
                    semCategorias
                        ? translate("products.emptyNoCategoriesDescription")
                        : translate("products.emptyDefaultDescription")
                }
                action={
                    semCategorias ? (
                        <SofaButton
                            variant="primary"
                            onClick={() => navegar(`/orgs/${orgId}/structure`)}
                        >
                            {translate("products.goToStructure")}
                        </SofaButton>
                    ) : (
                        <SofaButton
                            variant="primary"
                            icon={<IconPlus aria-hidden="true" />}
                            onClick={() => setEdicao({ aberto: true, produto: null })}
                        >
                            {translate("products.createFirstProduct")}
                        </SofaButton>
                    )
                }
            />
        );
    } else if (semResultado) {
        corpo = (
            <SofaEmptyState
                icon={<IconTag aria-hidden="true" />}
                title={translate("products.filteredEmptyTitle")}
                description={translate("products.filteredEmptyDescription")}
                action={
                    <SofaButton variant="secondary" onClick={limparFiltros}>
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
                    columns={colunas}
                    data={visiveis}
                    getRowId={(produto) => produto.id}
                    isLoading={carregando}
                    footerMeta={({ total }) =>
                        total === produtos.length
                            ? total === 1
                                ? translate("products.footerOne")
                                : translate("products.footerMany", { total })
                            : translate("products.footerFiltered", {
                                  total,
                                  all: produtos.length,
                              })
                    }
                />
            </div>
        );
    }

    const totalDeCategorias = categorias.length;

    return (
        <>
            <div className="poph-page poph-products">
            <div className="poph-page__top">
                <div>
                    <p className="poph-products__eyebrow">
                        {nomeDaOrg
                            ? translate("products.eyebrowOrg", { name: nomeDaOrg })
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
                        disabled={totalDeCategorias === 0}
                        onClick={() => setEdicao({ aberto: true, produto: null })}
                    >
                        {translate("products.newProduct")}
                    </SofaButton>
                    <SofaButton
                        variant="secondary"
                        icon={<IconRefresh aria-hidden="true" />}
                        onClick={() => {
                            void carregar();
                            void carregarAgentes();
                        }}
                        loading={carregando}
                    >
                        {translate("products.refresh")}
                    </SofaButton>
                </div>
            </div>
                {avisoDoIndice && (
                    <SofaBanner
                        tone="warn"
                        title={translate("products.indexWarningTitle")}
                        onDismiss={() => setAvisoDoIndice(null)}
                    >
                        {translate("products.indexWarningBody")}{" "}
                        <span className="poph-mono-muted">{avisoDoIndice}</span>
                    </SofaBanner>
                )}
                {agentes === null && !carregando && (
                    <SofaBanner tone="info" title={translate("products.agentsBannerTitle")}>
                        {translate("products.agentsBannerBody")}
                    </SofaBanner>
                )}
                <section className="poph-section poph-products__section" aria-labelledby={idDoCatalogo}>
                    <div className="poph-products__header">
                        <h2 id={idDoCatalogo} className="poph-section__title">
                            {translate("products.catalogHeading")}
                        </h2>
                        <div className="poph-products__filters">
                            <SofaField label={translate("products.filterSearchLabel")} htmlFor="poph-produtos-busca">
                                {(controle) => (
                                    <SofaInput
                                        id={controle.id}
                                        value={filtros.busca}
                                        placeholder={translate("products.filterSearchPlaceholder")}
                                        autoComplete="off"
                                        onChange={(evento) =>
                                            setFiltros((atual) => ({
                                                ...atual,
                                                busca: evento.target.value,
                                            }))
                                        }
                                    />
                                )}
                            </SofaField>
                            <SofaField label={translate("products.filterStatusLabel")} htmlFor="poph-produtos-situacao">
                                {(controle) => (
                                    <SofaSelect
                                        id={controle.id}
                                        value={filtros.situacao}
                                        onChange={(evento) =>
                                            setFiltros((atual) => ({
                                                ...atual,
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
                                {(controle) => (
                                    <SofaSelect
                                        id={controle.id}
                                        value={filtros.categoria}
                                        onChange={(evento) =>
                                            setFiltros((atual) => ({
                                                ...atual,
                                                categoria: evento.target.value,
                                            }))
                                        }
                                    >
                                        <option value="todas">{translate("products.filterAll")}</option>
                                        {categorias.map((categoria) => (
                                            <option key={categoria.id} value={categoria.id}>
                                                {categoria.name}
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
                aberto={edicao.aberto}
                produto={edicao.produto}
                categories={categorias}
                onClose={() => setEdicao({ aberto: false, produto: null })}
                onSave={aoSalvarProduto}
            />

            <DialogDeleteProduct
                aberto={paraExcluir !== null}
                onClose={() => setParaExcluir(null)}
                org={orgId}
                produto={paraExcluir}
                agentsSee={
                    paraExcluir && agentes !== null && contagemDeAgentes.has(paraExcluir.id)
                        ? (contagemDeAgentes.get(paraExcluir.id) ?? 0)
                        : null
                }
                aoExcluir={(aviso) => {
                    if (paraExcluir) {
                        setProdutos((atual) => atual.filter((item) => item.id !== paraExcluir.id));
                    }
                    setParaExcluir(null);
                    setAvisoDoIndice(aviso);
                    void carregarAgentes();
                }}
            />
        </>
    );
}
