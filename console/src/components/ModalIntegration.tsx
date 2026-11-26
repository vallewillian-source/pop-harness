
import { useCallback, useEffect, useState, type FormEvent } from "react";
import {
    SofaBanner,
    SofaButton,
    SofaField,
    SofaInput,
    SofaModal,
    SofaSelect,
    SofaSkeleton,
    SofaStatusIndicator,
    type SofaFieldControlProps,
    type SofaStatusTone,
} from "@sofacoding/sofa-ui";
import { ApiError } from "../api/client";
import {
    KIND_WHATSAPP,
    LIMIT_KIND,
    INTEGRATION_STATUSES,
    updateIntegration,
    createIntegration,
    getIntegration,
    getManifest,
    connectionLabel,
    noManifestForType,
    validateIntegration,
    type IntegrationData,
    type Integracao,
    type PluginManifest,
    type IntegrationStatus,
} from "../api/integrations";
import {
    FormSchema,
    fieldsSchema,
    validateValuesSchema,
    valuesIntegration,
    valuesConfiguration,
    valuesSecrets,
    type ValuesSchema,
} from "../components/FormSchema";
import { useI18n } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";

function withoutDescribedBy(control: SofaFieldControlProps) {
    const { describedBy, ...rest } = control;
    return { ...rest, "aria-describedby": describedBy };
}

const FIELDS_API: Record<string, string> = {
    kind: "kind",
    tipo: "kind",
    status: "situacao",
    situacao: "situacao",
};

export function fieldErrorApi(campo: string): string | null {
    const direct = FIELDS_API[campo];
    if (direct) return direct;
    for (const prefix of ["config_json.", "secrets."]) {
        if (campo.startsWith(prefix)) {
            const rest = campo.slice(prefix.length);
            return rest.split(".")[0] || null;
        }
    }
    return null;
}

export function stateInitialSecrets(integracao: Integracao | null): string[] | null {
    if (!integracao) return [];
    const filled = integracao.segredos_preenchidos;
    if (Array.isArray(filled)) return filled;
    if (filled === null) return null;
    return integracao.tem_segredo ? null : [];
}

function toneConnection(estado: string): SofaStatusTone {
    switch (estado) {
        case "connected":
            return "success";
        case "awaiting_scan":
            return "attention";
        case "disconnected":
            return "danger";
        default:
            return "muted";
    }
}

type StateManifest =
    | { situacao: "ocioso" }
    | { situacao: "carregando" }
    | { situacao: "pronto"; manifesto: PluginManifest }
    | { situacao: "sem_formulario" }
    | { situacao: "falha"; mensagem: string; messageKey?: TranslationKey; codigo?: string; tipKey?: TranslationKey };

const MANIFEST_TIP_KEYS: Record<string, TranslationKey> = {
    manifesto_indisponivel: "integrations.manifestTipUnavailable",
    network: "integrations.manifestTipNetwork",
    not_found: "integrations.manifestTipNotFound",
};

export function configurationSend(
    manifesto: PluginManifest,
    values: ValuesSchema,
    stored: Record<string, unknown> | null | undefined,
): Record<string, unknown> {
    const sent: Record<string, unknown> = { ...(stored ?? {}) };
    const editaveis = valuesConfiguration(manifesto.configuracao, values);
    for (const campo of fieldsSchema(manifesto.configuracao, null)) {
        if (campo.secret) continue;
        const desenhado =
            campo.property.enum !== undefined ||
            campo.property.type === "string" ||
            campo.property.type === "number" ||
            campo.property.type === "integer" ||
            campo.property.type === "boolean";
        if (!desenhado) continue;
        if (campo.nome in editaveis) {
            sent[campo.nome] = editaveis[campo.nome];
        } else {
            delete sent[campo.nome];
        }
    }
    return sent;
}

export type ModalIntegrationProps = {
    org: string;
    agente: string;
    opened: boolean;
    onClose: () => void;
    integracao: Integracao | null;
    onSave: (integracao: Integracao) => void;
};

