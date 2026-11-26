
import { useEffect, useState, type ReactNode } from "react";
import {
    SofaBanner,
    SofaButton,
    SofaCheckbox,
    SofaField,
    SofaInput,
    SofaModal,
    type SofaFieldControlProps,
} from "@sofacoding/sofa-ui";
import { ApiError } from "../api/client";
import { translate, useI18n } from "../i18n";

function withoutDescribedBy(control: SofaFieldControlProps) {
    const { describedBy, ...rest } = control;
    return { ...rest, "aria-describedby": describedBy };
}

export type DialogExclusionProps = {
    aberto: boolean;
    onClose: () => void;
    titulo: string;
    descricao: string;
    resource: string;
    labelResource: string;
    labelConfirm: string;
    consequencia?: ReactNode;
    permiteForcar?: boolean;
    forceOnlyAfterRefusal?: boolean;
    labelForce?: string;
    descriptionForce?: string;
    aoConfirmar: (forcar: boolean) => Promise<void>;
    aoExcluir?: () => void;
};

function messageError(error: unknown): { texto: string; codigo?: string } {
    if (error instanceof ApiError) {
        return { texto: error.message, codigo: error.codigo };
    }
    if (error instanceof Error) {
        return { texto: error.message };
    }
    return { texto: translate("orgs.deleteUnexpected") };
}

export function DialogExclusion({
    aberto: isOpen,
    onClose,
    titulo: title,
    descricao,
    resource,
    labelResource,
    labelConfirm,
    consequencia,
    permiteForcar: allowForce = false,
    forceOnlyAfterRefusal = false,
    labelForce,
    descriptionForce,
    aoConfirmar: onConfirm,
    aoExcluir: onDelete,
}: DialogExclusionProps) {
    const { translate: t } = useI18n();
    const [typed, setTyped] = useState("");
    const [forcar, setForce] = useState(false);
    const [sending, setSending] = useState(false);
    const [error, setError] = useState<{ texto: string; codigo?: string } | null>(null);
    const [refusedByLink, setRefusedByLink] = useState(false);

    useEffect(() => {
        if (isOpen) {
            setTyped("");
            setForce(false);
            setSending(false);
            setError(null);
            setRefusedByLink(false);
        }
    }, [isOpen]);

    const checks = typed === resource;

    async function confirm() {
        if (!checks || sending) return;
        setSending(true);
        setError(null);
        try {
            await onConfirm(forcar);
            onDelete?.();
            onClose();
        } catch (failure) {
            const traduzido = messageError(failure);
            setError(traduzido);
            if (allowForce && failure instanceof ApiError && failure.status === 409) {
                setRefusedByLink(true);
            }
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
            title={title}
            description={descricao}
            closeLabel={t("orgs.deleteClose")}
            size="sm"
            id="poph-dialogo-exclusao"
            footer={
                <>
                    <SofaButton variant="secondary" disabled={sending} onClick={onClose}>
                        {t("common.cancel")}
                    </SofaButton>
                    <SofaButton
                        variant="danger"
                        loading={sending}
                        disabled={!checks}
                        onClick={() => void confirm()}
                    >
                        {labelConfirm}
                    </SofaButton>
                </>
            }
        >
            <div className="poph-stack" style={{ gap: "var(--pop-spacing-lg)" }}>
                {consequencia}

                {error && (
                    <SofaBanner tone="danger" title={t("orgs.deleteRefusedTitle")}>
                        <span>{error.texto}</span>
                        {error.codigo && (
                            <div className="poph-mono-muted" style={{ marginTop: 4 }}>
                                {t("orgs.deleteCode", { code: error.codigo })}
                            </div>
                        )}
                        {refusedByLink && (
                            <div style={{ marginTop: 6 }}>{t("orgs.deleteForceHint")}</div>
                        )}
                    </SofaBanner>
                )}

                {allowForce && (!forceOnlyAfterRefusal || refusedByLink) && (
                    <SofaCheckbox
                        label={labelForce ?? t("orgs.deleteForceLabel")}
                        description={descriptionForce ?? t("orgs.deleteForceDescription")}
                        checked={forcar}
                        error={refusedByLink}
                        onChange={(evento) => setForce(evento.target.checked)}
                    />
                )}

                <div>
                    <SofaField
                        label={
                            <>
                                {t("orgs.deleteTypePrefix")} <code>{resource}</code>{" "}
                                {t("orgs.deleteTypeSuffix")}
                            </>
                        }
                        hint={t("orgs.deleteTypeHint", { label: labelResource })}
                        htmlFor="poph-confirma-exclusao"
                    >
                        {(control) => (
                            <SofaInput
                                {...withoutDescribedBy(control)}
                                value={typed}
                                autoComplete="off"
                                spellCheck={false}
                                onChange={(evento) => setTyped(evento.target.value)}
                            />
                        )}
                    </SofaField>
                </div>
            </div>
        </SofaModal>
    );
}
