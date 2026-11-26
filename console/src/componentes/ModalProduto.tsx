/**
 * Formulário de produto do catálogo (criar e editar), em modal (HR-42).
 *
 * Mesmo molde do `ModalEstrutura`: criar faz `POST` (sem situação, que a API recusa na criação);
 * editar faz `PATCH` com `If-Match` da versão lida. No `412` a tela relê o produto, passa a usar a
 * versão atual e **mantém tudo o que foi digitado** para o operador revisar.
 *
 * Preço: a tela recebe em reais (`450` ou `450,00`) e grava em centavos, como a API espera.
 *
 * Preço como o cliente vê (`price_text`) — a API devolve sempre o texto calculado, sem dizer se
 * alguém digitou um texto próprio. Por isso o campo **começa vazio** e o `price_text` só vai no
 * corpo quando:
 * - o operador digitou no campo (vai o texto digitado; vazio volta a calcular pelo valor);
 * - o preço em centavos mudou e o campo não foi mexido: vai vazio, para o cliente não continuar
 *   vendo o valor antigo quando havia um texto próprio gravado.
 * Nos outros casos o `price_text` não vai, e o servidor mantém o que já está gravado.
 */

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

export const LIMITE_NOME_PRODUTO = 160;

const SITUACOES: { valor: ProductStatus; chave: "products.statusActive" | "products.statusInactive" | "products.statusArchived" }[] = [
    { valor: "active", chave: "products.statusActive" },
    { valor: "inactive", chave: "products.statusInactive" },
    { valor: "archived", chave: "products.statusArchived" },
];

/**
 * Converte o preço digitado em reais para centavos. Aceita `450`, `450,00`, `1.234,50` e `R$ 450`.
 * Devolve `null` quando o texto não é um valor (a tela mostra o erro no campo).
 */
export function centavosDe(texto: string): number | null {
    const limpo = texto.replace(/R\$/gi, "").replace(/\s/g, "");
    if (limpo === "") return 0;
    let normal = limpo;
    if (normal.includes(",")) {
        normal = normal.replace(/\./g, "").replace(",", ".");
    }
    if (!/^\d+(\.\d{1,2})?$/.test(normal)) return null;
    return Math.round(Number(normal) * 100);
}

/** Centavos para o campo de reais, com vírgula decimal (`45000` → `450,00`). */
export function reaisDe(centavos: number): string {
    return (centavos / 100).toFixed(2).replace(".", ",");
}

/**
 * Texto de preço que o servidor monta sozinho (`890000` → `R$ 8.900,00`). Mesma regra de
 * `formatar_reais` no servidor, só para a prévia da tela; quem grava continua sendo o servidor.
 */
export function formatarReais(centavos: number): string {
    const sinal = centavos < 0 ? "-" : "";
    const absoluto = Math.abs(Math.trunc(centavos));
    const reais = Math.floor(absoluto / 100)
        .toString()
        .replace(/\B(?=(\d{3})+(?!\d))/g, ".");
    return `${sinal}R$ ${reais},${String(absoluto % 100).padStart(2, "0")}`;
}

/** Lista separada por vírgula: tira espaços e vazios. */
export function listaDe(texto: string): string[] {
    return texto
        .split(",")
        .map((item) => item.trim())
        .filter((item) => item.length > 0);
}

