
import { useEffect, useState, type FormEvent } from "react";
import {
    SofaBanner,
    SofaButton,
    SofaField,
    SofaInput,
    SofaModal,
    SofaSelect,
    type SofaFieldControlProps,
} from "@sofacoding/sofa-ui";
import { ApiError, type Org } from "../api/client";
import {
    updateOrg,
    withRetention,
    versionConflict,
    createOrg,
    getOrg,
    retentionDays,
} from "../api/orgs";
import { useI18n } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";

const SLUG = /^[a-z0-9-]{3,64}$/;

const SITUATIONS: { value: string; rotulo: TranslationKey }[] = [
    { value: "active", rotulo: "orgs.statusActive" },
    { value: "inactive", rotulo: "orgs.statusInactive" },
    { value: "archived", rotulo: "orgs.statusArchived" },
];

export type ModalOrgProps = {
    opened: boolean;
    onClose: () => void;
    org: Org | null;
    onSave: (org: Org) => void;
};

function withoutDescribedBy(control: SofaFieldControlProps) {
    const { describedBy, ...rest } = control;
    return { ...rest, "aria-describedby": describedBy };
}

const FIELDS_API: Record<string, keyof Errors> = {
    name: "nome",
    slug: "slug",
    config: "retencao",
    retention_days: "retencao",
};

type Errors = {
    nome?: string;
    slug?: string;
    retencao?: string;
};

