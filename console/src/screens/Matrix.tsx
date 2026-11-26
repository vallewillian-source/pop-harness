
import { useCallback, useEffect, useId, useRef, useState, type FormEvent } from "react";
import { useParams } from "react-router-dom";
import {
    IconRefresh,
    SofaBanner,
    SofaButton,
    SofaConfirmDialog,
    SofaEmptyState,
    SofaErrorState,
    SofaField,
    SofaInput,
    SofaModal,
    SofaSkeleton,
} from "@sofacoding/sofa-ui";
import { ApiError } from "../api/client";
import { translate, useI18n } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";
import { formatDateTime, formatNumber } from "../i18n/format";
import { listStructure, type Departamento } from "../api/structure";
import {
    updateMatrixModule,
    unlinkMatrixSpace,
    readMatrixState,
    linkMatrixSpace,
    listMatrixSpaces,
    stateVersion,
    type MatrixState,
    type Espaco,
} from "../api/matrix";
import { formatUptime, messageFailure } from "./Operation";
import "../styles/matrix.css";

export function conflictMessage(): string {
    return translate("matrix.conflict");
}

type TextSource =
    | { key: TranslationKey }
    | { erro: unknown; what: TranslationKey; espaco?: true };

function textNotice(fonte: TextSource): string {
    if ("key" in fonte) return translate(fonte.key);
    const what = translate(fonte.what);
    return fonte.espaco ? messageSpace(fonte.erro, what) : messageFailure(fonte.erro, what);
}

type Load<T> = { loading: boolean; dados: T | null; erro: TextSource | null };
type Notice = { tom: "ok" | "error"; fonte: TextSource } | null;

export type RowSpace = {
    id: string;
    nome: string;
    espaco: Espaco | null;
    archived: boolean;
};

const loadInitial = <T,>(): Load<T> => ({ loading: true, dados: null, erro: null });

export function statusConnection(connected: boolean | null | undefined): { value: string; detalhe?: string } {
    if (connected === true) return { value: translate("matrix.connectionConnected") };
    if (connected === false) {
        return {
            value: translate("matrix.connectionDisconnected"),
            detalhe: translate("matrix.connectionDisconnectedDetail"),
        };
    }
    return {
        value: translate("matrix.connectionUnknown"),
        detalhe: translate("matrix.connectionUnknownDetail"),
    };
}

export function valueLag(messages: number): string {
    return translate(messages === 1 ? "matrix.backlogMessageOne" : "matrix.backlogMessageMany", {
        count: formatNumber(messages),
    });
}

export function valueFailures(quantity: number): string {
    return translate(quantity === 1 ? "matrix.failuresPendingOne" : "matrix.failuresPendingMany", {
        count: formatNumber(quantity),
    });
}

export function textOrigin(source: string): string {
    if (source === "auto") return translate("matrix.originAuto");
    if (source === "linked") return translate("matrix.originLinked");
    return source;
}

export function buildRows(departments: Departamento[] | null, spaces: Espaco[]): RowSpace[] {
    const activeList = (departments ?? []).filter((departamento) => departamento.status === "active");
    const rows: RowSpace[] = activeList.map((departamento) => ({
        id: departamento.id,
        nome: departamento.name,
        espaco: spaces.find((espaco) => espaco.department_id === departamento.id) ?? null,
        archived: false,
    }));
    for (const espaco of spaces) {
        if (activeList.some((departamento) => departamento.id === espaco.department_id)) continue;
        rows.push({
            id: espaco.department_id,
            nome: espaco.department_name ?? translate("matrix.departmentUnnamed"),
            espaco,
            archived: departments !== null,
        });
    }
    return rows;
}

export function messageSpace(error: unknown, what: string): string {
    if (error instanceof ApiError && (error.status === 400 || error.status === 409) && error.message) {
        return error.message;
    }
    return messageFailure(error, what);
}

function dateCut(ms: number): string {
    return formatDateTime(new Date(ms));
}

