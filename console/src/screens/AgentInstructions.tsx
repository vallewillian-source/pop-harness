
import { useCallback, useEffect, useMemo, useState } from "react";
import { useParams } from "react-router-dom";
import {
    IconArrowLeft,
    IconRefresh,
    SofaBanner,
    SofaButton,
    SofaErrorState,
    SofaSkeleton,
} from "@sofacoding/sofa-ui";
import { ApiError } from "../api/client";
import { getAgent, type Agente } from "../api/agents";
import { getOrg } from "../api/orgs";
import {
    defaultStructured,
    readInstructions,
    sameInstructions,
    saveInstructions,
    type Structured,
} from "../api/instructions";
import {
    TabsAgent,
    HeaderAgent,
    SectionAgent,
} from "../components/TabsAgent";
import {
    FormInstructions,
    type ErrorsForm,
} from "../components/FormInstructions";
import { useGuardChanges } from "../components/GuardChanges";
import { translate, useI18n } from "../i18n";
import "../styles/instructions.css";

export function validateInstructions(value: Structured): ErrorsForm {
    const errors: ErrorsForm = {};
    if (!value.identity.name.trim()) {
        errors.nome = translate("instructions.validationNameRequired");
    }
    if (value.principles.some((principle) => !principle.rule.trim())) {
        errors.principles = translate("instructions.validationPrinciplesRequired");
    }
    return errors;
}

function fieldError(failure: ApiError): "ferramentas" | "nome" | null {
    const details = failure.details as Record<string, unknown> | null | undefined;
    const campo = typeof details?.["field"] === "string" ? details["field"] : "";
    if (campo.startsWith("tools")) return "ferramentas";
    if (campo.startsWith("identity")) return "nome";
    if (campo === "structured") {
        const motivo = typeof details?.["reason"] === "string" ? details["reason"] : "";
        if (motivo.includes("ferramenta")) return "ferramentas";
    }
    return null;
}

function reasonError(failure: ApiError): string {
    const details = failure.details as Record<string, unknown> | null | undefined;
    return typeof details?.["reason"] === "string" ? details["reason"] : failure.message;
}

