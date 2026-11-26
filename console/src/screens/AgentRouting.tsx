
import { useCallback, useEffect, useMemo, useState } from "react";
import { useParams } from "react-router-dom";
import {
    IconArrowLeft,
    IconPlus,
    IconRefresh,
    IconTrash,
    SofaBanner,
    SofaButton,
    SofaEmptyState,
    SofaErrorState,
    SofaField,
    type SofaFieldControlProps,
    SofaIconButton,
    SofaInput,
    SofaSelect,
    SofaSkeleton,
    SofaSwitch,
} from "@sofacoding/sofa-ui";
import { ApiError } from "../api/client";
import { getAgent, type Agente } from "../api/agents";
import { listStructure, type Departamento } from "../api/structure";
import { getOrg } from "../api/orgs";
import {
    LIMIT_REASON,
    LIMIT_REASONS,
    jevWithReasons,
    cleanReasons,
    agentRouting,
    saveRouting,
    validateRouting,
    type RoutingChange,
    type Routing,
} from "../api/routing";
import {
    TabsAgent,
    HeaderAgent,
    SectionAgent,
} from "../components/TabsAgent";
import { useGuardChanges } from "../components/GuardChanges";
import { useI18n } from "../i18n";
import "../styles/routing.css";

function sameRouting(a: Routing, b: Routing): boolean {
    return (
        a.defaultDepartment === b.defaultDepartment &&
        a.humanHandoff === b.humanHandoff &&
        a.handoffDepartment === b.handoffDepartment &&
        JSON.stringify(cleanReasons(a.reasons)) === JSON.stringify(cleanReasons(b.reasons))
    );
}

export function routingChange(
    saved: Routing,
    draft: Routing,
    currentJev: Record<string, unknown> | undefined,
): RoutingChange {
    const change: RoutingChange = {};
    if (draft.defaultDepartment && draft.defaultDepartment !== saved.defaultDepartment) {
        change.department = draft.defaultDepartment;
    }
    if (draft.humanHandoff !== saved.humanHandoff) {
        change.handoff = draft.humanHandoff;
    }
    if (draft.handoffDepartment && draft.handoffDepartment !== saved.handoffDepartment) {
        change.handoff_department = draft.handoffDepartment;
    }
    if (JSON.stringify(cleanReasons(draft.reasons)) !== JSON.stringify(cleanReasons(saved.reasons))) {
        change.jev_json = jevWithReasons(currentJev, cleanReasons(draft.reasons));
    }
    return change;
}

function calls(control: SofaFieldControlProps) {
    const { describedBy, invalid, required, id } = control;
    return { id, "aria-describedby": describedBy, invalid, required };
}

const API_FIELDS: Record<string, string> = {
    department: "defaultDepartment",
    handoff: "humanHandoff",
    handoff_department: "handoffDepartment",
    jev_json: "reasons",
};

