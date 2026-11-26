/**
 * Formulário de item (trecho) da base, em modal (HR-41).
 *
 * Título até 200 caracteres e conteúdo até 1000 — o mesmo limite do servidor
 * (`LIMITE_CONTEUDO_ITEM`), mostrado como contador ao vivo. Concorrência otimista igual às outras
 * telas: `PATCH` com `If-Match`; no `412` a tela relê o item, adota a versão atual e mantém o texto
 * digitado.
 *
 * A resposta de `POST`/`PATCH` traz `index_warning` quando o índice não pôde ser sincronizado; a
 * tela repassa o aviso para quem chamou (a aba mostra o estado e oferece reindexar).
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
import {
    updateItem,
    createItem,
    writeTags,
    readTags,
    LIMIT_ITEM_CONTENT,
    LIMIT_ITEM_TITLE,
    listItems,
    type KnowledgeItem,
} from "../api/knowledge";
import { useI18n } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";

const SITUACOES: { valor: string; labelKey: TranslationKey }[] = [
    { valor: "active", labelKey: "knowledge.statusActive" },
    { valor: "inactive", labelKey: "knowledge.statusInactive" },
    { valor: "archived", labelKey: "knowledge.statusArchived" },
];

export type ModalItemProps = {
    org: string;
    grupo: string;
    aberto: boolean;
    aoFechar: () => void;
    /** Item em edição; `null` cria um novo. */
    item: KnowledgeItem | null;
    /** Chamado com o item devolvido pela API (criado ou alterado). */
    aoSalvar: (item: KnowledgeItem) => void;
};

type Erros = { titulo?: string; conteudo?: string; ordem?: string };

/** Nome do campo da API → campo do formulário. */
const CAMPOS_DA_API: Record<string, keyof Erros> = {
    title: "titulo",
    titulo: "titulo",
    content: "conteudo",
    conteudo: "conteudo",
};

/** Ligações do `SofaField` sem `describedBy`, que vira `aria-describedby` no controle. */
function semDescribedBy(controle: SofaFieldControlProps) {
    const { describedBy, ...resto } = controle;
    return { ...resto, "aria-describedby": describedBy };
}

/** Releitura usada no `412`: o item vivo (pode ter mais de uma página por grupo). */
async function relerItem(
    org: string,
    grupo: string,
    id: string,
): Promise<KnowledgeItem | null> {
    const itens = await listItems(org, grupo);
    return itens.find((candidato) => candidato.id === id) ?? null;
}

