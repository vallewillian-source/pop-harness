
import { useEffect, useState, type FormEvent } from "react";
import {
    SofaBanner,
    SofaButton,
    SofaField,
    SofaInput,
    SofaModal,
    SofaSelect,
    SofaTextarea,
    type SofaFieldControlProps,
} from "@sofacoding/sofa-ui";
import { ApiError } from "../api/client";
import type { Category } from "../api/structure";
import { useI18n } from "../i18n";
import {
    updateProduct,
    createProduct,
    getProduct,
    type ProductChange,
    type Product,
    type ProductStatus,
} from "../api/products";

export const LIMIT_NAME_PRODUCT = 160;

const SITUATIONS: { valor: ProductStatus; chave: "products.statusActive" | "products.statusInactive" | "products.statusArchived" }[] = [
    { valor: "active", chave: "products.statusActive" },
    { valor: "inactive", chave: "products.statusInactive" },
    { valor: "archived", chave: "products.statusArchived" },
];

export function centsOf(texto: string): number | null {
    const clean = texto.replace(/R\$/gi, "").replace(/\s/g, "");
    if (clean === "") return 0;
    let normal = clean;
    if (normal.includes(",")) {
        normal = normal.replace(/\./g, "").replace(",", ".");
    }
    if (!/^\d+(\.\d{1,2})?$/.test(normal)) return null;
    return Math.round(Number(normal) * 100);
}

export function brlOf(cents: number): string {
    return (cents / 100).toFixed(2).replace(".", ",");
}

export function formatBRL(cents: number): string {
    const signal = cents < 0 ? "-" : "";
    const absoluto = Math.abs(Math.trunc(cents));
    const brl = Math.floor(absoluto / 100)
        .toString()
        .replace(/\B(?=(\d{3})+(?!\d))/g, ".");
    return `${signal}R$ ${brl},${String(absoluto % 100).padStart(2, "0")}`;
}

export function listOf(texto: string): string[] {
    return texto
        .split(",")
        .map((item) => item.trim())
        .filter((item) => item.length > 0);
}

type Errors = {
    categoria?: string;
    nome?: string;
    slug?: string;
    preco?: string;
    precoTexto?: string;
    aliases?: string;
    etiquetas?: string;
    unidade?: string;
    descricao?: string;
    inclui?: string;
    exclui?: string;
    condicoes?: string;
    situacao?: string;
};

const FIELDS_API: Record<string, keyof Errors> = {
    category_id: "categoria",
    categoria: "categoria",
    name: "nome",
    nome: "nome",
    slug: "slug",
    price_cents: "preco",
    preco: "preco",
    price_text: "precoTexto",
    aliases: "aliases",
    tags: "etiquetas",
    unit: "unidade",
    description: "descricao",
    includes: "inclui",
    excludes: "exclui",
    conditions: "condicoes",
    status: "situacao",
    situacao: "situacao",
};

function withoutDescribedBy(control: SofaFieldControlProps) {
    const { describedBy, ...rest } = control;
    return { ...rest, "aria-describedby": describedBy };
}

export type ModalProductProps = {
    org: string;
    aberto: boolean;
    onClose: () => void;
    produto: Product | null;
    categories: Category[];
    onSave: (product: Product) => void;
};

