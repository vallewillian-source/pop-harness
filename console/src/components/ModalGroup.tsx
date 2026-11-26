
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
    updateGroup,
    createGroup,
    getGroup,
    LIMIT_GROUP_NAME,
    type KnowledgeGroup,
} from "../api/knowledge";
import { useI18n } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";

const SITUATIONS: { value: string; labelKey: TranslationKey }[] = [
    { value: "active", labelKey: "knowledge.statusActive" },
    { value: "inactive", labelKey: "knowledge.statusInactive" },
    { value: "archived", labelKey: "knowledge.statusArchived" },
];

export type ModalGroupProps = {
    org: string;
    opened: boolean;
    onClose: () => void;
    group: KnowledgeGroup | null;
    onSave: (group: KnowledgeGroup) => void;
};

type Errors = { nome?: string; descricao?: string };

const FIELDS_API: Record<string, keyof Errors> = {
    name: "nome",
    nome: "nome",
    description: "descricao",
    descricao: "descricao",
};

function withoutDescribedBy(control: SofaFieldControlProps) {
    const { describedBy, ...rest } = control;
    return { ...rest, "aria-describedby": describedBy };
}

export function ModalGroup({ org, opened: isOpen, onClose, group, onSave }: ModalGroupProps) {
    const { translate: t } = useI18n();
    const editing = group !== null;
    const [nome, setName] = useState("");
    const [descricao, setDescription] = useState("");
    const [situacao, setSituation] = useState("active");
    const [versao, setVersion] = useState(0);
    const [errors, setErrors] = useState<Errors>({});
    const [errorGeneral, setErrorGeneral] = useState<string | null>(null);
    const [conflict, setConflict] = useState<string | null>(null);
    const [sending, setSending] = useState(false);

    useEffect(() => {
        if (!isOpen) return;
        setName(group?.name ?? "");
        setDescription(group?.description ?? "");
        setSituation(group?.status ?? "active");
        setVersion(group?.version ?? 0);
        setErrors({});
        setErrorGeneral(null);
        setConflict(null);
        setSending(false);
    }, [isOpen, group]);

    function validar(): Errors {
        const freshItems: Errors = {};
        if (!nome.trim()) {
            freshItems.nome = t("knowledge.groupNameRequired");
        } else if (nome.trim().length > LIMIT_GROUP_NAME) {
            freshItems.nome = t("knowledge.groupNameTooLong", { limit: LIMIT_GROUP_NAME });
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
            code: failure.code,
        }));
    }

    async function save(evento?: FormEvent) {
        evento?.preventDefault();
        const freshItems = validar();
        setErrors(freshItems);
        setErrorGeneral(null);
        setConflict(null);
        if (Object.keys(freshItems).length > 0) return;

        setSending(true);
        try {
            if (!group) {
                const created = await createGroup(org, {
                    name: nome.trim(),
                    description: descricao,
                });
                onSave(created);
                onClose();
                return;
            }

            const changed = await updateGroup(org, group.id, versao, {
                name: nome.trim(),
                description: descricao,
                status: situacao,
            });
            onSave(changed);
            onClose();
        } catch (failure) {
            if (failure instanceof ApiError && failure.status === 412 && group) {
                let versionCurrent: number | undefined;
                try {
                    const current = await getGroup(org, group.id);
                    versionCurrent = current.version;
                } catch {
                }
                if (typeof versionCurrent === "number") setVersion(versionCurrent);
                setConflict(
                    typeof versionCurrent === "number"
                        ? t("knowledge.groupConflictWithVersion", { version: versionCurrent })
                        : t("knowledge.groupConflict"),
                );
                return;
            }
            if (failure instanceof ApiError) {
                showErrorApi(failure);
                return;
            }
            setErrorGeneral(t("knowledge.groupSaveUnexpected"));
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
            title={editing ? t("knowledge.groupEditTitle") : t("knowledge.groupCreateTitle")}
            description={
                editing
                    ? t("knowledge.groupEditDescription", { name: group?.name ?? "" })
                    : t("knowledge.groupCreateDescription")
            }
            closeLabel={t("knowledge.groupFormClose")}
            size="md"
            id="poph-formulario-grupo"
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
                            : t("knowledge.groupCreateSubmit")}
                    </SofaButton>
                </>
            }
        >
            <form
                id="poph-form-grupo"
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
                    label={t("knowledge.nameLabel")}
                    hint={t("knowledge.groupNameHint", { limit: LIMIT_GROUP_NAME })}
                    error={errors.nome}
                    required
                    htmlFor="poph-grupo-nome"
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
                    label={t("knowledge.descriptionLabel")}
                    hint={t("knowledge.groupDescriptionHint")}
                    error={errors.descricao}
                    htmlFor="poph-grupo-descricao"
                >
                    {(control) => (
                        <SofaTextarea
                            {...withoutDescribedBy(control)}
                            value={descricao}
                            rows={3}
                            onChange={(evento) => setDescription(evento.target.value)}
                        />
                    )}
                </SofaField>

                {editing && (
                    <SofaField
                        label={t("knowledge.statusLabel")}
                        hint={t("knowledge.groupStatusHint")}
                        htmlFor="poph-grupo-situacao"
                    >
                        {(control) => (
                            <SofaSelect
                                {...withoutDescribedBy(control)}
                                value={situacao}
                                onChange={(evento) => setSituation(evento.target.value)}
                            >
                                {SITUATIONS.map((option) => (
                                    <option key={option.value} value={option.value}>
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