export function AgentInstructions() {
    const { orgId = "", agentId = "" } = useParams();
    const guard = useGuardChanges();
    const { translate: t } = useI18n();

    const [agente, setAgent] = useState<Agente | null>(null);
    const [nameOrg, setOrgName] = useState<string | null>(null);
    const [loading, setLoading] = useState(true);
    const [errorLoad, setLoadError] = useState<string | null>(null);

    const [draft, setDraft] = useState<Structured | null>(null);
    const [saved, setSaved] = useState<Structured | null>(null);
    const [versao, setVersion] = useState<number | null>(null);
    const [markdown, setMarkdown] = useState("");
    const [versionPreview, setPreviewVersion] = useState<number | null>(null);

    const [saving, setSaving] = useState(false);
    const [reloadingPreview, setReloadingPreview] = useState(false);
    const [conflict, setConflict] = useState<string | null>(null);
    const [generalError, setGeneralError] = useState<string | null>(null);
    const [errors, setErrors] = useState<ErrorsForm>({});
    const [noticePreview, setPreviewNotice] = useState<string | null>(null);

    const dirty = useMemo(
        () => (draft && saved ? !sameInstructions(draft, saved) : false),
        [draft, saved],
    );

    const load = useCallback(async () => {
        setLoading(true);
        setLoadError(null);
        setConflict(null);
        setGeneralError(null);
        setErrors({});
        setPreviewNotice(null);
        try {
            const [dataAgent, instructions] = await Promise.all([
                getAgent(orgId, agentId),
                readInstructions(orgId, agentId),
            ]);
            setAgent(dataAgent);
            const base = instructions ? instructions.structured : defaultStructured();
            if (!instructions && !base.identity.name) {
                base.identity.name = dataAgent.name;
            }
            setDraft(base);
            setSaved(base);
            setVersion(instructions?.version ?? null);
            setPreviewVersion(instructions?.version ?? null);
            setMarkdown(instructions?.markdown ?? "");
        } catch (failure) {
            const mensagem =
                failure instanceof Error ? failure.message : t("instructions.loadFailed");
            setLoadError(mensagem);
        } finally {
            setLoading(false);
        }
    }, [orgId, agentId, t]);

    useEffect(() => {
        void load();
        getOrg(orgId)
            .then((org) => setOrgName(org.name))
            .catch(() => setOrgName(null));
    }, [load, orgId]);

    const save = useCallback(async (): Promise<boolean> => {
        if (!draft) return false;
        const freshItems = validateInstructions(draft);
        setErrors(freshItems);
        setConflict(null);
        setGeneralError(null);
        if (Object.keys(freshItems).length > 0) return false;

        setSaving(true);
        try {
            const response = await saveInstructions(orgId, agentId, draft, versao);
            setDraft(response.structured);
            setSaved(response.structured);
            setVersion(response.version);
            setPreviewVersion(response.version);
            setMarkdown(response.markdown);
            setPreviewNotice(null);
            return true;
        } catch (failure) {
            if (failure instanceof ApiError && failure.status === 412) {
                let current: Awaited<ReturnType<typeof readInstructions>> = null;
                try {
                    current = await readInstructions(orgId, agentId);
                } catch {
                }
                if (current) {
                    setVersion(current.version);
                    setPreviewVersion(current.version);
                    setMarkdown(current.markdown);
                }
                setConflict(
                    current
                        ? t("instructions.conflictMessageVersion", { version: current.version })
                        : t("instructions.conflictMessage"),
                );
                return false;
            }
            if (failure instanceof ApiError) {
                const campo = fieldError(failure);
                const motivo = reasonError(failure);
                if (campo === "ferramentas") {
                    setErrors({ ferramentas: motivo });
                } else if (campo === "nome") {
                    setErrors({ nome: motivo });
                } else {
                    setGeneralError(
                        t("instructions.errorWithCode", {
                            message: failure.message,
                            code: failure.code,
                        }),
                    );
                }
                return false;
            }
            setGeneralError(t("instructions.unexpectedSaveError"));
            return false;
        } finally {
            setSaving(false);
        }
    }, [draft, versao, orgId, agentId, t]);

    useEffect(() => {
        if (!draft) return;
        guard.register({ dirty, save });
    }, [guard, dirty, save, draft]);

    useEffect(() => {
        return () => guard.register(null);
    }, [guard]);

    useEffect(() => {
        const onKeyPress = (evento: KeyboardEvent) => {
            if (!(evento.ctrlKey || evento.metaKey) || evento.key.toLowerCase() !== "s") return;
            if (!dirty && versao !== null) return;
            evento.preventDefault();
            void save();
        };
        window.addEventListener("keydown", onKeyPress);
        return () => window.removeEventListener("keydown", onKeyPress);
    }, [dirty, versao, save]);

    async function updatePreview() {
        setReloadingPreview(true);
        setPreviewNotice(null);
        try {
            const current = await readInstructions(orgId, agentId);
            if (!current) {
                setMarkdown("");
                setPreviewVersion(null);
                return;
            }
            setMarkdown(current.markdown);
            setPreviewVersion(current.version);
            if (!dirty) {
                setDraft(current.structured);
                setSaved(current.structured);
                setVersion(current.version);
            } else if (current.version !== versao) {
                setPreviewNotice(t("instructions.previewNewerVersion", { version: current.version }));
            }
        } catch (failure) {
            setPreviewNotice(
                failure instanceof Error
                    ? t("instructions.previewRefetchFailed", { message: failure.message })
                    : t("instructions.previewRefetchFailedPlain"),
            );
        } finally {
            setReloadingPreview(false);
        }
    }

    const previewStale = dirty && versionPreview !== null;

    if (loading && !agente) {
        return (
            <div className="poph-page" aria-busy="true">
                <SofaSkeleton width={260} height={26} />
                <div className="poph-instructions__skeleton-line">
                    <SofaSkeleton width={360} height={12} />
                </div>
                <SofaSkeleton lines={5} />
            </div>
        );
    }

    if (errorLoad && !agente) {
        return (
            <div className="poph-page">
                <SofaErrorState
                    title={t("instructions.loadErrorTitle")}
                    description={t("instructions.loadErrorDescription")}
                    details={errorLoad}
                    retry={{ label: t("instructions.retry"), onClick: () => void load() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            icon={<IconArrowLeft aria-hidden="true" />}
                            onClick={() => guard.tryNavigate(`/orgs/${orgId}/agents`)}
                        >
                            {t("instructions.backToAgents")}
                        </SofaButton>
                    }
                />
            </div>
        );
    }

    if (!agente || !draft) return null;

    return (
        <div className="poph-page">
            <HeaderAgent
                org={orgId}
                nome={agente.name}
                slug={agente.slug}
                situacao={agente.status}
                organization={nameOrg}
                descricao={
                    <>
                        {versao === null
                            ? t("instructions.noInstructionsSaved")
                            : t("instructions.versionLabel", { version: versao })}
                        {dirty && (
                            <span className="poph-dirty">{t("instructions.unsavedSuffix")}</span>
                        )}
                    </>
                }
                actions={
                    <>
                        <SofaButton
                            variant="secondary"
                            disabled={!dirty || saving}
                            onClick={() => {
                                setDraft(saved);
                                setErrors({});
                                setConflict(null);
                                setGeneralError(null);
                            }}
                        >
                            {t("instructions.undo")}
                        </SofaButton>
                        <SofaButton
                            variant="primary"
                            disabled={!dirty && versao !== null}
                            loading={saving}
                            onClick={() => void save()}
                        >
                            {versao === null
                                ? t("instructions.saveNew")
                                : t("instructions.saveVersion", { version: versao })}
                        </SofaButton>
                    </>
                }
            />

            <TabsAgent org={orgId} agente={agentId} active="instructions" nome={agente.name} model={agente.model ?? null} />

            {(conflict || generalError) && (
                <div className="poph-stack">
                    {conflict && (
                        <SofaBanner tone="warn" title={t("instructions.conflictTitle")}>
                            {conflict}
                        </SofaBanner>
                    )}
                    {generalError && (
                        <SofaBanner tone="danger" title={t("instructions.saveErrorTitle")}>
                            {generalError}
                        </SofaBanner>
                    )}
                </div>
            )}

            <div className="poph-instructions">
                <form
                    id="poph-form-instrucoes"
                    noValidate
                    onSubmit={(evento) => {
                        evento.preventDefault();
                        void save();
                    }}
                >
                    <FormInstructions
                        value={draft}
                        onChange={setDraft}
                        errors={errors}
                        disabled={saving}
                    />
                </form>

                <aside className="poph-instructions__preview">
                    <SectionAgent
                        title={t("instructions.previewSection")}
                        descricao={t("instructions.previewDescription")}
                        actions={
                            <SofaButton
                                variant="subtle"
                                icon={<IconRefresh aria-hidden="true" />}
                                loading={reloadingPreview}
                                onClick={() => void updatePreview()}
                            >
                                {t("instructions.refreshPreview")}
                            </SofaButton>
                        }
                    >
                        <div className="poph-stack">
                            {previewStale && (
                                <SofaBanner
                                    tone="info"
                                    title={t("instructions.previewStaleTitle")}
                                >
                                    {versionPreview !== null
                                        ? t("instructions.previewStaleBodyVersion", {
                                              version: versionPreview,
                                          })
                                        : t("instructions.previewStaleBody")}
                                </SofaBanner>
                            )}
                            {noticePreview && (
                                <SofaBanner tone="warn" title={t("instructions.previewTitle")}>
                                    {noticePreview}
                                </SofaBanner>
                            )}
                            {markdown ? (
                                <pre
                                    className="poph-preview"
                                    aria-label={t("instructions.previewAriaLabel")}
                                >
                                    {markdown}
                                </pre>
                            ) : (
                                <p className="poph-mono-muted">{t("instructions.previewEmpty")}</p>
                            )}
                        </div>
                    </SectionAgent>
                </aside>
            </div>
        </div>
    );
}