export function ModalOrg({ opened: isOpen, onClose, org, onSave }: ModalOrgProps) {
    const { translate: t } = useI18n();
    const editing = org !== null;
    const [nome, setName] = useState("");
    const [slug, setSlug] = useState("");
    const [situacao, setSituation] = useState("active");
    const [retencao, setRetention] = useState("");
    const [versao, setVersion] = useState(0);
    const [errors, setErrors] = useState<Errors>({});
    const [errorGeneral, setErrorGeneral] = useState<string | null>(null);
    const [conflict, setConflict] = useState<string | null>(null);
    const [sending, setSending] = useState(false);

    useEffect(() => {
        if (!isOpen) return;
        setName(org?.name ?? "");
        setSlug(org?.slug ?? "");
        setSituation(org?.status ?? "active");
        const dias = org ? retentionDays(org) : undefined;
        setRetention(dias === undefined ? "" : String(dias));
        setVersion(org?.version ?? 0);
        setErrors({});
        setErrorGeneral(null);
        setConflict(null);
        setSending(false);
    }, [isOpen, org]);

    function validar(): Errors {
        const freshItems: Errors = {};
        if (!nome.trim()) {
            freshItems.nome = t("orgs.formNameRequired");
        } else if (nome.trim().length > 120) {
            freshItems.nome = t("orgs.formNameTooLong");
        }
        if (!editing && slug.trim() && !SLUG.test(slug.trim())) {
            freshItems.slug = t("orgs.formSlugInvalid");
        }
        if (retencao.trim()) {
            const dias = Number(retencao);
            if (!Number.isInteger(dias) || dias < 0) {
                freshItems.retencao = t("orgs.formRetentionInvalid");
            }
        }
        return freshItems;
    }

    function showErrorApi(failure: ApiError, criar: boolean) {
        const details = failure.details as Record<string, unknown> | null | undefined;
        const campo = typeof details?.["field"] === "string" ? details["field"] : undefined;
        if (failure.status === 409 || failure.code === "conflict") {
            if (criar) {
                setErrors({ slug: t("orgs.formSlugTaken") });
            } else {
                setErrorGeneral(failure.message);
            }
            return;
        }
        const target = campo ? FIELDS_API[campo] : undefined;
        if (target) {
            setErrors({ [target]: failure.message });
            return;
        }
        setErrorGeneral(failure.message);
    }

    async function save(evento?: FormEvent) {
        evento?.preventDefault();
        const freshItems = validar();
        setErrors(freshItems);
        setErrorGeneral(null);
        setConflict(null);
        if (Object.keys(freshItems).length > 0) return;

        const dias = retencao.trim() ? Number(retencao) : null;
        setSending(true);
        try {
            if (!org) {
                const config: Record<string, unknown> = {};
                if (dias !== null) config["retention_days"] = dias;
                const created = await createOrg({
                    name: nome.trim(),
                    ...(slug.trim() ? { slug: slug.trim() } : {}),
                    config,
                });
                onSave(created);
                onClose();
                return;
            }

            const changed = await updateOrg(org.id, versao, {
                name: nome.trim(),
                status: situacao,
                config: withRetention(org.config as Record<string, unknown>, dias),
            });
            onSave(changed);
            onClose();
        } catch (failure) {
            const disputa = versionConflict(failure);
            if (disputa && org) {
                let versionCurrent = disputa.current;
                try {
                    const current = await getOrg(org.id);
                    versionCurrent = current.version;
                } catch {
                }
                if (typeof versionCurrent === "number") setVersion(versionCurrent);
                setConflict(
                    typeof versionCurrent === "number"
                        ? t("orgs.formConflictWithVersion", { version: versionCurrent })
                        : t("orgs.formConflictPlain"),
                );
                return;
            }
            if (failure instanceof ApiError) {
                showErrorApi(failure, !editing);
                return;
            }
            setErrorGeneral(t("orgs.formSaveUnexpected"));
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
            title={editing ? t("orgs.modalEditTitle") : t("orgs.modalCreateTitle")}
            description={
                editing
                    ? t("orgs.modalEditDescription", { name: org?.name ?? "" })
                    : t("orgs.tenantDescription")
            }
            closeLabel={t("orgs.modalClose")}
            size="md"
            id="poph-formulario-org"
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
                        {editing ? t("orgs.formSaveChanges") : t("orgs.formCreate")}
                    </SofaButton>
                </>
            }
        >
            <form
                id="poph-form-org"
                className="poph-stack"
                style={{ gap: "var(--pop-spacing-lg)" }}
                onSubmit={(evento) => void save(evento)}
                noValidate
            >
                {conflict && (
                    <SofaBanner tone="warn" title={t("orgs.formConflictTitle")}>
                        {conflict}
                    </SofaBanner>
                )}
                {errorGeneral && (
                    <SofaBanner tone="danger" title={t("orgs.formErrorTitle")}>
                        {errorGeneral}
                    </SofaBanner>
                )}

                <SofaField
                    label={t("orgs.formNameLabel")}
                    hint={t("orgs.formNameHint")}
                    error={errors.nome}
                    required
                    htmlFor="poph-org-nome"
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
                    label={t("orgs.formSlugLabel")}
                    hint={editing ? t("orgs.formSlugHintLocked") : t("orgs.formSlugHintNew")}
                    error={errors.slug}
                    htmlFor="poph-org-slug"
                >
                    {(control) => (
                        <SofaInput
                            {...withoutDescribedBy(control)}
                            value={slug}
                            disabled={editing}
                            autoComplete="off"
                            spellCheck={false}
                            className="poph-mono-muted"
                            onChange={(evento) => setSlug(evento.target.value)}
                        />
                    )}
                </SofaField>

                {editing && (
                    <SofaField
                        label={t("orgs.formStatusLabel")}
                        hint={t("orgs.formStatusHint")}
                        htmlFor="poph-org-situacao"
                    >
                        {(control) => (
                            <SofaSelect
                                {...withoutDescribedBy(control)}
                                value={situacao}
                                onChange={(evento) => setSituation(evento.target.value)}
                            >
                                {SITUATIONS.map((option) => (
                                    <option key={option.value} value={option.value}>
                                        {t(option.rotulo)}
                                    </option>
                                ))}
                            </SofaSelect>
                        )}
                    </SofaField>
                )}

                <SofaField
                    label={t("orgs.formRetentionLabel")}
                    hint={t("orgs.formRetentionHint")}
                    error={errors.retencao}
                    htmlFor="poph-org-retencao"
                >
                    {(control) => (
                        <SofaInput
                            {...withoutDescribedBy(control)}
                            type="number"
                            min={0}
                            step={1}
                            inputMode="numeric"
                            value={retencao}
                            onChange={(evento) => setRetention(evento.target.value)}
                        />
                    )}
                </SofaField>
            </form>
        </SofaModal>
    );
}
