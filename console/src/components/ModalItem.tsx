
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

const SITUATIONS: { valor: string; labelKey: TranslationKey }[] = [
    { valor: "active", labelKey: "knowledge.statusActive" },
    { valor: "inactive", labelKey: "knowledge.statusInactive" },
    { valor: "archived", labelKey: "knowledge.statusArchived" },
];

export type ModalItemProps = {
    org: string;
    grupo: string;
    aberto: boolean;
    onClose: () => void;
    item: KnowledgeItem | null;
    onSave: (item: KnowledgeItem) => void;
};

type Errors = { titulo?: string; conteudo?: string; ordem?: string };

const FIELDS_API: Record<string, keyof Errors> = {
    title: "titulo",
    titulo: "titulo",
    content: "conteudo",
    conteudo: "conteudo",
};

function withoutDescribedBy(control: SofaFieldControlProps) {
    const { describedBy, ...rest } = control;
    return { ...rest, "aria-describedby": describedBy };
}

async function relerItem(
    org: string,
    group: string,
    id: string,
): Promise<KnowledgeItem | null> {
    const items = await listItems(org, group);
    return items.find((candidate) => candidate.id === id) ?? null;
}

export function ModalItem({ org, grupo: group, aberto: isOpen, onClose, item, onSave }: ModalItemProps) {
    const { translate: t } = useI18n();
    const editing = item !== null;
    const [title, setTitle] = useState("");
    const [content, setContent] = useState("");
    const [tags, setTags] = useState("");
    const [position, setPosition] = useState("0");
    const [situacao, setSituation] = useState("active");
    const [versao, setVersion] = useState(0);
    const [errors, setErrors] = useState<Errors>({});
    const [errorGeneral, setErrorGeneral] = useState<string | null>(null);
    const [conflict, setConflict] = useState<string | null>(null);
    const [sending, setSending] = useState(false);

    useEffect(() => {
        if (!isOpen) return;
        setTitle(item?.title ?? "");
        setContent(item?.content ?? "");
        setTags(writeTags(item?.tags ?? []));
        setPosition(String(item?.position ?? 0));
        setSituation(item?.status ?? "active");
        setVersion(item?.version ?? 0);
        setErrors({});
        setErrorGeneral(null);
        setConflict(null);
        setSending(false);
    }, [isOpen, item]);

    const characters = content.trim().length;

    function validar(): Errors {
        const freshItems: Errors = {};
        if (!title.trim()) {
            freshItems.titulo = t("knowledge.itemTitleRequired");
        } else if (title.trim().length > LIMIT_ITEM_TITLE) {
            freshItems.titulo = t("knowledge.itemTitleTooLong", { limit: LIMIT_ITEM_TITLE });
        }
        if (!content.trim()) {
            freshItems.conteudo = t("knowledge.itemContentRequired");
        } else if (characters > LIMIT_ITEM_CONTENT) {
            freshItems.conteudo = t("knowledge.itemContentTooLong", {
                limit: LIMIT_ITEM_CONTENT,
                count: characters,
            });
        }
        if (position.trim() && !Number.isInteger(Number(position))) {
            freshItems.ordem = t("knowledge.itemOrderInvalid");
        }
        return freshItems;
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
        setErrorGeneral(t("knowledge.apiErrorWithCode", {
            message: failure.message,
            code: failure.codigo,
        }));
    }

    async function save(evento?: FormEvent) {
        evento?.preventDefault();
        const freshItems = validar();
        setErrors(freshItems);
        setErrorGeneral(null);
        setConflict(null);
        if (Object.keys(freshItems).length > 0) return;

        const ordem = position.trim() ? Number(position) : 0;
        setSending(true);
        try {
            if (!item) {
                const created = await createItem(org, group, {
                    title: title.trim(),
                    content: content,
                    tags: readTags(tags),
                    position: ordem,
                });
                onSave(created);
                onClose();
                return;
            }

            const changed = await updateItem(org, group, item.id, versao, {
                title: title.trim(),
                content: content,
                tags: readTags(tags),
                position: ordem,
                status: situacao,
            });
            onSave(changed);
            onClose();
        } catch (failure) {
            if (failure instanceof ApiError && failure.status === 412 && item) {
                let versionCurrent: number | undefined;
                try {
                    const current = await relerItem(org, group, item.id);
                    versionCurrent = current?.version;
                } catch {
                }
                if (typeof versionCurrent === "number") setVersion(versionCurrent);
                setConflict(
                    typeof versionCurrent === "number"
                        ? t("knowledge.itemConflictWithVersion", { version: versionCurrent })
                        : t("knowledge.itemConflict"),
                );
                return;
            }
            if (failure instanceof ApiError) {
                showErrorApi(failure);
                return;
            }
            setErrorGeneral(t("knowledge.itemSaveUnexpected"));
        } finally {
            setSending(false);
        }
    }

    return (
        <SofaModal
            open={isOpen}
            onOpenChange={(fresh) => {
                if (!fresh && !sending) onClose();
            }}
            title={editing ? t("knowledge.itemEditTitle") : t("knowledge.itemCreateTitle")}
            description={
                editing
                    ? t("knowledge.itemEditDescription")
                    : t("knowledge.itemCreateDescription")
            }
            closeLabel={t("knowledge.itemFormClose")}
            size="lg"
            id="poph-formulario-item"
            footer={
                <>
                    <SofaButton variant="secondary" disabled={sending} onClick={onClose}>
                        {t("common.cancel")}
                    </SofaButton>
                    <SofaButton
                        variant="primary"
                        type="button"
                        onClick={() => void save()}
                        loading={sending}
                    >
                        {editing
                            ? t("knowledge.saveChanges")
                            : t("knowledge.itemCreateSubmit")}
                    </SofaButton>
                </>
            }
        >
            <form
                id="poph-form-item"
                className="poph-stack"
                style={{ gap: "var(--pop-spacing-lg)" }}
                onSubmit={(evento) => void save(evento)}
                noValidate
            >
                {conflict && (
                    <SofaBanner tone="warn" title={t("knowledge.conflictTitle")}>
                        {conflict}
                    </SofaBanner>
                )}
                {errorGeneral && (
                    <SofaBanner tone="danger" title={t("knowledge.saveFailedTitle")}>
                        {errorGeneral}
                    </SofaBanner>
                )}

                <SofaField
                    label={t("knowledge.itemTitleLabel")}
                    hint={t("knowledge.itemTitleHint", { limit: LIMIT_ITEM_TITLE })}
                    error={errors.titulo}
                    required
                    htmlFor="poph-item-titulo"
                >
                    {(control) => (
                        <SofaInput
                            {...withoutDescribedBy(control)}
                            value={title}
                            autoComplete="off"
                            onChange={(evento) => setTitle(evento.target.value)}
                        />
                    )}
                </SofaField>

                <SofaField
                    label={t("knowledge.itemContentLabel")}
                    hint={t("knowledge.itemContentHint", { limit: LIMIT_ITEM_CONTENT })}
                    error={errors.conteudo}
                    required
                    htmlFor="poph-item-conteudo"
                >
                    {(control) => (
                        <SofaTextarea
                            {...withoutDescribedBy(control)}
                            value={content}
                            rows={8}
                            onChange={(evento) => setContent(evento.target.value)}
                        />
                    )}
                </SofaField>

                <div
                    className="poph-mono-muted"
                    aria-live="polite"
                    style={{
                        marginTop: -8,
                        color:
                            characters > LIMIT_ITEM_CONTENT
                                ? "var(--pop-danger, #e5484d)"
                                : undefined,
                    }}
                >
                    {t("knowledge.itemCounter", {
                        count: characters,
                        limit: LIMIT_ITEM_CONTENT,
                    })}
                </div>

                <div className="poph-two-columns">
                    <SofaField
                        label={t("knowledge.tagsLabel")}
                        hint={t("knowledge.tagsHint")}
                        htmlFor="poph-item-tags"
                    >
                        {(control) => (
                            <SofaInput
                                {...withoutDescribedBy(control)}
                                value={tags}
                                autoComplete="off"
                                onChange={(evento) => setTags(evento.target.value)}
                            />
                        )}
                    </SofaField>
                    <SofaField
                        label={t("knowledge.orderLabel")}
                        hint={t("knowledge.orderHint")}
                        error={errors.ordem}
                        htmlFor="poph-item-ordem"
                    >
                        {(control) => (
                            <SofaInput
                                {...withoutDescribedBy(control)}
                                value={position}
                                inputMode="numeric"
                                onChange={(evento) => setPosition(evento.target.value)}
                            />
                        )}
                    </SofaField>
                </div>

                {editing && (
                    <SofaField
                        label={t("knowledge.statusLabel")}
                        hint={t("knowledge.itemStatusHint")}
                        htmlFor="poph-item-situacao"
                    >
                        {(control) => (
                            <SofaSelect
                                {...withoutDescribedBy(control)}
                                value={situacao}
                                onChange={(evento) => setSituation(evento.target.value)}
                            >
                                {SITUATIONS.map((option) => (
                                    <option key={option.valor} value={option.valor}>
                                        {t(option.labelKey)}
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
