
import { useEffect, useState, type FormEvent } from "react";
import {
    SofaBanner,
    SofaButton,
    SofaField,
    SofaInput,
    SofaModal,
    SofaTextarea,
    type SofaFieldControlProps,
} from "@sofacoding/sofa-ui";
import { ApiError } from "../api/client";
import {
    LIMIT_FILE_BYTES,
    LIMIT_FILE_DESCRIPTION,
    LIMIT_FILE_NAME,
    sendFile,
    sizeInMb,
    type LibraryFile,
} from "../api/files";
import { translate, useI18n } from "../i18n";

const TIPOS_ACEITOS =
    ".pdf,.docx,.xlsx,.xls,.pptx,image/jpeg,image/png,image/webp,image/gif";

export type ModalFileProps = {
    org: string;
    agente: string;
    opened: boolean;
    onClose: () => void;
    onSend: (file: LibraryFile) => void;
};

type Errors = { file?: string; nome?: string; descricao?: string };

function withoutDescribedBy(control: SofaFieldControlProps) {
    const { describedBy, ...rest } = control;
    return { ...rest, "aria-describedby": describedBy };
}

function excessReadable(bytes: number): string {
    const excesso = bytes - LIMIT_FILE_BYTES;
    if (excesso < 1024 * 1024) {
        return `${Math.max(1, Math.ceil(excesso / 1024))} KB`;
    }
    return sizeInMb(excesso);
}

export function errorSize(bytes: number): string | null {
    if (bytes === 0) return translate("files.fileEmpty");
    if (bytes > LIMIT_FILE_BYTES) {
        return translate("files.fileTooLarge", { excess: excessReadable(bytes) });
    }
    return null;
}

export function ModalFile({ org, agente, opened: isOpen, onClose, onSend }: ModalFileProps) {
    const { translate: t } = useI18n();
    const [chosen, setChosen] = useState<File | null>(null);
    const [nome, setName] = useState("");
    const [descricao, setDescription] = useState("");
    const [errors, setErrors] = useState<Errors>({});
    const [errorGeneral, setErrorGeneral] = useState<string | null>(null);
    const [sending, setSending] = useState(false);

    useEffect(() => {
        if (!isOpen) return;
        setChosen(null);
        setName("");
        setDescription("");
        setErrors({});
        setErrorGeneral(null);
        setSending(false);
    }, [isOpen]);

    function onChoose(file: File | null) {
        setChosen(file);
        setErrorGeneral(null);
        if (!file) {
            setErrors((current) => ({ ...current, file: undefined }));
            return;
        }
        setErrors((current) => ({ ...current, file: errorSize(file.size) ?? undefined }));
    }

    function validar(): Errors {
        const freshItems: Errors = {};
        if (!chosen) {
            freshItems.file = t("files.chooseFile");
        } else {
            const size = errorSize(chosen.size);
            if (size) freshItems.file = size;
        }
        if (nome.trim().length > LIMIT_FILE_NAME) {
            freshItems.nome = t("files.nameTooLong", { limit: LIMIT_FILE_NAME });
        }
        if (descricao.trim().length > LIMIT_FILE_DESCRIPTION) {
            freshItems.descricao = t("files.descriptionTooLong", {
                limit: LIMIT_FILE_DESCRIPTION,
            });
        }
        return freshItems;
    }

    async function send(evento?: FormEvent) {
        evento?.preventDefault();
        const freshItems = validar();
        setErrors(freshItems);
        setErrorGeneral(null);
        if (Object.keys(freshItems).length > 0 || !chosen) return;

        setSending(true);
        try {
            const created = await sendFile(org, agente, chosen, nome, descricao);
            onSend(created);
            onClose();
        } catch (failure) {
            if (failure instanceof ApiError) {
                const details = failure.details as Record<string, unknown> | null | undefined;
                const motivo =
                    typeof details?.["reason"] === "string" ? details["reason"] : null;
                if (motivo && failure.status === 400) {
                    setErrors({ file: motivo });
                    return;
                }
                setErrorGeneral(t("files.apiErrorWithCode", {
                    message: failure.message,
                    code: failure.code,
                }));
                return;
            }
            setErrorGeneral(t("files.uploadUnexpected"));
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
            title={t("files.uploadTitle")}
            description={t("files.uploadDescription")}
            closeLabel={t("files.uploadClose")}
            size="md"
            id="poph-formulario-arquivo"
            footer={
                <>
                    <SofaButton variant="secondary" disabled={sending} onClick={onClose}>
                        {t("common.cancel")}
                    </SofaButton>
                    <SofaButton
                        variant="primary"
                        type="button"
                        onClick={() => void send()}
                        loading={sending}
                    >
                        {t("files.upload")}
                    </SofaButton>
                </>
            }
        >
            <form
                id="poph-form-arquivo"
                className="poph-stack"
                style={{ gap: "var(--pop-spacing-lg)" }}
                onSubmit={(evento) => void send(evento)}
                noValidate
            >
                {errorGeneral && (
                    <SofaBanner tone="danger" title={t("files.uploadFailedTitle")}>
                        {errorGeneral}
                    </SofaBanner>
                )}

                <SofaField
                    label={t("files.fileLabel")}
                    hint={t("files.fileHint")}
                    error={errors.file}
                    required
                    htmlFor="poph-arquivo-escolha"
                >
                    {(control) => (
                        <SofaInput
                            {...withoutDescribedBy(control)}
                            type="file"
                            accept={TIPOS_ACEITOS}
                            invalid={Boolean(errors.file)}
                            onChange={(evento) =>
                                onChoose(evento.target.files?.[0] ?? null)
                            }
                        />
                    )}
                </SofaField>
                {chosen && !errors.file && (
                    <p className="poph-mono-muted" style={{ margin: 0 }}>
                        {t("files.chosenFile", {
                            name: chosen.name,
                            size: sizeInMb(chosen.size),
                        })}
                    </p>
                )}

                <SofaField
                    label={t("files.nameLabel")}
                    hint={t("files.nameHint", { limit: LIMIT_FILE_NAME })}
                    error={errors.nome}
                    htmlFor="poph-arquivo-nome"
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
                    label={t("files.descriptionLabel")}
                    hint={t("files.descriptionHint", { limit: LIMIT_FILE_DESCRIPTION })}
                    error={errors.descricao}
                    htmlFor="poph-arquivo-descricao"
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
            </form>
        </SofaModal>
    );
}