export function ModalItem({ org, grupo, aberto, aoFechar, item, aoSalvar }: ModalItemProps) {
    const { translate: t } = useI18n();
    const editando = item !== null;
    const [titulo, setTitulo] = useState("");
    const [conteudo, setConteudo] = useState("");
    const [tags, setTags] = useState("");
    const [posicao, setPosicao] = useState("0");
    const [situacao, setSituacao] = useState("active");
    const [versao, setVersao] = useState(0);
    const [erros, setErros] = useState<Erros>({});
    const [erroGeral, setErroGeral] = useState<string | null>(null);
    const [conflito, setConflito] = useState<string | null>(null);
    const [enviando, setEnviando] = useState(false);

    useEffect(() => {
        if (!aberto) return;
        setTitulo(item?.title ?? "");
        setConteudo(item?.content ?? "");
        setTags(writeTags(item?.tags ?? []));
        setPosicao(String(item?.position ?? 0));
        setSituacao(item?.status ?? "active");
        setVersao(item?.version ?? 0);
        setErros({});
        setErroGeral(null);
        setConflito(null);
        setEnviando(false);
    }, [aberto, item]);

    const caracteres = conteudo.trim().length;

    function validar(): Erros {
        const novos: Erros = {};
        if (!titulo.trim()) {
            novos.titulo = t("knowledge.itemTitleRequired");
        } else if (titulo.trim().length > LIMIT_ITEM_TITLE) {
            novos.titulo = t("knowledge.itemTitleTooLong", { limit: LIMIT_ITEM_TITLE });
        }
        if (!conteudo.trim()) {
            novos.conteudo = t("knowledge.itemContentRequired");
        } else if (caracteres > LIMIT_ITEM_CONTENT) {
            novos.conteudo = t("knowledge.itemContentTooLong", {
                limit: LIMIT_ITEM_CONTENT,
                count: caracteres,
            });
        }
        if (posicao.trim() && !Number.isInteger(Number(posicao))) {
            novos.ordem = t("knowledge.itemOrderInvalid");
        }
        return novos;
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
        setErroGeral(t("knowledge.apiErrorWithCode", {
            message: falha.message,
            code: falha.codigo,
        }));
    }

    async function salvar(evento?: FormEvent) {
        evento?.preventDefault();
        const novos = validar();
        setErros(novos);
        setErroGeral(null);
        setConflito(null);
        if (Object.keys(novos).length > 0) return;

        const ordem = posicao.trim() ? Number(posicao) : 0;
        setEnviando(true);
        try {
            if (!item) {
                const criado = await createItem(org, grupo, {
                    title: titulo.trim(),
                    content: conteudo,
                    tags: readTags(tags),
                    position: ordem,
                });
                aoSalvar(criado);
                aoFechar();
                return;
            }

            const alterado = await updateItem(org, grupo, item.id, versao, {
                title: titulo.trim(),
                content: conteudo,
                tags: readTags(tags),
                position: ordem,
                status: situacao,
            });
            aoSalvar(alterado);
            aoFechar();
        } catch (falha) {
            if (falha instanceof ApiError && falha.status === 412 && item) {
                let versaoAtual: number | undefined;
                try {
                    const atual = await relerItem(org, grupo, item.id);
                    versaoAtual = atual?.version;
                } catch {
                    // Sem a releitura, a próxima tentativa devolve 412 de novo; nada se perde.
                }
                if (typeof versaoAtual === "number") setVersao(versaoAtual);
                setConflito(
                    typeof versaoAtual === "number"
                        ? t("knowledge.itemConflictWithVersion", { version: versaoAtual })
                        : t("knowledge.itemConflict"),
                );
                return;
            }
            if (falha instanceof ApiError) {
                mostrarErroDaApi(falha);
                return;
            }
            setErroGeral(t("knowledge.itemSaveUnexpected"));
        } finally {
            setEnviando(false);
        }
    }

    return (
        <SofaModal
            open={aberto}
            onOpenChange={(novo) => {
                if (!novo && !enviando) aoFechar();
            }}
            title={editando ? t("knowledge.itemEditTitle") : t("knowledge.itemCreateTitle")}
            description={
                editando
                    ? t("knowledge.itemEditDescription")
                    : t("knowledge.itemCreateDescription")
            }
            closeLabel={t("knowledge.itemFormClose")}
            size="lg"
            id="poph-formulario-item"
            footer={
                <>
                    <SofaButton variant="secondary" disabled={enviando} onClick={aoFechar}>
                        {t("common.cancel")}
                    </SofaButton>
                    <SofaButton
                        variant="primary"
                        type="button"
                        onClick={() => void salvar()}
                        loading={enviando}
                    >
                        {editando
                            ? t("knowledge.saveChanges")
                            : t("knowledge.itemCreateSubmit")}
                    </SofaButton>
                </>
            }
        >
            <form
                id="poph-form-item"
                className="poph-pilha"
                style={{ gap: "var(--pop-spacing-lg)" }}
                onSubmit={(evento) => void salvar(evento)}
                noValidate
            >
                {conflito && (
                    <SofaBanner tone="warn" title={t("knowledge.conflictTitle")}>
                        {conflito}
                    </SofaBanner>
                )}
                {erroGeral && (
                    <SofaBanner tone="danger" title={t("knowledge.saveFailedTitle")}>
                        {erroGeral}
                    </SofaBanner>
                )}

                <SofaField
                    label={t("knowledge.itemTitleLabel")}
                    hint={t("knowledge.itemTitleHint", { limit: LIMIT_ITEM_TITLE })}
                    error={erros.titulo}
                    required
                    htmlFor="poph-item-titulo"
                >
                    {(controle) => (
                        <SofaInput
                            {...semDescribedBy(controle)}
                            value={titulo}
                            autoComplete="off"
                            onChange={(evento) => setTitulo(evento.target.value)}
                        />
                    )}
                </SofaField>

                <SofaField
                    label={t("knowledge.itemContentLabel")}
                    hint={t("knowledge.itemContentHint", { limit: LIMIT_ITEM_CONTENT })}
                    error={erros.conteudo}
                    required
                    htmlFor="poph-item-conteudo"
                >
                    {(controle) => (
                        <SofaTextarea
                            {...semDescribedBy(controle)}
                            value={conteudo}
                            rows={8}
                            onChange={(evento) => setConteudo(evento.target.value)}
                        />
                    )}
                </SofaField>

                <div
                    className="poph-mono-mudo"
                    aria-live="polite"
                    style={{
                        marginTop: -8,
                        color:
                            caracteres > LIMIT_ITEM_CONTENT
                                ? "var(--pop-danger, #e5484d)"
                                : undefined,
                    }}
                >
                    {t("knowledge.itemCounter", {
                        count: caracteres,
                        limit: LIMIT_ITEM_CONTENT,
                    })}
                </div>

                <div className="poph-duas-colunas">
                    <SofaField
                        label={t("knowledge.tagsLabel")}
                        hint={t("knowledge.tagsHint")}
                        htmlFor="poph-item-tags"
                    >
                        {(controle) => (
                            <SofaInput
                                {...semDescribedBy(controle)}
                                value={tags}
                                autoComplete="off"
                                onChange={(evento) => setTags(evento.target.value)}
                            />
                        )}
                    </SofaField>
                    <SofaField
                        label={t("knowledge.orderLabel")}
                        hint={t("knowledge.orderHint")}
                        error={erros.ordem}
                        htmlFor="poph-item-ordem"
                    >
                        {(controle) => (
                            <SofaInput
                                {...semDescribedBy(controle)}
                                value={posicao}
                                inputMode="numeric"
                                onChange={(evento) => setPosicao(evento.target.value)}
                            />
                        )}
                    </SofaField>
                </div>

                {editando && (
                    <SofaField
                        label={t("knowledge.statusLabel")}
                        hint={t("knowledge.itemStatusHint")}
                        htmlFor="poph-item-situacao"
                    >
                        {(controle) => (
                            <SofaSelect
                                {...semDescribedBy(controle)}
                                value={situacao}
                                onChange={(evento) => setSituacao(evento.target.value)}
                            >
                                {SITUACOES.map((opcao) => (
                                    <option key={opcao.valor} value={opcao.valor}>
                                        {t(opcao.labelKey)}
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
