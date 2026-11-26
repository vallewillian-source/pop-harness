
import { useCallback, useEffect, useMemo, useState } from "react";
import { useParams } from "react-router-dom";
import {
    IconArrowLeft,
    IconRefresh,
    SofaBanner,
    SofaButton,
    SofaErrorState,
    SofaField,
    type SofaFieldControlProps,
    SofaInput,
    SofaSelect,
    SofaSkeleton,
    SofaSwitch,
} from "@sofacoding/sofa-ui";
import { ApiError } from "../api/client";
import { getAgent, type Agente } from "../api/agents";
import { getOrg } from "../api/orgs";
import {
    PRESERVED_KEYS,
    type ModelField,
    GATE_DESCRIPTION,
    type GateState,
    GATES,
    type IdGate,
    LIMIT_CONTEXT_MIN,
    LIMIT_CUT_MAX,
    LIMIT_CUT_MIN,
    LIMIT_TOOLS_MAX,
    LIMIT_WINDOW_MS,
    LIMIT_ROUNDS_MAX,
    LIMIT_TEMPERATURE_MAX,
    LIMIT_TEMPERATURE_MIN,
    LIMIT_TOP_K_MAX,
    LIMIT_TOP_K_MIN,
    type TextModel,
    FACTORY_DEFAULTS,
    type JevValues,
    gateEffect,
    agentJev,
    agentModel,
    jevChange,
    modelChanges,
    saveModelJev,
    validateModel,
} from "../api/modelJev";
import { TabsAgent, HeaderAgent, SectionAgent } from "../components/TabsAgent";
import { useGuardChanges } from "../components/GuardChanges";
import { translate, useI18n } from "../i18n";
import "../styles/model-jev.css";

function calls(control: SofaFieldControlProps) {
    const { describedBy, invalid, required, id } = control;
    return { id, "aria-describedby": describedBy, invalid, required };
}

function sameModel(a: TextModel, b: TextModel): boolean {
    return (Object.keys(a) as ModelField[]).every((campo) => a[campo].trim() === b[campo].trim());
}

function sameJev(a: JevValues, b: JevValues): boolean {
    return a.jevDesligado === b.jevDesligado && GATES.every((gate) => a.gates[gate] === b.gates[gate]);
}

function statusGate(values: JevValues, gate: IdGate): string {
    const on = gateEffect(values, gate);
    if (values.jevDesligado) return translate("modelJev.gateStatusDisabledBySwitch");
    if (values.gates[gate] === "herdar") {
        return translate("modelJev.gateStatusInherit", {
            state: on ? translate("modelJev.gateStateOnLower") : translate("modelJev.gateStateOffLower"),
        });
    }
    return on
        ? translate("modelJev.gateStatusActiveInAgent")
        : translate("modelJev.gateStatusInactiveInAgent");
}

function definedKeys(jev: Record<string, unknown> | undefined): Set<string> {
    return new Set(Object.keys(jev ?? {}));
}