export function ModalProduct({
    org,
    aberto: isOpen,
    onClose,
    produto: product,
    categories,
    onSave,
}: ModalProductProps) {
    const { translate } = useI18n();
    const editing = product !== null;
    const [category, setCategory] = useState("");
    const [nome, setName] = useState("");
    const [slug, setSlug] = useState("");
    const [aliases, setAliases] = useState("");
    const [price, setPrice] = useState("");
    const [textClient, setTextClient] = useState("");
    const [textChanged, setTextChanged] = useState(false);
    const [base, setBase] = useState<Product | null>(product);
    const [unit, setUnit] = useState("");
    const [descricao, setDescription] = useState("");
    const [includes, setIncludes] = useState("");
    const [exclui, setExclui] = useState("");
    const [conditions, setConditions] = useState("");
    const [etiquetas, setEtiquetas] = useState("");
    const [situacao, setSituation] = useState<ProductStatus>("active");
    const [versao, setVersion] = useState(0);
    const [errors, setErrors] = useState<Errors>({});
    const [errorGeneral, setErrorGeneral] = useState<string | null>(null);
    const [conflict, setConflict] = useState<string | null>(null);
    const [sending, setSending] = useState(false);

    useEffect(() => {
        if (!isOpen) return;
        setCategory(product?.category_id ?? "");
        setName(product?.name ?? "");
        setSlug(product?.slug ?? "");
        setAliases((product?.aliases ?? []).join(", "));
        setPrice(product ? brlOf(product.price_cents) : "");
        setTextClient("");
        setTextChanged(false);
        setBase(product);
        setUnit(product?.unit ?? "");
        setDescription(product?.description ?? "");
        setIncludes(product?.includes ?? "");
        setExclui(product?.excludes ?? "");
        setConditions(product?.conditions ?? "");
        setEtiquetas((product?.tags ?? []).join(", "));
        setSituation(product?.status ?? "active");
        setVersion(product?.version ?? 0);
        setErrors({});
        setErrorGeneral(null);
        setConflict(null);
        setSending(false);
    }, [isOpen, product]);

    const centsCurrent = centsOf(price);
    const priceChanged =
        base !== null && centsCurrent !== null && centsCurrent !== base.price_cents;

    function textToSend(): string | undefined {
        if (textChanged) return textClient.trim();
        if (base === null) return "";
        if (priceChanged) return "";
        return undefined;
    }

    function textClientNow(): string {
        if (centsCurrent === null) return "—";
        const calculado = formatBRL(centsCurrent);
        if (textChanged) return textClient.trim() || calculado;
        if (base !== null && !priceChanged && base.price_text.trim() !== "") {
            return base.price_text;
        }
        return calculado;
    }

    function validar(): { errors: Errors; cents: number } {
        const freshItems: Errors = {};
        if (!category) {
            freshItems.categoria = translate("products.validationCategoryRequired");
        }
        if (!nome.trim()) {
            freshItems.nome = translate("products.validationNameRequired");
        } else if (nome.trim().length > LIMIT_NAME_PRODUCT) {
            freshItems.nome = translate("products.validationNameMaxLength", { limit: LIMIT_NAME_PRODUCT });
        }
        const cents = centsOf(price);
        if (cents === null) {
            freshItems.preco = translate("products.validationPriceFormat");
        }
        return { errors: freshItems, cents: cents ?? 0 };
    }

    function showErrorApi(failure: ApiError) {
        const details = failure.detalhes as Record<string, unknown> | null | undefined;
        const campo = typeof details?.["field"] === "string" ? details["field"] : undefined;
        const motivo =
            typeof details?.["reason"] === "string" ? details["reason"] : failure.message;
        const target = campo ? FIELDS_API[campo] : undefined;
        if (target) {
            setErrors({ [target]: motivo });
            return;
        }
        setErrorGeneral(failure.message);
    }

    async function save(evento?: FormEvent) {
        evento?.preventDefault();
        const { errors: freshItems, cents } = validar();
        setErrors(freshItems);
        setErrorGeneral(null);
        setConflict(null);
        if (Object.keys(freshItems).length > 0) return;

        const common = {
            category_id: category,
            name: nome.trim(),
            aliases: listOf(aliases),
            description: descricao,
            price_cents: cents,
            unit: unit,
            includes: includes,
            excludes: exclui,
            conditions: conditions,
            tags: listOf(etiquetas),
        };
        const sendText = textToSend();

        setSending(true);
        try {
            if (!product) {
                const created = await createProduct(org, {
                    ...common,
                    price_text: sendText ?? "",
                    slug: slug.trim() || undefined,
                });
                onSave(created);
                onClose();
                return;
            }

            const change: ProductChange = { ...common, status: situacao };
            if (sendText !== undefined) {
                change.price_text = sendText;
            }
            if (slug.trim() !== product.slug) {
                change.slug = slug.trim();
            }
            const changed = await updateProduct(org, product.id, versao, change);
            onSave(changed);
            onClose();
        } catch (failure) {
            if (failure instanceof ApiError && failure.status === 412 && product) {
                let versionCurrent: number | undefined;
                try {
                    const current = await getProduct(org, product.id);
                    versionCurrent = current.version;
                    setBase(current);
                } catch {
                }
                if (typeof versionCurrent === "number") setVersion(versionCurrent);
                const versionText =
                    typeof versionCurrent === "number"
                        ? translate("products.conflictVersionSuffix", { version: versionCurrent })
                        : "";
                setConflict(translate("products.conflictMessage", { version: versionText }));
                return;
            }
            if (failure instanceof ApiError) {
                showErrorApi(failure);
                return;
            }
            setErrorGeneral(translate("products.unexpectedSaveError"));
        } finally {
            setSending(false);
        }
    }

    const withoutCategories = categories.length === 0;
    const title = editing ? translate("products.modalTitleEdit") : translate("products.modalTitleNew");

    const calculado = centsCurrent === null ? null : formatBRL(centsCurrent);
    const preview = textClientNow();
    const hasTextOwn = calculado !== null && preview !== calculado;
    const helpPriceClient =
        `${translate("products.pricePreviewPrefix", { preview: preview })} ` +
        (!textChanged && priceChanged
            ? translate("products.pricePreviewChangedHint")
            : translate("products.pricePreviewDefaultHint"));

    return (
        <SofaModal
            open={isOpen}
            onOpenChange={(fresh) => {
                if (!fresh && !sending) onClose();
            }}
            title={title}
            description={
                editing
                    ? translate("products.modalDescriptionEdit", { name: product?.name ?? "" })
                    : translate("products.modalDescriptionNew")
            }
            closeLabel={translate("products.modalClose")}
            size="lg"
            id="poph-formulario-produto"
            footer={
                <>
                    <SofaButton variant="secondary" disabled={sending} onClick={onClose}>
                        {translate("products.cancel")}
                    </SofaButton>
                    <SofaButton
                        variant="primary"
                        type="button"
                        disabled={withoutCategories}
                        onClick={() => void save()}
                        loading={sending}
                    >
                        {editing ? translate("products.saveChanges") : translate("products.createProduct")}
                    </SofaButton>
                </>
            }
        >
            <form
                id="poph-form-produto"
                className="poph-stack"
                style={{ gap: "var(--pop-spacing-lg)" }}
                onSubmit={(evento) => void save(evento)}
                noValidate
            >
                {conflict && (
                    <SofaBanner tone="warn" title={translate("products.bannerOutdatedTitle")}>
                        {conflict}
                    </SofaBanner>
                )}
                {errorGeneral && (
                    <SofaBanner tone="danger" title={translate("products.bannerSaveErrorTitle")}>
                        {errorGeneral}
                    </SofaBanner>
                )}
                {withoutCategories && (
                    <SofaBanner tone="info" title={translate("products.bannerNoCategoriesTitle")}>
                        {translate("products.bannerNoCategoriesBody")}
                    </SofaBanner>
                )}

                <SofaField
                    label={translate("products.fieldCategoryLabel")}
                    hint={translate("products.fieldCategoryHint")}
                    error={errors.categoria}
                    required
                    htmlFor="poph-produto-categoria"
                >
                    {(control) => (
                        <SofaSelect
                            {...withoutDescribedBy(control)}
                            value={category}
                            onChange={(evento) => setCategory(evento.target.value)}
                        >
                            <option value="">{translate("products.fieldCategoryPlaceholder")}</option>
                            {categories.map((item) => (
                                <option key={item.id} value={item.id}>
                                    {item.name}
                                </option>
                            ))}
                        </SofaSelect>
                    )}
                </SofaField>

                <SofaField
                    label={translate("products.fieldNameLabel")}
                    hint={translate("products.fieldNameHint", { limit: LIMIT_NAME_PRODUCT })}
                    error={errors.nome}
                    required
                    htmlFor="poph-produto-nome"
                >
                    {(control) => (
                        <SofaInput
                            {...withoutDescribedBy(control)}
                            value={nome}
                            autoComplete="off"
                            onChange={(evento) => setName(evento.target.value)}
                        />
                    )}
                </SofaField>

                <SofaField
                    label={translate("products.fieldSlugLabel")}
                    hint={translate("products.fieldSlugHint")}
                    error={errors.slug}
                    htmlFor="poph-produto-slug"
                >
                    {(control) => (
                        <SofaInput
                            {...withoutDescribedBy(control)}
                            value={slug}
                            autoComplete="off"
                            onChange={(evento) => setSlug(evento.target.value)}
                        />
                    )}
                </SofaField>

                <SofaField
                    label={translate("products.fieldAliasesLabel")}
                    hint={translate("products.fieldAliasesHint")}
                    error={errors.aliases}
                    htmlFor="poph-produto-aliases"
                >
                    {(control) => (
                        <SofaInput
                            {...withoutDescribedBy(control)}
                            value={aliases}
                            autoComplete="off"
                            onChange={(evento) => setAliases(evento.target.value)}
                        />
                    )}
                </SofaField>

                <div className="poph-two-columns">
                    <SofaField
                        label={translate("products.fieldPriceLabel")}
                        hint={translate("products.fieldPriceHint")}
                        error={errors.preco}
                        htmlFor="poph-produto-preco"
                    >
                        {(control) => (
                            <SofaInput
                                {...withoutDescribedBy(control)}
                                value={price}
                                inputMode="decimal"
                                autoComplete="off"
                                onChange={(evento) => setPrice(evento.target.value)}
                            />
                        )}
                    </SofaField>

                    <SofaField
                        label={translate("products.fieldPriceTextLabel")}
                        hint={helpPriceClient}
                        error={errors.precoTexto}
                        htmlFor="poph-produto-preco-texto"
                    >
                        {(control) => (
                            <SofaInput
                                {...withoutDescribedBy(control)}
                                value={textClient}
                                autoComplete="off"
                                onChange={(evento) => {
                                    setTextChanged(true);
                                    setTextClient(evento.target.value);
                                }}
                            />
                        )}
                    </SofaField>
                </div>
                {hasTextOwn && (
                    <div>
                        <SofaButton
                            variant="subtle"
                            size="sm"
                            type="button"
                            disabled={sending}
                            onClick={() => {
                                setTextChanged(true);
                                setTextClient("");
                            }}
                        >
                            {translate("products.resetPriceText")}
                        </SofaButton>
                    </div>
                )}

                <SofaField
                    label={translate("products.fieldUnitLabel")}
                    hint={translate("products.fieldUnitHint")}
                    error={errors.unidade}
                    htmlFor="poph-produto-unidade"
                >
                    {(control) => (
                        <SofaInput
                            {...withoutDescribedBy(control)}
                            value={unit}
                            autoComplete="off"
                            onChange={(evento) => setUnit(evento.target.value)}
                        />
                    )}
                </SofaField>

                <SofaField
                    label={translate("products.fieldDescriptionLabel")}
                    error={errors.descricao}
                    htmlFor="poph-produto-descricao"
                >
                    {(control) => (
                        <SofaTextarea
                            {...withoutDescribedBy(control)}
                            value={descricao}
                            rows={2}
                            onChange={(evento) => setDescription(evento.target.value)}
                        />
                    )}
                </SofaField>

                <div className="poph-two-columns">
                    <SofaField label={translate("products.fieldIncludesLabel")} error={errors.inclui} htmlFor="poph-produto-inclui">
                        {(control) => (
                            <SofaTextarea
                                {...withoutDescribedBy(control)}
                                value={includes}
                                rows={2}
                                onChange={(evento) => setIncludes(evento.target.value)}
                            />
                        )}
                    </SofaField>
                    <SofaField label={translate("products.fieldExcludesLabel")} error={errors.exclui} htmlFor="poph-produto-exclui">
                        {(control) => (
                            <SofaTextarea
                                {...withoutDescribedBy(control)}
                                value={exclui}
                                rows={2}
                                onChange={(evento) => setExclui(evento.target.value)}
                            />
                        )}
                    </SofaField>
                </div>

                <SofaField
                    label={translate("products.fieldConditionsLabel")}
                    hint={translate("products.fieldConditionsHint")}
                    error={errors.condicoes}
                    htmlFor="poph-produto-condicoes"
                >
                    {(control) => (
                        <SofaTextarea
                            {...withoutDescribedBy(control)}
                            value={conditions}
                            rows={2}
                            onChange={(evento) => setConditions(evento.target.value)}
                        />
                    )}
                </SofaField>

                <SofaField
                    label={translate("products.fieldTagsLabel")}
                    hint={translate("products.fieldTagsHint")}
                    error={errors.etiquetas}
                    htmlFor="poph-produto-etiquetas"
                >
                    {(control) => (
                        <SofaInput
                            {...withoutDescribedBy(control)}
                            value={etiquetas}
                            autoComplete="off"
                            onChange={(evento) => setEtiquetas(evento.target.value)}
                        />
                    )}
                </SofaField>

                {editing && (
                    <SofaField
                        label={translate("products.fieldStatusLabel")}
                        hint={translate("products.fieldStatusHint")}
                        error={errors.situacao}
                        htmlFor="poph-produto-situacao"
                    >
                        {(control) => (
                            <SofaSelect
                                {...withoutDescribedBy(control)}
                                value={situacao}
                                onChange={(evento) =>
                                    setSituation(evento.target.value as ProductStatus)
                                }
                            >
                                {SITUATIONS.map((option) => (
                                    <option key={option.valor} value={option.valor}>
                                        {translate(option.chave)}
                                    </option>
                                ))}
                            </SofaSelect>
                        )}
                    </SofaField>
                )}
            </form>
        </SofaModal>
    );
}