type Erros = {
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

/** Campo da API → campo do formulário. A API usa o nome do corpo JSON. */
const CAMPOS_DA_API: Record<string, keyof Erros> = {
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

/** Ligações do `SofaField` sem `describedBy`, que vira `aria-describedby` no controle. */
function semDescribedBy(controle: SofaFieldControlProps) {
    const { describedBy, ...resto } = controle;
    return { ...resto, "aria-describedby": describedBy };
}

export type ModalProdutoProps = {
    org: string;
    aberto: boolean;
    aoFechar: () => void;
    /** Produto em edição; `null` cria um novo. */
    produto: Product | null;
    /** Categorias da org para escolher; sem categoria não há como criar produto. */
    categorias: Category[];
    /** Chamado com o produto devolvido pela API (criado ou alterado). */
    aoSalvar: (produto: Product) => void;
};

export function ModalProduto({
    org,
    aberto,
    aoFechar,
    produto,
    categorias,
    aoSalvar,
}: ModalProdutoProps) {
    const { translate } = useI18n();
    const editando = produto !== null;
    const [categoria, setCategoria] = useState("");
    const [nome, setNome] = useState("");
    const [slug, setSlug] = useState("");
    const [aliases, setAliases] = useState("");
    const [preco, setPreco] = useState("");
    const [textoCliente, setTextoCliente] = useState("");
    /** O operador mexeu no campo "Preço como o cliente vê"? Sem isso, o texto não vai no corpo. */
    const [textoMexido, setTextoMexido] = useState(false);
    /** Produto contra o qual a tela compara o preço digitado (o 412 troca pelo valor atual). */
    const [base, setBase] = useState<Product | null>(produto);
    const [unidade, setUnidade] = useState("");
    const [descricao, setDescricao] = useState("");
    const [inclui, setInclui] = useState("");
    const [exclui, setExclui] = useState("");
    const [condicoes, setCondicoes] = useState("");
    const [etiquetas, setEtiquetas] = useState("");
    const [situacao, setSituacao] = useState<ProductStatus>("active");
    const [versao, setVersao] = useState(0);
    const [erros, setErros] = useState<Erros>({});
    const [erroGeral, setErroGeral] = useState<string | null>(null);
    const [conflito, setConflito] = useState<string | null>(null);
    const [enviando, setEnviando] = useState(false);

    useEffect(() => {
        if (!aberto) return;
        setCategoria(produto?.category_id ?? "");
        setNome(produto?.name ?? "");
        setSlug(produto?.slug ?? "");
        setAliases((produto?.aliases ?? []).join(", "));
        setPreco(produto ? reaisDe(produto.price_cents) : "");
        setTextoCliente("");
        setTextoMexido(false);
        setBase(produto);
        setUnidade(produto?.unit ?? "");
        setDescricao(produto?.description ?? "");
        setInclui(produto?.includes ?? "");
        setExclui(produto?.excludes ?? "");
        setCondicoes(produto?.conditions ?? "");
        setEtiquetas((produto?.tags ?? []).join(", "));
        setSituacao(produto?.status ?? "active");
        setVersao(produto?.version ?? 0);
        setErros({});
        setErroGeral(null);
        setConflito(null);
        setEnviando(false);
    }, [aberto, produto]);

    /** Preço digitado em centavos, ou `null` enquanto o texto não é um valor. */
    const centavosAtuais = centavosDe(preco);
    /** Preço mudou em relação ao que a tela está comparando (só existe na edição). */
    const precoMudou =
        base !== null && centavosAtuais !== null && centavosAtuais !== base.price_cents;

    /**
     * O que vai em `price_text` no corpo. `undefined` = não enviar: o servidor mantém o texto que
     * já está gravado. `""` = calcular pelo valor.
     */
    function textoParaEnviar(): string | undefined {
        if (textoMexido) return textoCliente.trim();
        if (base === null) return "";
        if (precoMudou) return "";
        return undefined;
    }

    /** Texto que o cliente vê depois de salvar, para a prévia. */
    function textoDoClienteAgora(): string {
        if (centavosAtuais === null) return "—";
        const calculado = formatarReais(centavosAtuais);
        if (textoMexido) return textoCliente.trim() || calculado;
        if (base !== null && !precoMudou && base.price_text.trim() !== "") {
            return base.price_text;
        }
        return calculado;
    }

    function validar(): { erros: Erros; centavos: number } {
        const novos: Erros = {};
        if (!categoria) {
            novos.categoria = translate("products.validationCategoryRequired");
        }
        if (!nome.trim()) {
            novos.nome = translate("products.validationNameRequired");
        } else if (nome.trim().length > LIMITE_NOME_PRODUTO) {
            novos.nome = translate("products.validationNameMaxLength", { limit: LIMITE_NOME_PRODUTO });
        }
        const centavos = centavosDe(preco);
        if (centavos === null) {
            novos.preco = translate("products.validationPriceFormat");
        }
        return { erros: novos, centavos: centavos ?? 0 };
    }

    /** Põe o erro da API no campo que ele aponta; o que não tem campo vai para o aviso geral. */
    function mostrarErroDaApi(falha: ApiError) {
        const detalhes = falha.detalhes as Record<string, unknown> | null | undefined;
        const campo = typeof detalhes?.["field"] === "string" ? detalhes["field"] : undefined;
        const motivo =
            typeof detalhes?.["reason"] === "string" ? detalhes["reason"] : falha.message;
        const alvo = campo ? CAMPOS_DA_API[campo] : undefined;
        if (alvo) {
            setErros({ [alvo]: motivo });
            return;
        }
        setErroGeral(falha.message);
    }

    async function salvar(evento?: FormEvent) {
        evento?.preventDefault();
        const { erros: novos, centavos } = validar();
        setErros(novos);
        setErroGeral(null);
        setConflito(null);
        if (Object.keys(novos).length > 0) return;

        const comum = {
            category_id: categoria,
            name: nome.trim(),
            aliases: listaDe(aliases),
            description: descricao,
            price_cents: centavos,
            unit: unidade,
            includes: inclui,
            excludes: exclui,
            conditions: condicoes,
            tags: listaDe(etiquetas),
        };
        const textoEnvio = textoParaEnviar();

        setEnviando(true);
        try {
            if (!produto) {
                const criado = await createProduct(org, {
                    ...comum,
                    price_text: textoEnvio ?? "",
                    slug: slug.trim() || undefined,
                });
                aoSalvar(criado);
                aoFechar();
                return;
            }

            // O slug só vai no corpo se o operador mexeu nele: o original é válido e não precisa voltar.
            const mudanca: ProductChange = { ...comum, status: situacao };
            if (textoEnvio !== undefined) {
                mudanca.price_text = textoEnvio;
            }
            if (slug.trim() !== produto.slug) {
                mudanca.slug = slug.trim();
            }
            const alterado = await updateProduct(org, produto.id, versao, mudanca);
            aoSalvar(alterado);
            aoFechar();
        } catch (falha) {
            if (falha instanceof ApiError && falha.status === 412 && produto) {
                // Relê para continuar de uma versão válida; o que foi digitado não é tocado.
                // A releitura também vira a referência do preço: o valor que o operador digitou
                // é comparado com o que está gravado agora, não com o de quando ele abriu a tela.
                let versaoAtual: number | undefined;
                try {
                    const atual = await getProduct(org, produto.id);
                    versaoAtual = atual.version;
                    setBase(atual);
                } catch {
                    // Sem a releitura, a próxima tentativa devolve 412 de novo; nada se perde.
                }
                if (typeof versaoAtual === "number") setVersao(versaoAtual);
                const versionText =
                    typeof versaoAtual === "number"
                        ? translate("products.conflictVersionSuffix", { version: versaoAtual })
                        : "";
                setConflito(translate("products.conflictMessage", { version: versionText }));
                return;
            }
            if (falha instanceof ApiError) {
                mostrarErroDaApi(falha);
                return;
            }
            setErroGeral(translate("products.unexpectedSaveError"));
        } finally {
            setEnviando(false);
        }
    }

    const semCategorias = categorias.length === 0;
    const titulo = editando ? translate("products.modalTitleEdit") : translate("products.modalTitleNew");

    const calculado = centavosAtuais === null ? null : formatarReais(centavosAtuais);
    const previa = textoDoClienteAgora();
    /** Há um texto próprio no lugar do calculado: o botão de voltar ao cálculo aparece. */
    const temTextoProprio = calculado !== null && previa !== calculado;
    const ajudaPrecoCliente =
        `${translate("products.pricePreviewPrefix", { preview: previa })} ` +
        (!textoMexido && precoMudou
            ? translate("products.pricePreviewChangedHint")
            : translate("products.pricePreviewDefaultHint"));

    return (
        <SofaModal
            open={aberto}
            onOpenChange={(novo) => {
                if (!novo && !enviando) aoFechar();
            }}
            title={titulo}
            description={
                editando
                    ? translate("products.modalDescriptionEdit", { name: produto?.name ?? "" })
                    : translate("products.modalDescriptionNew")
            }
            closeLabel={translate("products.modalClose")}
            size="lg"
            id="poph-formulario-produto"
            footer={
                <>
                    <SofaButton variant="secondary" disabled={enviando} onClick={aoFechar}>
                        {translate("products.cancel")}
                    </SofaButton>
                    <SofaButton
                        variant="primary"
                        type="button"
                        disabled={semCategorias}
                        onClick={() => void salvar()}
                        loading={enviando}
                    >
                        {editando ? translate("products.saveChanges") : translate("products.createProduct")}
                    </SofaButton>
                </>
            }
        >
            <form
                id="poph-form-produto"
                className="poph-pilha"
                style={{ gap: "var(--pop-spacing-lg)" }}
                onSubmit={(evento) => void salvar(evento)}
                noValidate
            >
                {conflito && (
                    <SofaBanner tone="warn" title={translate("products.bannerOutdatedTitle")}>
                        {conflito}
                    </SofaBanner>
                )}
                {erroGeral && (
                    <SofaBanner tone="danger" title={translate("products.bannerSaveErrorTitle")}>
                        {erroGeral}
                    </SofaBanner>
                )}
                {semCategorias && (
                    <SofaBanner tone="info" title={translate("products.bannerNoCategoriesTitle")}>
                        {translate("products.bannerNoCategoriesBody")}
                    </SofaBanner>
                )}

                <SofaField
                    label={translate("products.fieldCategoryLabel")}
                    hint={translate("products.fieldCategoryHint")}
                    error={erros.categoria}
                    required
                    htmlFor="poph-produto-categoria"
                >
                    {(controle) => (
                        <SofaSelect
                            {...semDescribedBy(controle)}
                            value={categoria}
                            onChange={(evento) => setCategoria(evento.target.value)}
                        >
                            <option value="">{translate("products.fieldCategoryPlaceholder")}</option>
                            {categorias.map((item) => (
                                <option key={item.id} value={item.id}>
                                    {item.name}
                                </option>
                            ))}
                        </SofaSelect>
                    )}
                </SofaField>

                <SofaField
                    label={translate("products.fieldNameLabel")}
                    hint={translate("products.fieldNameHint", { limit: LIMITE_NOME_PRODUTO })}
                    error={erros.nome}
                    required
                    htmlFor="poph-produto-nome"
                >
                    {(controle) => (
                        <SofaInput
                            {...semDescribedBy(controle)}
                            value={nome}
                            autoComplete="off"
                            onChange={(evento) => setNome(evento.target.value)}
                        />
                    )}
                </SofaField>

                <SofaField
                    label={translate("products.fieldSlugLabel")}
                    hint={translate("products.fieldSlugHint")}
                    error={erros.slug}
                    htmlFor="poph-produto-slug"
                >
                    {(controle) => (
                        <SofaInput
                            {...semDescribedBy(controle)}
                            value={slug}
                            autoComplete="off"
                            onChange={(evento) => setSlug(evento.target.value)}
                        />
                    )}
                </SofaField>

                <SofaField
                    label={translate("products.fieldAliasesLabel")}
                    hint={translate("products.fieldAliasesHint")}
                    error={erros.aliases}
                    htmlFor="poph-produto-aliases"
                >
                    {(controle) => (
                        <SofaInput
                            {...semDescribedBy(controle)}
                            value={aliases}
                            autoComplete="off"
                            onChange={(evento) => setAliases(evento.target.value)}
                        />
                    )}
                </SofaField>

                <div className="poph-duas-colunas">
                    <SofaField
                        label={translate("products.fieldPriceLabel")}
                        hint={translate("products.fieldPriceHint")}
                        error={erros.preco}
                        htmlFor="poph-produto-preco"
                    >
                        {(controle) => (
                            <SofaInput
                                {...semDescribedBy(controle)}
                                value={preco}
                                inputMode="decimal"
                                autoComplete="off"
                                onChange={(evento) => setPreco(evento.target.value)}
                            />
                        )}
                    </SofaField>

                    <SofaField
                        label={translate("products.fieldPriceTextLabel")}
                        hint={ajudaPrecoCliente}
                        error={erros.precoTexto}
                        htmlFor="poph-produto-preco-texto"
                    >
                        {(controle) => (
                            <SofaInput
                                {...semDescribedBy(controle)}
                                value={textoCliente}
                                autoComplete="off"
                                onChange={(evento) => {
                                    setTextoMexido(true);
                                    setTextoCliente(evento.target.value);
                                }}
                            />
                        )}
                    </SofaField>
                </div>
                {temTextoProprio && (
                    <div>
                        <SofaButton
                            variant="subtle"
                            size="sm"
                            type="button"
                            disabled={enviando}
                            onClick={() => {
                                setTextoMexido(true);
                                setTextoCliente("");
                            }}
                        >
                            {translate("products.resetPriceText")}
                        </SofaButton>
                    </div>
                )}

                <SofaField
                    label={translate("products.fieldUnitLabel")}
                    hint={translate("products.fieldUnitHint")}
                    error={erros.unidade}
                    htmlFor="poph-produto-unidade"
                >
                    {(controle) => (
                        <SofaInput
                            {...semDescribedBy(controle)}
                            value={unidade}
                            autoComplete="off"
                            onChange={(evento) => setUnidade(evento.target.value)}
                        />
                    )}
                </SofaField>

                <SofaField
                    label={translate("products.fieldDescriptionLabel")}
                    error={erros.descricao}
                    htmlFor="poph-produto-descricao"
                >
                    {(controle) => (
                        <SofaTextarea
                            {...semDescribedBy(controle)}
                            value={descricao}
                            rows={2}
                            onChange={(evento) => setDescricao(evento.target.value)}
                        />
                    )}
                </SofaField>

                <div className="poph-duas-colunas">
                    <SofaField label={translate("products.fieldIncludesLabel")} error={erros.inclui} htmlFor="poph-produto-inclui">
                        {(controle) => (
                            <SofaTextarea
                                {...semDescribedBy(controle)}
                                value={inclui}
                                rows={2}
                                onChange={(evento) => setInclui(evento.target.value)}
                            />
                        )}
                    </SofaField>
                    <SofaField label={translate("products.fieldExcludesLabel")} error={erros.exclui} htmlFor="poph-produto-exclui">
                        {(controle) => (
                            <SofaTextarea
                                {...semDescribedBy(controle)}
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
                    error={erros.condicoes}
                    htmlFor="poph-produto-condicoes"
                >
                    {(controle) => (
                        <SofaTextarea
                            {...semDescribedBy(controle)}
                            value={condicoes}
                            rows={2}
                            onChange={(evento) => setCondicoes(evento.target.value)}
                        />
                    )}
                </SofaField>

                <SofaField
                    label={translate("products.fieldTagsLabel")}
                    hint={translate("products.fieldTagsHint")}
                    error={erros.etiquetas}
                    htmlFor="poph-produto-etiquetas"
                >
                    {(controle) => (
                        <SofaInput
                            {...semDescribedBy(controle)}
                            value={etiquetas}
                            autoComplete="off"
                            onChange={(evento) => setEtiquetas(evento.target.value)}
                        />
                    )}
                </SofaField>

                {editando && (
                    <SofaField
                        label={translate("products.fieldStatusLabel")}
                        hint={translate("products.fieldStatusHint")}
                        error={erros.situacao}
                        htmlFor="poph-produto-situacao"
                    >
                        {(controle) => (
                            <SofaSelect
                                {...semDescribedBy(controle)}
                                value={situacao}
                                onChange={(evento) =>
                                    setSituacao(evento.target.value as ProductStatus)
                                }
                            >
                                {SITUACOES.map((opcao) => (
                                    <option key={opcao.valor} value={opcao.valor}>
                                        {translate(opcao.chave)}
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