export function AgentRouting() {
    const { orgId = "", agentId = "" } = useParams();
    const guard = useGuardChanges();
    const { translate: t } = useI18n();

    const [agente, setAgent] = useState<Agente | null>(null);
    const [saved, setSaved] = useState<Routing | null>(null);
    const [draft, setDraft] = useState<Routing | null>(null);
    const [versao, setVersion] = useState<number>(0);
    const [nameOrg, setOrgName] = useState<string | null>(null);
    const [errorLoad, setLoadError] = useState<string | null>(null);

    const [departments, setDepartments] = useState<Departamento[]>([]);
    const [loadingDepartments, setLoadingDepartments] = useState(true);
    const [errorDepartments, setDepartmentsError] = useState<string | null>(null);

    const [errors, setErrors] = useState<Record<string, string>>({});
    const [generalError, setGeneralError] = useState<string | null>(null);
    const [conflict, setConflict] = useState<string | null>(null);
    const [notice, setNotice] = useState<string | null>(null);
    const [saving, setSaving] = useState(false);

    const loadDepartments = useCallback(async () => {
        setLoadingDepartments(true);
        setDepartmentsError(null);
        try {
            const list = await listStructure("departamento", orgId);
            setDepartments(list as Departamento[]);
        } catch (failure) {
            setDepartmentsError(
                failure instanceof Error ? failure.message : t("routing.loadDepartmentsFailed"),
            );
            setDepartments([]);
        } finally {
            setLoadingDepartments(false);
        }
    }, [orgId, t]);

    const load = useCallback(async () => {
        setLoadError(null);
        try {
            const dados = await getAgent(orgId, agentId);
            const routing = agentRouting(dados);
            setAgent(dados);
            setVersion(dados.version);
            setSaved(routing);
            setDraft(routing);
        } catch (failure) {
            setLoadError(failure instanceof Error ? failure.message : t("routing.loadAgentFailed"));
            return;
        }
        void loadDepartments();
    }, [orgId, agentId, loadDepartments]);

    useEffect(() => {
        void load();
        getOrg(orgId)
            .then((org) => setOrgName(org.name))
            .catch(() => setOrgName(null));
    }, [load, orgId]);

    const dirty = saved !== null && draft !== null && !sameRouting(saved, draft);

    const rereadAgent = useCallback(async (): Promise<number | undefined> => {
        try {
            const current = await getAgent(orgId, agentId);
            setAgent(current);
            setVersion(current.version);
            return current.version;
        } catch {
            return undefined;
        }
    }, [orgId, agentId]);

    const save = useCallback(async (): Promise<boolean> => {
        if (!draft || !saved || !agente) return true;
        const newErrors = validateRouting(draft);
        setErrors(newErrors);
        setGeneralError(null);
        setConflict(null);
        setNotice(null);
        if (Object.keys(newErrors).length > 0) return false;

        const change = routingChange(saved, draft, agente.jev);
        if (Object.keys(change).length === 0) {
            setNotice(t("routing.nothingToSave"));
            return true;
        }

        setSaving(true);
        try {
            const recorded = await saveRouting(orgId, agentId, versao, change);
            setAgent(recorded);
            setVersion(recorded.version);
            const fresh = agentRouting(recorded);
            setSaved(fresh);
            setDraft(fresh);
            setNotice(t("routing.savedVersion", { version: recorded.version }));
            return true;
        } catch (failure) {
            if (failure instanceof ApiError && failure.status === 412) {
                const current = await rereadAgent();
                setConflict(
                    typeof current === "number"
                        ? t("routing.conflictMessageVersion", { version: current })
                        : t("routing.conflictMessage"),
                );
                return false;
            }
            if (failure instanceof ApiError) {
                const details = failure.details as Record<string, unknown> | null | undefined;
                const campo = typeof details?.["field"] === "string" ? details["field"] : "";
                const reason =
                    typeof details?.["reason"] === "string" ? details["reason"] : null;
                const chave = API_FIELDS[campo];
                if (chave && reason) {
                    setErrors((current) => ({ ...current, [chave]: reason }));
                    return false;
                }
                setGeneralError(
                    reason
                        ? t("routing.errorWithDetails", {
                              message: failure.message,
                              reason: reason,
                              code: failure.code,
                          })
                        : t("routing.errorWithCode", {
                              message: failure.message,
                              code: failure.code,
                          }),
                );
                return false;
            }
            setGeneralError(t("routing.unexpectedSaveError"));
            return false;
        } finally {
            setSaving(false);
        }
    }, [draft, saved, agente, versao, orgId, agentId, rereadAgent, t]);

    useEffect(() => {
        if (!draft) return;
        guard.register({ dirty, save: async () => save() });
    }, [guard, dirty, save, draft]);

    useEffect(() => {
        return () => guard.register(null);
    }, [guard]);

    useEffect(() => {
        function onKeyPress(evento: KeyboardEvent) {
            if ((evento.ctrlKey || evento.metaKey) && evento.key.toLowerCase() === "s") {
                evento.preventDefault();
                if (dirty && !saving) void save();
            }
        }
        window.addEventListener("keydown", onKeyPress);
        return () => window.removeEventListener("keydown", onKeyPress);
    }, [dirty, saving, save]);

    const optionsDepartment = useMemo(() => {
        const options = departments.map((departamento) => ({
            value: departamento.id,
            label:
                departamento.status === "active"
                    ? departamento.name
                    : t("routing.departmentInactive", { name: departamento.name }),
        }));
        for (const id of [draft?.defaultDepartment, draft?.handoffDepartment]) {
            if (id && !options.some((option) => option.value === id)) {
                options.push({ value: id, label: t("routing.departmentNotFound") });
            }
        }
        return options;
    }, [departments, draft?.defaultDepartment, draft?.handoffDepartment, t]);

    function change(partial: Partial<Routing>) {
        setDraft((current) => (current ? { ...current, ...partial } : current));
        setNotice(null);
    }

    function changeReason(index: number, texto: string) {
        if (!draft) return;
        const reasons = draft.reasons.map((reason, position) =>
            position === index ? texto : reason,
        );
        change({ reasons: reasons });
    }

    function removeReason(index: number) {
        if (!draft) return;
        change({ reasons: draft.reasons.filter((_, position) => position !== index) });
    }

    function addReason() {
        if (!draft) return;
        change({ reasons: [...draft.reasons, ""] });
    }

    function descartar() {
        if (!saved) return;
        setDraft(saved);
        setErrors({});
        setGeneralError(null);
        setConflict(null);
        setNotice(t("routing.discarded"));
    }

    if (!agente && errorLoad) {
        return (
            <div className="poph-page">
                <SofaErrorState
                    title={t("routing.loadErrorTitle")}
                    description={t("routing.loadErrorDescription")}
                    details={errorLoad}
                    retry={{ label: t("routing.retry"), onClick: () => void load() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            icon={<IconArrowLeft aria-hidden="true" />}
                            onClick={() => guard.tryNavigate(`/orgs/${orgId}/agents`)}
                        >
                            {t("routing.backToAgents")}
                        </SofaButton>
                    }
                />
            </div>
        );
    }

    if (!agente || !draft) {
        return (
            <div className="poph-page" aria-busy="true">
                <SofaSkeleton lines={6} />
            </div>
        );
    }

    const noDepartments = !loadingDepartments && !errorDepartments && departments.length === 0;
    const reasonsFull = draft.reasons.length >= LIMIT_REASONS;

    return (
        <div className="poph-page">
            <HeaderAgent
                org={orgId}
                nome={agente.name}
                slug={agente.slug}
                situacao={agente.status}
                organization={nameOrg}
                descricao={t("routing.headerDescription")}
            />

            <TabsAgent org={orgId} agente={agentId} active="routing" nome={agente.name} model={agente.model ?? null} />

            {conflict && (
                <SofaBanner tone="warn" title={t("routing.conflictTitle")}>
                    {conflict}
                </SofaBanner>
            )}
            {generalError && (
                <SofaBanner tone="danger" title={t("routing.saveErrorTitle")}>
                    {generalError}
                </SofaBanner>
            )}
            {notice && !generalError && !conflict && (
                <p className="poph-mono-muted" style={{ margin: 0 }} role="status">
                    {notice}
                </p>
            )}

            {errorDepartments && (
                <SofaBanner tone="danger" title={t("routing.departmentsErrorTitle")}>
                    {errorDepartments}.{" "}
                    <SofaButton variant="subtle" size="sm" onClick={() => void loadDepartments()}>
                        {t("routing.retry")}
                    </SofaButton>
                </SofaBanner>
            )}

            <SectionAgent
                title={t("routing.sectionDepartments")}
                descricao={t("routing.sectionDepartmentsDescription")}
            >
                {noDepartments ? (
                    <SofaEmptyState
                        title={t("routing.emptyDepartmentsTitle")}
                        description={t("routing.emptyDepartmentsDescription")}
                        action={
                            <SofaButton
                                variant="primary"
                                icon={<IconPlus aria-hidden="true" />}
                                onClick={() => guard.tryNavigate(`/orgs/${orgId}/structure`)}
                            >
                                {t("routing.goToStructure")}
                            </SofaButton>
                        }
                    />
                ) : loadingDepartments && departments.length === 0 ? (
                    <SofaSkeleton lines={3} />
                ) : (
                    <form
                        id="poph-form-roteamento"
                        className="poph-stack"
                        style={{ gap: "var(--pop-spacing-lg)" }}
                        onSubmit={(evento) => {
                            evento.preventDefault();
                            void save();
                        }}
                        noValidate
                    >
                        <SofaField
                            label={t("routing.defaultDepartmentLabel")}
                            hint={t("routing.defaultDepartmentHint")}
                            error={errors["defaultDepartment"]}
                            htmlFor="poph-roteamento-padrao"
                        >
                            {(control) => (
                                <SofaSelect
                                    {...calls(control)}
                                    value={draft.defaultDepartment ?? ""}
                                    onChange={(evento) =>
                                        change({ defaultDepartment: evento.target.value || null })
                                    }
                                >
                                    {draft.defaultDepartment === null && (
                                        <option value="">{t("routing.chooseDepartment")}</option>
                                    )}
                                    {optionsDepartment.map((option) => (
                                        <option key={option.value} value={option.value}>
                                            {option.label}
                                        </option>
                                    ))}
                                </SofaSelect>
                            )}
                        </SofaField>

                        <SofaSwitch
                            label={t("routing.handoffLabel")}
                            description={t("routing.handoffDescription")}
                            checked={draft.humanHandoff}
                            onChange={(evento) =>
                                change({ humanHandoff: evento.target.checked })
                            }
                        />

                        <SofaField
                            label={t("routing.humanDepartmentLabel")}
                            hint={t("routing.humanDepartmentHint")}
                            error={errors["handoffDepartment"]}
                            htmlFor="poph-roteamento-humano"
                        >
                            {(control) => (
                                <SofaSelect
                                    {...calls(control)}
                                    value={draft.handoffDepartment ?? ""}
                                    onChange={(evento) =>
                                        change({ handoffDepartment: evento.target.value || null })
                                    }
                                >
                                    {draft.handoffDepartment === null && (
                                        <option value="">{t("routing.chooseDepartment")}</option>
                                    )}
                                    {optionsDepartment.map((option) => (
                                        <option key={option.value} value={option.value}>
                                            {option.label}
                                        </option>
                                    ))}
                                </SofaSelect>
                            )}
                        </SofaField>
                    </form>
                )}
            </SectionAgent>

            <SectionAgent
                title={t("routing.sectionReasons")}
                descricao={t("routing.sectionReasonsDescription", {
                    limit: LIMIT_REASONS,
                    maxLength: LIMIT_REASON,
                })}
                actions={
                    <SofaButton
                        variant="secondary"
                        size="sm"
                        icon={<IconPlus aria-hidden="true" />}
                        disabled={reasonsFull}
                        onClick={addReason}
                    >
                        {t("routing.addReason")}
                    </SofaButton>
                }
            >
                {draft.reasons.length === 0 ? (
                    <p className="poph-mono-muted poph-routing__empty">{t("routing.noReasons")}</p>
                ) : (
                    <div className="poph-routing__reasons">
                        {draft.reasons.map((reason, index) => (
                            <div key={index} className="poph-routing__reason">
                                <div className="poph-routing__reason-field">
                                    <SofaField
                                        label={t("routing.reasonLabel", { number: index + 1 })}
                                        hint={`${reason.trim().length}/${LIMIT_REASON}`}
                                        error={errors[`motivo-${index}`]}
                                        htmlFor={`poph-motivo-${index}`}
                                    >
                                        {(control) => (
                                            <SofaInput
                                                {...calls(control)}
                                                value={reason}
                                                autoComplete="off"
                                                onChange={(evento) =>
                                                    changeReason(index, evento.target.value)
                                                }
                                            />
                                        )}
                                    </SofaField>
                                </div>
                                <SofaIconButton
                                    label={t("routing.removeReasonLabel", { number: index + 1 })}
                                    onClick={() => removeReason(index)}
                                >
                                    <IconTrash aria-hidden="true" />
                                </SofaIconButton>
                            </div>
                        ))}
                    </div>
                )}
                {errors["reasons"] && (
                    <p role="alert" className="poph-routing__error">
                        {errors["reasons"]}
                    </p>
                )}
            </SectionAgent>

            <div className="poph-line-title poph-routing__bar">
                <span
                    className={`poph-status ${dirty ? "poph-status--attention" : "poph-status--inactive"}`}
                >
                    <span className="poph-status__point" aria-hidden="true" />
                    {dirty
                        ? t("routing.unsavedChanges")
                        : t("routing.savedStatus", { version: versao })}
                </span>
                <SofaButton variant="subtle" disabled={!dirty || saving} onClick={descartar}>
                    {t("routing.discardChanges")}
                </SofaButton>
                <SofaButton
                    variant="primary"
                    icon={<IconRefresh aria-hidden="true" />}
                    disabled={!dirty}
                    loading={saving}
                    onClick={() => void save()}
                >
                    {t("routing.saveRouting")}
                </SofaButton>
            </div>
        </div>
    );
}