export function ModalIntegration({
    org,
    agente,
    opened: isOpen,
    onClose,
    integracao,
    onSave,
}: ModalIntegrationProps) {
    const { translate: t } = useI18n();
    const editing = integracao !== null;
    const [kind, setKind] = useState("");
    const [situacao, setSituation] = useState<IntegrationStatus>("active");
    const [versao, setVersion] = useState(0);
    const [errors, setErrors] = useState<Record<string, string>>({});
    const [errorGeneral, setErrorGeneral] = useState<string | null>(null);
    const [conflict, setConflict] = useState<string | null>(null);
    const [sending, setSending] = useState(false);

    const [manifesto, setManifest] = useState<StateManifest>({ situacao: "ocioso" });
    const [values, setValues] = useState<ValuesSchema>({});
    const [kindValues, setKindValues] = useState<string | null>(null);
    const [secretsRemoved, setSecretsRemoved] = useState<string[]>([]);
    const [secretsFilled, setSecretsFilled] = useState<string[] | null>([]);

    useEffect(() => {
        if (!isOpen) return;
        setKind(integracao?.kind ?? "");
        setSituation((integracao?.status as IntegrationStatus | undefined) ?? "active");
        setVersion(integracao?.version ?? 0);
        setErrors({});
        setErrorGeneral(null);
        setConflict(null);
        setSending(false);
        setManifest({ situacao: "ocioso" });
        setValues({});
        setKindValues(null);
        setSecretsRemoved([]);
        setSecretsFilled(stateInitialSecrets(integracao));
    }, [isOpen, integracao]);

    useEffect(() => {
        if (!isOpen) return;
        const tipo = (editing ? (integracao?.kind ?? "") : kind).trim();
        if (tipo === "") {
            setManifest({ situacao: "ocioso" });
            return;
        }
        let cancelled = false;
        const temporizador = setTimeout(
            () => {
                setManifest({ situacao: "carregando" });
                getManifest(org, tipo)
                    .then((read) => {
                        if (cancelled) return;
                        if (
                            !read ||
                            typeof read.configuracao !== "object" ||
                            read.configuracao === null
                        ) {
                            setManifest({
                                situacao: "falha",
                                mensagem: "",
                                messageKey: "integrations.manifestInvalidError",
                                codigo: "manifesto_invalido",
                            });
                            return;
                        }
                        setManifest({ situacao: "pronto", manifesto: read });
                        setValues(
                            valuesIntegration(
                                read.configuracao,
                                read.segredos ?? null,
                                editing ? integracao?.configuracao : null,
                            ),
                        );
                        setKindValues(tipo);
                    })
                    .catch((failure: unknown) => {
                        if (cancelled) return;
                        if (noManifestForType(failure)) {
                            setManifest({ situacao: "sem_formulario" });
                            return;
                        }
                        if (failure instanceof ApiError) {
                            setManifest({
                                situacao: "falha",
                                mensagem: failure.message,
                                codigo: failure.code,
                                tipKey: MANIFEST_TIP_KEYS[failure.code],
                            });
                            return;
                        }
                        setManifest({
                            situacao: "falha",
                            mensagem: failure instanceof Error ? failure.message : "",
                            messageKey:
                                failure instanceof Error ? undefined : "integrations.manifestReadError",
                        });
                    });
            },
            editing ? 0 : 400,
        );
        return () => {
            cancelled = true;
            clearTimeout(temporizador);
        };
    }, [isOpen, kind, editing, integracao, org]);

    useEffect(() => {
        if (manifesto.situacao !== "pronto") return;
        const tipo = manifesto.manifesto.kind || (editing ? (integracao?.kind ?? "") : kind);
        if (kindValues === tipo) return;
        setValues(
            valuesIntegration(
                manifesto.manifesto.configuracao,
                manifesto.manifesto.segredos ?? null,
                editing ? integracao?.configuracao : null,
            ),
        );
        setKindValues(tipo);
    }, [manifesto, kindValues, editing, integracao, kind]);

    function showErrorApi(failure: ApiError) {
        const details = failure.details as Record<string, unknown> | null | undefined;
        const campo = typeof details?.["field"] === "string" ? details["field"] : undefined;
        const motivo =
            typeof details?.["reason"] === "string" ? details["reason"] : failure.message;
        const target = campo ? fieldErrorApi(campo) : null;
        if (target) {
            setErrors({ [target]: motivo });
            return;
        }
        setErrorGeneral(failure.message);
    }

    const toggleRemoval = useCallback((nome: string, remove: boolean) => {
        setSecretsRemoved((current) =>
            remove ? [...new Set([...current, nome])] : current.filter((item) => item !== nome),
        );
    }, []);

    async function save(evento?: FormEvent) {
        evento?.preventDefault();
        const freshItems = validateIntegration({ kind, situacao, editing: editing });

        if (manifesto.situacao === "carregando") {
            setErrorGeneral(t("integrations.manifestLoadingSaveError"));
            return;
        }
        const ready = manifesto.situacao === "pronto" ? manifesto.manifesto : null;
        if (ready) {
            Object.assign(
                freshItems,
                validateValuesSchema(
                    ready.configuracao,
                    ready.segredos ?? null,
                    values,
                    { filled: secretsFilled, removed: secretsRemoved },
                ),
            );
        }
        setErrors(freshItems);
        setErrorGeneral(null);
        setConflict(null);
        if (Object.keys(freshItems).length > 0) return;

        const dados: IntegrationData = { status: situacao };
        if (ready) {
            dados.configuracao = configurationSend(
                ready,
                values,
                editing ? integracao?.configuracao : null,
            );
            const segredos = valuesSecrets(
                ready.segredos ?? null,
                values,
                secretsRemoved,
            );
            if (Object.keys(segredos).length > 0) dados.segredos = segredos;
        }

        setSending(true);
        try {
            if (!integracao) {
                const created = await createIntegration(org, agente, kind.trim(), dados);
                onSave(created);
                onClose();
                return;
            }
            const changed = await updateIntegration(
                org,
                agente,
                integracao.id,
                versao,
                dados,
            );
            onSave(changed);
            onClose();
        } catch (failure) {
            if (failure instanceof ApiError && failure.status === 412 && integracao) {
                let versionCurrent: number | undefined;
                try {
                    const current = await getIntegration(org, agente, integracao.id);
                    versionCurrent = current.version;
                    setSecretsFilled(stateInitialSecrets(current));
                } catch {
                }
                if (typeof versionCurrent === "number") setVersion(versionCurrent);
                setConflict(
                    t("integrations.modalConflict", {
                        versionSuffix:
                            typeof versionCurrent === "number"
                                ? t("integrations.modalConflictVersionSuffix", {
                                      version: versionCurrent,
                                  })
                                : "",
                    }),
                );
                return;
            }
            if (failure instanceof ApiError) {
                showErrorApi(failure);
                return;
            }
            setErrorGeneral(t("integrations.modalUnexpectedSaveError"));
        } finally {
            setSending(false);
        }
    }

    const title = editing ? t("integrations.modalTitleEdit") : t("integrations.modalTitleNew");
    const blockedByManifest = manifesto.situacao === "falha";
    const withoutConfiguration = manifesto.situacao === "sem_formulario";

    return (
        <SofaModal
            open={isOpen}
            onOpenChange={(fresh) => {
                if (!fresh && !sending) onClose();
            }}
            title={title}
            description={
                editing
                    ? t("integrations.modalDescriptionEdit")
                    : t("integrations.modalDescriptionNew")
            }
            closeLabel={t("integrations.modalCloseLabel")}
            id="poph-formulario-integracao"
            footer={
                <>
                    <SofaButton variant="secondary" disabled={sending} onClick={onClose}>
                        {t("integrations.modalCancel")}
                    </SofaButton>
                    <SofaButton
                        variant="primary"
                        type="button"
                        onClick={() => void save()}
                        loading={sending}
                        disabled={blockedByManifest}
                    >
                        {editing ? t("integrations.modalSaveChanges") : t("integrations.modalCreate")}
                    </SofaButton>
                </>
            }
        >
            <form
                id="poph-form-integracao"
                className="poph-stack"
                style={{ gap: "var(--pop-spacing-lg)" }}
                onSubmit={(evento) => void save(evento)}
                noValidate
            >
                {conflict && (
                    <SofaBanner tone="warn" title={t("integrations.bannerOutdatedTitle")}>
                        {conflict}
                    </SofaBanner>
                )}
                {errorGeneral && (
                    <SofaBanner tone="danger" title={t("integrations.bannerSaveErrorTitle")}>
                        {errorGeneral}
                    </SofaBanner>
                )}

                <SofaField
                    label={t("integrations.fieldKindLabel")}
                    hint={t("integrations.fieldKindHint", {
                        whatsapp: KIND_WHATSAPP,
                        max: LIMIT_KIND,
                    })}
                    error={errors["kind"]}
                    required
                    htmlFor="poph-integracao-kind"
                >
                    {(control) => (
                        <SofaInput
                            {...withoutDescribedBy(control)}
                            value={kind}
                            autoComplete="off"
                            disabled={editing}
                            onChange={(evento) => setKind(evento.target.value)}
                        />
                    )}
                </SofaField>

                {editing && integracao && (
                    <div className="poph-stack" style={{ gap: 4 }}>
                        <span className="poph-mono-muted">
                            {t("integrations.connectionStateLabel")}
                        </span>
                        <div>
                            <SofaStatusIndicator tone={toneConnection(integracao.connection_state)}>
                                {connectionLabel(integracao.connection_state)}
                            </SofaStatusIndicator>
                        </div>
                    </div>
                )}

                {manifesto.situacao === "carregando" && (
                    <div className="poph-stack" style={{ gap: 4 }} role="status">
                        <span className="poph-mono-muted">
                            {t("integrations.manifestLoadingText")}
                        </span>
                        <SofaSkeleton lines={3} />
                    </div>
                )}

                {manifesto.situacao === "pronto" && (
                    <div className="poph-stack" style={{ gap: "var(--pop-spacing-sm, 8px)" }}>
                        <span className="poph-mono-muted">
                            {t("integrations.pluginConfigHeader", {
                                name: manifesto.manifesto.nome,
                                version: manifesto.manifesto.versao,
                            })}
                        </span>
                        <FormSchema
                            configuracao={manifesto.manifesto.configuracao}
                            segredos={manifesto.manifesto.segredos ?? null}
                            values={values}
                            onChange={(nome, value) =>
                                setValues((current) => ({ ...current, [nome]: value }))
                            }
                            errors={errors}
                            idPrefix="poph-integracao"
                            secretsState={{
                                filled: secretsFilled,
                                removed: secretsRemoved,
                            }}
                            onToggleRemoval={toggleRemoval}
                        />
                    </div>
                )}

                {withoutConfiguration && (
                    <SofaBanner tone="info" title={t("integrations.noManifestTitle")}>
                        {t("integrations.noManifestDescription", { kind: kind.trim() })}
                    </SofaBanner>
                )}

                {manifesto.situacao === "falha" && (
                    <SofaBanner tone="danger" title={t("integrations.manifestErrorTitle")}>
                        {manifesto.messageKey ? t(manifesto.messageKey) : manifesto.mensagem}
                        {manifesto.tipKey ? ` ${t(manifesto.tipKey)}` : ""}
                        {manifesto.codigo ? (
                            <span className="poph-mono-muted">
                                {t("integrations.manifestErrorCode", { code: manifesto.codigo })}
                            </span>
                        ) : null}
                    </SofaBanner>
                )}

                <SofaField
                    label={t("integrations.fieldStatusLabel")}
                    hint={t("integrations.fieldStatusHint")}
                    error={errors["situacao"]}
                    required
                    htmlFor="poph-integracao-situacao"
                >
                    {(control) => (
                        <SofaSelect
                            {...withoutDescribedBy(control)}
                            value={situacao}
                            onChange={(evento) =>
                                setSituation(evento.target.value as IntegrationStatus)
                            }
                        >
                            {INTEGRATION_STATUSES.map((option) => (
                                <option key={option.value} value={option.value}>
                                    {option.label}
                                </option>
                            ))}
                        </SofaSelect>
                    )}
                </SofaField>

                {secretsFilled === null && integracao?.tem_segredo !== false && (
                    <p className="poph-mono-muted" style={{ margin: 0 }}>
                        {t("integrations.unreadableSecretsNote")}
                    </p>
                )}

                {manifesto.situacao === "pronto" && (
                    <p className="poph-mono-muted" style={{ margin: 0 }}>
                        {t("integrations.secretsBlankNote")}
                    </p>
                )}
            </form>
        </SofaModal>
    );
}