export function AgentModelJev() {
    const { translate: t } = useI18n();
    const { orgId = "", agentId = "" } = useParams();
    const guard = useGuardChanges();

    const [agente, setAgent] = useState<Agente | null>(null);
    const [versao, setVersion] = useState<number>(0);
    const [baseModel, setBaseModel] = useState<TextModel | null>(null);
    const [draftModel, setDraftModel] = useState<TextModel | null>(null);
    const [baseJev, setBaseJev] = useState<JevValues | null>(null);
    const [draftJev, setDraftJev] = useState<JevValues | null>(null);
    const [nameOrg, setOrgName] = useState<string | null>(null);
    const [errorLoad, setLoadError] = useState<string | null>(null);

    const [errors, setErrors] = useState<Partial<Record<ModelField, string>>>({});
    const [generalError, setGeneralError] = useState<string | null>(null);
    const [conflict, setConflict] = useState<string | null>(null);
    const [notice, setNotice] = useState<string | null>(null);
    const [saving, setSaving] = useState(false);

    const applyAgent = useCallback((dados: Agente) => {
        setAgent(dados);
        setVersion(dados.version);
        const modelo = agentModel(dados);
        const jev = agentJev(dados.jev);
        setBaseModel(modelo);
        setDraftModel(modelo);
        setBaseJev(jev);
        setDraftJev(jev);
    }, []);

    const load = useCallback(async () => {
        setLoadError(null);
        try {
            applyAgent(await getAgent(orgId, agentId));
        } catch (failure) {
            setLoadError(
                failure instanceof Error ? failure.message : t("modelJev.loadAgentFallbackError"),
            );
        }
    }, [orgId, agentId, applyAgent, t]);

    useEffect(() => {
        void load();
        getOrg(orgId)
            .then((org) => setOrgName(org.name))
            .catch(() => setOrgName(null));
    }, [load, orgId]);

    const dirty =
        baseModel !== null &&
        draftModel !== null &&
        baseJev !== null &&
        draftJev !== null &&
        (!sameModel(baseModel, draftModel) || !sameJev(baseJev, draftJev));

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
        if (!agente || !baseModel || !draftModel || !baseJev || !draftJev) return true;
        const newErrors = validateModel(draftModel, baseModel);
        setErrors(newErrors);
        setGeneralError(null);
        setConflict(null);
        setNotice(null);
        if (Object.keys(newErrors).length > 0) return false;

        const modelChange = modelChanges(baseModel, draftModel);
        const jevJson = jevChange(agente.jev, baseJev, draftJev);
        const change = jevJson === undefined ? modelChange : { ...modelChange, jev_json: jevJson };
        if (Object.keys(change).length === 0) {
            setNotice(t("modelJev.nothingToSave"));
            return true;
        }

        setSaving(true);
        try {
            const recorded = await saveModelJev(orgId, agentId, versao, change);
            applyAgent(recorded);
            setNotice(t("modelJev.savedNotice", { version: recorded.version }));
            return true;
        } catch (failure) {
            if (failure instanceof ApiError && failure.status === 412) {
                const current = await rereadAgent();
                setConflict(
                    t("modelJev.conflictMessage", {
                        versionSuffix:
                            typeof current === "number"
                                ? t("modelJev.conflictVersionSuffix", { version: current })
                                : "",
                    }),
                );
                return false;
            }
            if (failure instanceof ApiError) {
                const details = failure.detalhes as Record<string, unknown> | null | undefined;
                const campo = typeof details?.["field"] === "string" ? details["field"] : "";
                const motivo = typeof details?.["reason"] === "string" ? details["reason"] : null;
                const campos = baseModel ? (Object.keys(baseModel) as string[]) : [];
                if (motivo && campos.includes(campo)) {
                    setErrors((current) => ({ ...current, [campo]: motivo }));
                    return false;
                }
                setGeneralError(
                    motivo
                        ? t("modelJev.apiErrorWithReason", {
                              message: failure.message,
                              reason: motivo,
                              code: failure.codigo,
                          })
                        : t("modelJev.apiErrorWithoutReason", {
                              message: failure.message,
                              code: failure.codigo,
                          }),
                );
                return false;
            }
            setGeneralError(t("modelJev.unexpectedSaveError"));
            return false;
        } finally {
            setSaving(false);
        }
    }, [agente, baseModel, draftModel, baseJev, draftJev, versao, orgId, agentId, applyAgent, rereadAgent, t]);

    useEffect(() => {
        if (!draftModel) return;
        guard.register({ dirty, save: async () => save() });
    }, [guard, dirty, save, draftModel]);

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

    function changeModel(campo: ModelField, value: string) {
        setDraftModel((current) => (current ? { ...current, [campo]: value } : current));
        setErrors((current) => ({ ...current, [campo]: undefined }));
        setNotice(null);
    }

    function changeGate(gate: IdGate, estado: GateState) {
        setDraftJev((current) =>
            current ? { ...current, gates: { ...current.gates, [gate]: estado } } : current,
        );
        setNotice(null);
    }

    function changeSwitch(off: boolean) {
        setDraftJev((current) => (current ? { ...current, jevDesligado: off } : current));
        setNotice(null);
    }

    function descartar() {
        if (!baseModel || !baseJev) return;
        setDraftModel(baseModel);
        setDraftJev(baseJev);
        setErrors({});
        setGeneralError(null);
        setConflict(null);
        setNotice(t("modelJev.discardedNotice"));
    }

    const keysAgent = useMemo(() => definedKeys(agente?.jev), [agente]);

    if (!agente && errorLoad) {
        return (
            <div className="poph-page">
                <SofaErrorState
                    title={t("modelJev.loadErrorTitle")}
                    description={t("modelJev.loadErrorDescription")}
                    details={errorLoad}
                    retry={{ label: t("modelJev.retry"), onClick: () => void load() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            icon={<IconArrowLeft aria-hidden="true" />}
                            onClick={() => guard.tryNavigate(`/orgs/${orgId}/agents`)}
                        >
                            {t("modelJev.backToAgents")}
                        </SofaButton>
                    }
                />
            </div>
        );
    }

    if (!agente || !draftModel || !draftJev) {
        return (
            <div className="poph-page" aria-busy="true">
                <div className="poph-page__top">
                    <div>
                        <SofaSkeleton width={260} height={26} />
                        <div className="poph-model-jev__skeleton-line">
                            <SofaSkeleton width={360} height={12} />
                        </div>
                    </div>
                </div>
                <SofaSkeleton lines={8} />
            </div>
        );
    }

    function fieldText(
        campo: ModelField,
        label: string,
        options: { hint?: string; placeholder?: string; mode?: "decimal" | "numeric" } = {},
    ) {
        return (
            <SofaField
                label={label}
                hint={options.hint}
                error={errors[campo]}
                htmlFor={`poph-modelo-${campo}`}
            >
                {(control) => (
                    <SofaInput
                        {...calls(control)}
                        value={draftModel![campo]}
                        placeholder={options.placeholder}
                        inputMode={options.mode}
                        autoComplete="off"
                        onChange={(evento) => changeModel(campo, evento.target.value)}
                    />
                )}
            </SofaField>
        );
    }

    return (
        <>
            <div className="poph-page poph-model-jev">
                <HeaderAgent
                    org={orgId}
                    nome={agente.name}
                    slug={agente.slug}
                    situacao={agente.status}
                    organization={nameOrg}
                    descricao={t("modelJev.headerDescription")}
                />

                <TabsAgent org={orgId} agente={agentId} active="model-jev" nome={agente.name} model={agente.model ?? null} />

                {conflict && (
                    <SofaBanner tone="warn" title={t("modelJev.bannerOutdatedTitle")}>
                        {conflict}
                    </SofaBanner>
                )}
                {generalError && (
                    <SofaBanner tone="danger" title={t("modelJev.bannerSaveErrorTitle")}>
                        {generalError}
                    </SofaBanner>
                )}
                {notice && !generalError && !conflict && (
                    <p className="poph-mono-muted" style={{ margin: 0 }} role="status">
                        {notice}
                    </p>
                )}

                <form
                    id="poph-form-modelo-jev"
                    className="poph-model-jev__form"
                    onSubmit={(evento) => {
                        evento.preventDefault();
                        void save();
                    }}
                    noValidate
                >
                    <SectionAgent
                        title={t("modelJev.sectionModelTitle")}
                        descricao={t("modelJev.sectionModelDescription")}
                    >
                        <div className="poph-model-jev__grid">
                            {fieldText("provider", t("modelJev.fieldProviderLabel"), {
                                hint: t("modelJev.fieldProviderHint"),
                            })}
                            {fieldText("model", t("modelJev.fieldModelLabel"), {
                                hint: t("modelJev.fieldModelHint"),
                            })}
                            {fieldText("temperature", t("modelJev.fieldTemperatureLabel"), {
                                mode: "decimal",
                                placeholder:
                                    agente.temperature == null
                                        ? t("modelJev.placeholderInheritsDefault")
                                        : undefined,
                                hint: t("modelJev.fieldTemperatureHint", {
                                    min: LIMIT_TEMPERATURE_MIN,
                                    max: LIMIT_TEMPERATURE_MAX,
                                }),
                            })}
                            {fieldText("max_output_tokens", t("modelJev.fieldMaxOutputLabel"), {
                                mode: "numeric",
                                placeholder:
                                    agente.max_output_tokens == null
                                        ? t("modelJev.placeholderInheritsDefault")
                                        : undefined,
                                hint: t("modelJev.fieldMaxOutputHint"),
                            })}
                            {fieldText("max_context_tokens", t("modelJev.fieldMaxContextLabel"), {
                                mode: "numeric",
                                hint: t("modelJev.fieldMaxContextHint", { min: LIMIT_CONTEXT_MIN }),
                            })}
                        </div>
                    </SectionAgent>

                    <SectionAgent
                        title={t("modelJev.sectionServiceTitle")}
                        descricao={t("modelJev.sectionServiceDescription")}
                    >
                        <div className="poph-model-jev__grid">
                            {fieldText("grouping_window_ms", t("modelJev.fieldGroupingWindowLabel"), {
                                mode: "numeric",
                                hint: t("modelJev.fieldGroupingWindowHint", { max: LIMIT_WINDOW_MS }),
                            })}
                            {fieldText("max_tool_rounds", t("modelJev.fieldMaxToolRoundsLabel"), {
                                mode: "numeric",
                                hint: t("modelJev.fieldMaxToolRoundsHint", { max: LIMIT_ROUNDS_MAX }),
                            })}
                            {fieldText("max_tool_calls", t("modelJev.fieldMaxToolCallsLabel"), {
                                mode: "numeric",
                                hint: t("modelJev.fieldMaxToolCallsHint", { max: LIMIT_TOOLS_MAX }),
                            })}
                        </div>
                    </SectionAgent>

                    <SectionAgent
                        title={t("modelJev.sectionRagTitle")}
                        descricao={t("modelJev.sectionRagDescription")}
                    >
                        <div className="poph-model-jev__grid">
                            {fieldText("rag_top_k", t("modelJev.fieldRagTopKLabel"), {
                                mode: "numeric",
                                hint: t("modelJev.fieldRagTopKHint", {
                                    min: LIMIT_TOP_K_MIN,
                                    max: LIMIT_TOP_K_MAX,
                                }),
                            })}
                            {fieldText("rag_min_score", t("modelJev.fieldRagMinScoreLabel"), {
                                mode: "decimal",
                                hint: t("modelJev.fieldRagMinScoreHint", {
                                    min: LIMIT_CUT_MIN,
                                    max: LIMIT_CUT_MAX,
                                }),
                            })}
                        </div>
                    </SectionAgent>

                    <SectionAgent
                        title={t("modelJev.sectionJevTitle")}
                        descricao={t("modelJev.sectionJevDescription")}
                    >
                        <SofaSwitch
                            label={t("modelJev.switchDisableJevLabel")}
                            description={t("modelJev.switchDisableJevDescription")}
                            checked={draftJev.jevDesligado}
                            onChange={(evento) => changeSwitch(evento.target.checked)}
                        />

                        <div className="poph-model-jev__grid">
                            {GATES.map((gate) => {
                                const descricao = GATE_DESCRIPTION[gate];
                                return (
                                    <SofaField
                                        key={gate}
                                        label={descricao.titulo}
                                        hint={t("modelJev.gateHintToday", {
                                            explanation: descricao.explicacao,
                                            status: statusGate(draftJev, gate),
                                        })}
                                        htmlFor={`poph-gate-${gate}`}
                                    >
                                        {(control) => (
                                            <SofaSelect
                                                {...calls(control)}
                                                value={draftJev.gates[gate]}
                                                onChange={(evento) =>
                                                    changeGate(gate, evento.target.value as GateState)
                                                }
                                            >
                                                <option value="herdar">
                                                    {t("modelJev.gateOptionInherit", {
                                                        defaultState: FACTORY_DEFAULTS[gate]
                                                            ? t("modelJev.gateStateOnLower")
                                                            : t("modelJev.gateStateOffLower"),
                                                    })}
                                                </option>
                                                <option value="ligado">{t("modelJev.gateOptionOn")}</option>
                                                <option value="desligado">{t("modelJev.gateOptionOff")}</option>
                                            </SofaSelect>
                                        )}
                                    </SofaField>
                                );
                            })}
                        </div>
                    </SectionAgent>

                    <SectionAgent
                        title={t("modelJev.sectionOtherKeysTitle")}
                        descricao={t("modelJev.sectionOtherKeysDescription")}
                    >
                        <ul className="poph-model-jev__keys">
                            {PRESERVED_KEYS.map((item) => (
                                <li key={item.chave}>
                                    <code>{item.chave}</code> ·{" "}
                                    {keysAgent.has(item.chave)
                                        ? t("modelJev.keyDefined")
                                        : t("modelJev.keyUndefined")}
                                    <div className="poph-mono-muted">
                                        {item.explicacao}
                                        {item.ondeEdita
                                            ? t("modelJev.keyEditIn", { where: item.ondeEdita })
                                            : ""}
                                    </div>
                                </li>
                            ))}
                        </ul>
                    </SectionAgent>
                </form>

                <div className="poph-model-jev__bar">
                    <span
                        className={`poph-status ${dirty ? "poph-status--attention" : "poph-status--inactive"}`}
                    >
                        <span className="poph-status__point" aria-hidden="true" />
                        {dirty
                            ? t("modelJev.statusUnsaved")
                            : t("modelJev.statusSaved", { version: versao })}
                    </span>
                    <SofaButton variant="subtle" disabled={!dirty || saving} onClick={descartar}>
                        {t("modelJev.discardChanges")}
                    </SofaButton>
                    <SofaButton
                        variant="primary"
                        icon={<IconRefresh aria-hidden="true" />}
                        disabled={!dirty}
                        loading={saving}
                        onClick={() => void save()}
                    >
                        {t("modelJev.saveButton")}
                    </SofaButton>
                </div>
            </div>
        </>
    );
}