function Indicador({
    label,
    value,
    detail,
    tom,
}: {
    label: string;
    value: string;
    detail?: string;
    tom?: "ok" | "alert" | "error";
}) {
    return (
        <div className="poph-matrix__indicator">
            <dt className="poph-matrix__indicator-label">{label}</dt>
            <dd className="poph-matrix__indicator-body">
                <span
                    className={
                        tom ? `poph-matrix__indicator-value poph-matrix__indicator-value--${tom}` : "poph-matrix__indicator-value"
                    }
                >
                    {value}
                </span>
                {detail ? <span className="poph-matrix__indicator-detail">{detail}</span> : null}
            </dd>
        </div>
    );
}

export function Matrix() {
    const { orgId = "" } = useParams();
    const { translate: t } = useI18n();

    const [module, setModule] = useState<Load<MatrixState>>(loadInitial);
    const [spaces, setSpaces] = useState<Load<Espaco[]>>(loadInitial);
    const [departments, setDepartments] = useState<Load<Departamento[]>>(loadInitial);

    const [sendingModule, setSendingModule] = useState(false);
    const [noticeModule, setModuleNotice] = useState<Notice>(null);
    const [confirmingTurnOffModule, setConfirmingTurnOffModule] = useState(false);

    const [creating, setCreatingFor] = useState<string | null>(null);
    const [noticeSpace, setSpaceNotice] = useState<Notice>(null);
    const [spaceTurnOff, setSpaceToTurnOff] = useState<Espaco | null>(null);
    const [turningOffSpace, setTurningOffSpace] = useState(false);

    const [turningOn, setCallingA] = useState<{ id: string; nome: string } | null>(null);
    const [typedSpace, setSpaceTyped] = useState("");
    const [sendingCall, setSendingCall] = useState(false);
    const [errorDialog, setDialogError] = useState<TextSource | null>(null);

    const alive = useRef(true);
    const inProgress = useRef(false);
    const moduleId = useId();
    const spacesId = useId();
    const callFormId = useId();
    const existingSpaceId = useId();

    useEffect(() => {
        alive.current = true;
        return () => {
            alive.current = false;
        };
    }, []);

    const load = useCallback(async () => {
        if (inProgress.current) return;
        inProgress.current = true;
        setModule((current) => ({ ...current, loading: true }));
        setSpaces((current) => ({ ...current, loading: true }));
        setDepartments((current) => ({ ...current, loading: true }));

        const [resModule, resSpaces, resDepartments] = await Promise.allSettled([
            readMatrixState(orgId),
            listMatrixSpaces(orgId),
            listStructure("departamento", orgId),
        ]);
        inProgress.current = false;
        if (!alive.current) return;

        if (resModule.status === "fulfilled") {
            setModule({ loading: false, dados: resModule.value, erro: null });
        } else {
            setModule((current) => ({
                loading: false,
                dados: current.dados,
                erro: { erro: resModule.reason, what: "operation.whatMatrixModule" },
            }));
        }

        if (resSpaces.status === "fulfilled") {
            setSpaces({ loading: false, dados: resSpaces.value, erro: null });
        } else {
            setSpaces((current) => ({
                loading: false,
                dados: current.dados,
                erro: { erro: resSpaces.reason, what: "operation.whatSpaces" },
            }));
        }

        if (resDepartments.status === "fulfilled") {
            setDepartments({
                loading: false,
                dados: resDepartments.value as Departamento[],
                erro: null,
            });
        } else {
            setDepartments((current) => ({
                loading: false,
                dados: current.dados,
                erro: { erro: resDepartments.reason, what: "operation.whatDepartments" },
            }));
        }
    }, [orgId]);

    useEffect(() => {
        void load();
    }, [load]);

    const reloadSpaces = useCallback(async () => {
        try {
            const list = await listMatrixSpaces(orgId);
            if (alive.current) setSpaces({ loading: false, dados: list, erro: null });
        } catch (error) {
            if (alive.current) {
                setSpaces((current) => ({
                    ...current,
                    erro: { erro: error, what: "operation.whatSpaces" },
                }));
            }
        }
    }, [orgId]);

    async function toggleModule(on: boolean) {
        const current = module.dados;
        if (!current || sendingModule) return;
        setSendingModule(true);
        setModuleNotice(null);
        try {
            const fresh = await updateMatrixModule(orgId, on, stateVersion(current));
            if (!alive.current) return;
            setModule({ loading: false, dados: fresh, erro: null });
            setModuleNotice({
                tom: "ok",
                fonte: {
                    key: on ? "matrix.moduleEnabledNotice" : "matrix.moduleDisabledNotice",
                },
            });
        } catch (error) {
            if (!alive.current) return;
            if (error instanceof ApiError && error.status === 412) {
                setModuleNotice({ tom: "error", fonte: { key: "matrix.conflict" } });
                void load();
            } else {
                setModuleNotice({
                    tom: "error",
                    fonte: { erro: error, what: "operation.whatMatrixModule" },
                });
            }
        } finally {
            if (alive.current) setSendingModule(false);
        }
    }

    async function createSpace(row: RowSpace) {
        if (creating !== null) return;
        setCreatingFor(row.id);
        setSpaceNotice(null);
        try {
            await linkMatrixSpace(orgId, { department_id: row.id, space: null });
            if (!alive.current) return;
            setSpaceNotice({ tom: "ok", fonte: { key: "matrix.spaceCreated" } });
            await reloadSpaces();
        } catch (error) {
            if (alive.current) {
                setSpaceNotice({
                    tom: "error",
                    fonte: { erro: error, what: "operation.whatDepartment", espaco: true },
                });
            }
        } finally {
            if (alive.current) setCreatingFor(null);
        }
    }

    function openCall(row: RowSpace) {
        setCallingA({ id: row.id, nome: row.nome });
        setSpaceTyped("");
        setDialogError(null);
    }

    function closeCall() {
        if (sendingCall) return;
        setCallingA(null);
        setSpaceTyped("");
        setDialogError(null);
    }

    async function linkExisting(evento: FormEvent) {
        evento.preventDefault();
        const espaco = typedSpace.trim();
        if (!turningOn || !espaco || sendingCall) return;
        setSendingCall(true);
        setDialogError(null);
        try {
            await linkMatrixSpace(orgId, { department_id: turningOn.id, space: espaco });
            if (!alive.current) return;
            setCallingA(null);
            setSpaceTyped("");
            setSpaceNotice({ tom: "ok", fonte: { key: "matrix.spaceLinked" } });
            await reloadSpaces();
        } catch (error) {
            if (alive.current) {
                setDialogError({ erro: error, what: "operation.whatDepartment", espaco: true });
            }
        } finally {
            if (alive.current) setSendingCall(false);
        }
    }

    async function turnOffSpace(espaco: Espaco) {
        if (turningOffSpace) return;
        setTurningOffSpace(true);
        setSpaceNotice(null);
        try {
            await unlinkMatrixSpace(orgId, espaco.department_id);
            if (!alive.current) return;
            setSpaceToTurnOff(null);
            setSpaceNotice({
                tom: "ok",
                fonte: { key: "matrix.spaceDisconnected" },
            });
            await reloadSpaces();
        } catch (error) {
            if (!alive.current) return;
            setSpaceToTurnOff(null);
            setSpaceNotice({
                tom: "error",
                fonte: { erro: error, what: "operation.whatSpace", espaco: true },
            });
        } finally {
            if (alive.current) setTurningOffSpace(false);
        }
    }

    const m = module.dados;
    const list = spaces.dados;
    const deptos = departments.dados;
    const rows = list ? buildRows(deptos, list) : [];
    const loadingAny = module.loading || spaces.loading || departments.loading;
    const nameSpace = (espaco: Espaco) => espaco.department_name ?? t("matrix.departmentUnnamed");
    const connection = statusConnection(m?.connected);
    const mirroring = m?.enabled === true;

    return (
        <div className="poph-page poph-matrix">
            <div className="poph-page__top">
                <div>
                    <h1 className="poph-page__title">{t("matrix.pageTitle")}</h1>
                    <p className="poph-page__description">{t("matrix.pageDescription")}</p>
                </div>
                <div className="poph-page__actions">
                    <SofaButton
                        variant="subtle"
                        icon={<IconRefresh aria-hidden="true" />}
                        onClick={() => void load()}
                        disabled={loadingAny}
                    >
                        {loadingAny ? t("matrix.refreshing") : t("matrix.refresh")}
                    </SofaButton>
                    {sendingModule ? (
                        <span className="poph-matrix__status" role="status">{t("matrix.saving")}</span>
                    ) : null}
                    {m ? (
                        mirroring ? (
                            <SofaButton
                                variant="danger"
                                disabled={sendingModule}
                                onClick={() => setConfirmingTurnOffModule(true)}
                            >
                                {t("matrix.disableModule")}
                            </SofaButton>
                        ) : (
                            <SofaButton
                                variant="primary"
                                loading={sendingModule}
                                disabled={sendingModule}
                                onClick={() => void toggleModule(true)}
                            >
                                {t("matrix.enableModule")}
                            </SofaButton>
                        )
                    ) : null}
                </div>
            </div>

            <section className="poph-section poph-matrix__summary" aria-labelledby={moduleId}>
                <h2 id={moduleId} className="poph-matrix__hidden">{t("matrix.moduleSectionLabel")}</h2>
                {module.loading && !m ? (
                    <dl className="poph-matrix__indicators" aria-busy="true">
                        {[
                            t("matrix.indicatorModule"),
                            t("matrix.indicatorSynapse"),
                            t("matrix.indicatorBacklog"),
                            t("matrix.indicatorFailures"),
                            t("matrix.indicatorLastFailure"),
                        ].map(
                            (label) => (
                                <div className="poph-matrix__indicator" key={label}>
                                    <dt className="poph-matrix__indicator-label">{label}</dt>
                                    <dd className="poph-matrix__indicator-body">
                                        <SofaSkeleton lines={2} />
                                    </dd>
                                </div>
                            ),
                        )}
                    </dl>
                ) : module.erro && !m ? (
                    <SofaErrorState
                        title={t("matrix.moduleLoadFailed")}
                        description={textNotice(module.erro)}
                        retry={{ label: t("operation.retry"), onClick: () => void load() }}
                    />
                ) : m ? (
                    <>
                        {module.erro ? (
                            <p className="poph-matrix__notice" role="alert">
                                {t("matrix.lastUpdateFailed", { message: textNotice(module.erro) })}
                            </p>
                        ) : null}

                        {!m.enabled ? (
                            <SofaBanner tone="warn" title={t("matrix.disabledTitle")}>
                                {t("matrix.disabledText")}
                            </SofaBanner>
                        ) : null}

                        {m.enabled && m.connected === false ? (
                            <SofaBanner
                                tone="danger"
                                title={t("matrix.synapseTitle")}
                                action={
                                    <SofaButton variant="subtle" onClick={() => void load()}>
                                        {t("matrix.reconnect")}
                                    </SofaButton>
                                }
                            >
                                {t("matrix.synapseText")}
                            </SofaBanner>
                        ) : null}

                        {noticeModule ? (
                            <SofaBanner tone={noticeModule.tom === "ok" ? "success" : "danger"}>
                                {textNotice(noticeModule.fonte)}
                            </SofaBanner>
                        ) : null}

                        <dl className="poph-matrix__indicators">
                            <Indicador
                                label={t("matrix.indicatorModule")}
                                value={m.enabled ? t("matrix.valueEnabled") : t("matrix.valueDisabled")}
                                tom={m.enabled ? "ok" : undefined}
                                detail={
                                    m.enabled
                                        ? m.enabled_at
                                            ? t("matrix.detailActiveSince", {
                                                  date: dateCut(m.enabled_at),
                                              })
                                            : t("matrix.detailActive")
                                        : m.enabled_at
                                          ? t("matrix.detailPausedSince", {
                                                date: dateCut(m.enabled_at),
                                            })
                                          : t("matrix.detailPausedNever")
                                }
                            />
                            <Indicador
                                label={t("matrix.indicatorSynapse")}
                                value={connection.value}
                                tom={m.connected === true ? "ok" : m.connected === false ? "error" : undefined}
                                detail={connection.detalhe}
                            />
                            <Indicador
                                label={t("matrix.indicatorBacklog")}
                                value={mirroring ? valueLag(m.backlog.messages) : "—"}
                                detail={
                                    !mirroring
                                        ? t("matrix.backlogPaused")
                                        : m.backlog.messages > 0
                                          ? t("matrix.backlogOldest", {
                                                duration: formatUptime(m.backlog.oldest_seconds),
                                            })
                                          : undefined
                                }
                                tom={mirroring && m.backlog.messages > 0 ? "alert" : undefined}
                            />
                            <Indicador
                                label={t("matrix.indicatorFailures")}
                                value={valueFailures(m.pending_failures)}
                                tom={m.pending_failures > 0 ? "error" : undefined}
                            />
                            <Indicador
                                label={t("matrix.indicatorLastFailure")}
                                value={m.last_failure ?? t("matrix.noRecentFailure")}
                                tom={m.last_failure ? "error" : undefined}
                            />
                        </dl>
                    </>
                ) : null}
            </section>

            <section className="poph-section" aria-labelledby={spacesId}>
                <h2 id={spacesId} className="poph-section__title">{t("matrix.sectionSpaces")}</h2>
                <p className="poph-matrix__help">
                    {t("matrix.spacesHelpBefore")}
                    <code>!</code>
                    {t("matrix.spacesHelpBetween")}
                    <code>#</code>
                    {t("matrix.spacesHelpAfter")}
                </p>

                {spaces.loading && !list ? (
                    <div
                        className="poph-table poph-matrix__table"
                        aria-busy="true"
                        aria-label={t("matrix.loadingSpacesLabel")}
                    >
                        <div className="poph-table__header" role="presentation">
                            <span>{t("matrix.columnDepartment")}</span>
                            <span>{t("matrix.columnSpace")}</span>
                            <span>{t("matrix.columnOrigin")}</span>
                            <span>{t("matrix.columnStatus")}</span>
                            <span className="poph-table__cell--actions">{t("matrix.columnActions")}</span>
                        </div>
                        {[0, 1, 2].map((index) => (
                            <div className="poph-table__line" role="presentation" key={index}>
                                <SofaSkeleton lines={2} />
                                <SofaSkeleton lines={2} />
                                <SofaSkeleton lines={1} />
                                <SofaSkeleton lines={1} />
                                <SofaSkeleton lines={1} />
                            </div>
                        ))}
                    </div>
                ) : spaces.erro && !list ? (
                    <SofaErrorState
                        title={t("matrix.spacesLoadFailed")}
                        description={textNotice(spaces.erro)}
                        retry={{ label: t("operation.retry"), onClick: () => void load() }}
                    />
                ) : list ? (
                    <>
                        {spaces.erro ? (
                            <p className="poph-matrix__notice" role="alert">
                                {t("matrix.lastUpdateFailed", { message: textNotice(spaces.erro) })}
                            </p>
                        ) : null}

                        {departments.erro && !deptos ? (
                            <p className="poph-matrix__notice" role="alert">
                                {t("matrix.departmentsLoadFailed", {
                                    message: textNotice(departments.erro),
                                })}
                            </p>
                        ) : null}

                        {noticeSpace ? (
                            <SofaBanner tone={noticeSpace.tom === "ok" ? "success" : "danger"}>
                                {textNotice(noticeSpace.fonte)}
                            </SofaBanner>
                        ) : null}

                        {departments.loading && !deptos ? (
                            <SofaSkeleton lines={3} />
                        ) : rows.length === 0 ? (
                            <SofaEmptyState
                                title={t("matrix.noActiveDepartment")}
                                description={t("matrix.noActiveDepartmentText")}
                            />
                        ) : (
                            <div
                                className="poph-table poph-matrix__table"
                                role="table"
                                aria-label={t("matrix.spacesTableLabel")}
                            >
                                <div className="poph-table__header" role="row">
                                    <span role="columnheader">{t("matrix.columnDepartment")}</span>
                                    <span role="columnheader">{t("matrix.columnSpace")}</span>
                                    <span role="columnheader">{t("matrix.columnOrigin")}</span>
                                    <span role="columnheader">{t("matrix.columnStatus")}</span>
                                    <span role="columnheader" className="poph-table__cell--actions">
                                        {t("matrix.columnActions")}
                                    </span>
                                </div>
                                {rows.map((row) => {
                                    const { espaco } = row;
                                    return (
                                        <div className="poph-table__line" role="row" key={row.id}>
                                            <span role="cell" className="poph-table__cell poph-matrix__department">
                                                <strong>{row.nome}</strong>
                                                <span className="poph-matrix__small">{row.id}</span>
                                            </span>
                                            <span role="cell" className="poph-table__cell">
                                                {espaco ? (
                                                    <>
                                                        <span className="poph-matrix__alias">{espaco.alias}</span>
                                                        <span className="poph-matrix__small">{espaco.space_room_id}</span>
                                                    </>
                                                ) : (
                                                    <>
                                                        <span className="poph-matrix__empty">
                                                            {t("matrix.noSpaceLinked")}
                                                        </span>
                                                        <span className="poph-matrix__small">
                                                            {t("matrix.noSpaceLinkHint")}
                                                        </span>
                                                    </>
                                                )}
                                            </span>
                                            <span role="cell" className="poph-table__cell">
                                                {espaco ? (
                                                    <span
                                                        className={
                                                            espaco.origin === "auto"
                                                                ? "poph-matrix__origin poph-matrix__origin--auto"
                                                                : "poph-matrix__origin poph-matrix__origin--linked"
                                                        }
                                                    >
                                                        {textOrigin(espaco.origin)}
                                                    </span>
                                                ) : (
                                                    <span className="poph-matrix__origin poph-matrix__origin--none">
                                                        {t("matrix.originNotConfigured")}
                                                    </span>
                                                )}
                                            </span>
                                            <span role="cell" className="poph-table__cell">
                                                {espaco && !row.archived ? (
                                                    <span className="poph-status poph-status--active">
                                                        <span className="poph-status__point" aria-hidden="true" />
                                                        {t("matrix.statusActive")}
                                                    </span>
                                                ) : espaco ? (
                                                    <span className="poph-status poph-status--inactive">
                                                        <span className="poph-status__point" aria-hidden="true" />
                                                        {t("matrix.statusArchivedDepartment")}
                                                    </span>
                                                ) : (
                                                    <span className="poph-status poph-status--inactive">
                                                        <span className="poph-status__point" aria-hidden="true" />
                                                        {t("matrix.statusNoSpace")}
                                                    </span>
                                                )}
                                            </span>
                                            <span role="cell" className="poph-table__cell poph-table__cell--actions">
                                                {espaco ? (
                                                    <SofaButton
                                                        variant="subtle"
                                                        aria-label={t("matrix.disconnectAria", {
                                                            name: row.nome,
                                                        })}
                                                        disabled={turningOffSpace}
                                                        onClick={() => setSpaceToTurnOff(espaco)}
                                                    >
                                                        {t("matrix.disconnect")}
                                                    </SofaButton>
                                                ) : (
                                                    <>
                                                        <SofaButton
                                                            variant="primary"
                                                            size="sm"
                                                            aria-label={t("matrix.createAutoAria", {
                                                                name: row.nome,
                                                            })}
                                                            loading={creating === row.id}
                                                            disabled={creating !== null}
                                                            onClick={() => void createSpace(row)}
                                                        >
                                                            {t("matrix.createAuto")}
                                                        </SofaButton>
                                                        <SofaButton
                                                            variant="secondary"
                                                            size="sm"
                                                            aria-label={t("matrix.linkExistingAria", {
                                                                name: row.nome,
                                                            })}
                                                            disabled={creating !== null}
                                                            onClick={() => openCall(row)}
                                                        >
                                                            {t("matrix.linkExisting")}
                                                        </SofaButton>
                                                    </>
                                                )}
                                            </span>
                                        </div>
                                    );
                                })}
                            </div>
                        )}

                        <SofaBanner tone="info" title={t("matrix.operationRuleTitle")}>
                            {t("matrix.operationRuleText")}
                        </SofaBanner>
                    </>
                ) : null}
            </section>

            <SofaModal
                open={turningOn !== null}
                onOpenChange={(opened) => {
                    if (!opened) closeCall();
                }}
                title={t("matrix.linkModalTitle")}
                description={t("matrix.linkModalDescription")}
                closeLabel={t("matrix.linkModalClose")}
                footer={
                    <>
                        <SofaButton variant="secondary" disabled={sendingCall} onClick={closeCall}>
                            {t("matrix.cancel")}
                        </SofaButton>
                        <SofaButton
                            type="submit"
                            form={callFormId}
                            variant="primary"
                            loading={sendingCall}
                            disabled={sendingCall || typedSpace.trim() === ""}
                        >
                            {t("matrix.linkSpaceAction")}
                        </SofaButton>
                    </>
                }
            >
                <form id={callFormId} className="poph-matrix__form" onSubmit={(evento) => void linkExisting(evento)} noValidate>
                    <div className="poph-matrix__department-chosen">
                        <span className="poph-matrix__small">{t("matrix.selectedDepartment")}</span>
                        <strong>{turningOn ? `${turningOn.nome} (${turningOn.id})` : ""}</strong>
                    </div>
                    {errorDialog ? (
                        <SofaBanner tone="danger">{textNotice(errorDialog)}</SofaBanner>
                    ) : null}
                    <SofaField
                        label={t("matrix.spaceFieldLabel")}
                        hint={t("matrix.spaceFieldHint")}
                        htmlFor={existingSpaceId}
                    >
                        {(control) => (
                            <SofaInput
                                id={control.id}
                                aria-describedby={control.describedBy}
                                invalid={control.invalid}
                                required={control.required}
                                value={typedSpace}
                                autoComplete="off"
                                autoFocus
                                disabled={sendingCall}
                                placeholder={t("matrix.spaceFieldPlaceholder")}
                                onChange={(evento) => setSpaceTyped(evento.target.value)}
                            />
                        )}
                    </SofaField>
                </form>
            </SofaModal>

            <SofaConfirmDialog
                open={confirmingTurnOffModule}
                onOpenChange={(opened) => {
                    if (!opened && !sendingModule) setConfirmingTurnOffModule(false);
                }}
                title={t("matrix.confirmDisableTitle")}
                description={t("matrix.confirmDisableText")}
                confirmLabel={t("matrix.disableModule")}
                cancelLabel={t("matrix.cancel")}
                closeLabel={t("matrix.confirmDisableClose")}
                tone="danger"
                loading={sendingModule}
                onConfirm={() => {
                    setConfirmingTurnOffModule(false);
                    void toggleModule(false);
                }}
            />

            <SofaConfirmDialog
                open={spaceTurnOff !== null}
                onOpenChange={(opened) => {
                    if (!opened && !turningOffSpace) setSpaceToTurnOff(null);
                }}
                title={
                    spaceTurnOff
                        ? t("matrix.confirmDisconnectTitle", {
                              department: nameSpace(spaceTurnOff),
                          })
                        : t("matrix.confirmDisconnectTitleFallback")
                }
                description={t("matrix.confirmDisconnectText")}
                confirmLabel={t("matrix.confirmDisconnectAction")}
                cancelLabel={t("matrix.cancel")}
                closeLabel={t("matrix.confirmDisconnectClose")}
                tone="danger"
                loading={turningOffSpace}
                onConfirm={() => {
                    if (spaceTurnOff) void turnOffSpace(spaceTurnOff);
                }}
            />
        </div>
    );
}
