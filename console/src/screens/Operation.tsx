
import { useCallback, useEffect, useId, useRef, useState, type ReactNode } from "react";
import { useParams } from "react-router-dom";
import {
    IconRefresh,
    SofaBanner,
    SofaButton,
    SofaConfirmDialog,
    SofaErrorState,
    SofaSkeleton,
} from "@sofacoding/sofa-ui";
import { ApiError } from "../api/client";
import { translate, useI18n } from "../i18n";
import { formatNumber } from "../i18n/format";
import type { TranslationKey } from "../i18n/catalog";
import {
    fetchMetrics,
    fetchPlugin,
    fetchHealth,
    listPlugins,
    reloadCluster,
    manifestSummary,
    type PluginItem,
    type Measurements,
    type ReloadReport,
    type ClusterMapSummary,
    type InstanceHealth,
} from "../api/operation";
import type { PluginManifest } from "../api/integrations";
import { ExportImportOrganization } from "../components/ExportImportOrganization";
import "../styles/operation.css";

export const REFRESH_INTERVAL_MS = 15_000;

export function formatUptime(seconds: number): string {
    const total = Math.max(0, Math.floor(seconds));
    const dias = Math.floor(total / 86_400);
    const hours = Math.floor((total % 86_400) / 3_600);
    const minutes = Math.floor((total % 3_600) / 60);
    if (dias > 0) {
        return translate(dias === 1 ? "operation.uptimeDaysOne" : "operation.uptimeDaysMany", {
            days: dias,
            hours,
        });
    }
    if (hours > 0) return translate("operation.uptimeHours", { hours, minutes });
    if (minutes > 0) return translate("operation.uptimeMinutes", { minutes });
    return translate("operation.uptimeLessThanMinute");
}

export function formatMemory(bytes: number): string {
    const kb = 1024;
    const mb = kb * 1024;
    const gb = mb * 1024;
    const formatar = (value: number, unit: string) =>
        `${formatNumber(value, { maximumFractionDigits: 1 })} ${unit}`;
    if (bytes >= gb) return formatar(bytes / gb, "GB");
    if (bytes >= mb) return formatar(bytes / mb, "MB");
    return formatar(bytes / kb, "KB");
}

export function formatDatabaseVersion(versao: number): string {
    return versao > 0
        ? translate("operation.schemaVersion", { version: versao })
        : translate("operation.schemaVersionMissing");
}

function numero(value: number | undefined): string {
    return value === undefined ? translate("operation.notInformed") : formatNumber(value);
}

function memory(value: number | undefined): string {
    return value === undefined ? translate("operation.notInformed") : formatMemory(value);
}

export function messageFailure(error: unknown, what: string): string {
    if (error instanceof ApiError) {
        if (error.noServer) return translate("operation.errorOffline");
        if (error.status === 401) return translate("operation.errorSession");
        if (error.status === 403) return translate("operation.errorForbidden");
        if (error.status === 404) return translate("operation.errorNotFound", { what });
        if (error.status >= 500) {
            return translate("operation.errorServer", { status: error.status });
        }
        return error.message;
    }
    return translate("operation.errorGeneric");
}

type FailureSection = { error: unknown; what: TranslationKey };

function textFailure(failure: FailureSection): string {
    return messageFailure(failure.error, translate(failure.what));
}

type Estado<T> = { loading: boolean; dados: T | null; error: FailureSection | null };

const stateInitial = <T,>(): Estado<T> => ({ loading: true, dados: null, error: null });

type Query = {
    loading: boolean;
    kind: string;
    manifesto: PluginManifest | null;
    error: unknown;
};

function messagePlugin(error: unknown): string {
    if (error instanceof ApiError && error.status === 404) return translate("operation.pluginNotDeclared");
    if (error instanceof ApiError && error.status === 503) return translate("operation.pluginNotResponding");
    return messageFailure(error, translate("operation.whatPlugin"));
}

function Property({ chave, value, mono }: { chave: string; value: string; mono?: boolean }) {
    return (
        <div className="poph-property">
            <dt className="poph-property__key">{chave}</dt>
            <dd className={mono ? "poph-property__value poph-property__value--mono" : "poph-property__value"}>
                {value}
            </dd>
        </div>
    );
}

function RowPlugin({
    item,
    opened,
    onOpen,
}: {
    item: PluginItem;
    opened: boolean;
    onOpen: () => void;
}) {
    const { translate: t } = useI18n();
    if (item.situacao === "ok") {
        return (
            <li className="poph-operation__plugin">
                <div className="poph-operation__plugin-text">
                    <span className="poph-operation__seal poph-operation__seal--ok">
                        {t("operation.pluginOnline")}
                    </span>
                    <span className="poph-operation__plugin-name">{item.nome ?? item.kind}</span>
                    <span className="poph-operation__plugin-meta">
                        {t("operation.pluginKindPrefix")} <code>{item.kind}</code>
                        {item.versao ? t("operation.pluginVersionSuffix", { version: item.versao }) : ""}
                    </span>
                </div>
                <SofaButton
                    variant="subtle"
                    aria-label={t("operation.pluginDetailsAria", { kind: item.kind })}
                    aria-pressed={opened}
                    onClick={onOpen}
                >
                    {t("operation.pluginDetails")}
                </SofaButton>
            </li>
        );
    }
    return (
        <li className="poph-operation__plugin poph-operation__plugin--unavailable">
            <div className="poph-operation__plugin-text">
                <span className="poph-operation__seal poph-operation__seal--unavailable">
                    {t("operation.pluginUnavailable")}
                </span>
                <span className="poph-operation__plugin-name">
                    {t("operation.pluginKindPrefix")} <code>{item.kind}</code>
                </span>
                <span className="poph-operation__plugin-meta">{(item.mensagem ?? "").replace(/`/g, "")}</span>
            </div>
        </li>
    );
}

const SIZE_PRINT = 12;

export function printScreen(snapshot: string): string {
    if (!snapshot) return translate("operation.fingerprintMissing");
    return snapshot.length > SIZE_PRINT
        ? `${snapshot.slice(0, SIZE_PRINT)}…`
        : snapshot;
}

export type FailureReload =
    | { tipo: "invalido"; file: string | null; problems: string[] }
    | { tipo: "sem_permissao" }
    | { tipo: "sem_resposta" }
    | { tipo: "outra"; error: unknown };

export function failureReload(error: unknown): FailureReload {
    if (error instanceof ApiError) {
        if (error.status === 400) {
            const raw: unknown = error.details;
            const details =
                raw !== null && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
            const problems = Array.isArray(details.problemas)
                ? details.problemas.filter((item): item is string => typeof item === "string")
                : [];
            return {
                tipo: "invalido",
                file: typeof details.arquivo === "string" ? details.arquivo : null,
                problems: problems.length > 0 ? problems : [error.message],
            };
        }
        if (error.status === 403) return { tipo: "sem_permissao" };
        if (error.noServer) return { tipo: "sem_resposta" };
    }
    return { tipo: "outra", error: error };
}

function listOrNone(items: string[]): string {
    return items.length > 0 ? items.join(", ") : translate("operation.listNone");
}

function modeText(summary: ClusterMapSummary): string {
    return summary.mode === "cluster"
        ? translate("operation.mapModeCluster")
        : translate("operation.mapModeSingle");
}

function TableMap({ before, after, printBefore, printAfter }: {
    before: ClusterMapSummary;
    after: ClusterMapSummary;
    printBefore: string;
    printAfter: string;
}) {
    const { translate: t } = useI18n();
    const rows: { label: string; before: ReactNode; after: ReactNode }[] = [
        { label: t("operation.mapMode"), before: modeText(before), after: modeText(after) },
        {
            label: t("operation.mapThisInstance"),
            before: before.localInstance || t("operation.mapNotDefined"),
            after: after.localInstance || t("operation.mapNotDefined"),
        },
        {
            label: t("operation.mapDefaultInstance"),
            before: before.instanceDefault ?? t("operation.mapNone"),
            after: after.instanceDefault ?? t("operation.mapNone"),
        },
        {
            label: t("operation.mapDeclaredInstances"),
            before: listOrNone(before.instances.map((item) => item.id)),
            after: listOrNone(after.instances.map((item) => item.id)),
        },
        {
            label: t("operation.mapRoutedOrgs"),
            before: formatNumber(before.quantityOrgs),
            after: formatNumber(after.quantityOrgs),
        },
        {
            label: t("operation.mapSharedSecret"),
            before: before.secretConfigured ? t("operation.mapConfigured") : t("operation.mapNotConfigured"),
            after: after.secretConfigured
                ? t("operation.mapConfigured")
                : t("operation.mapNotConfigured"),
        },
        {
            label: t("operation.mapFingerprint"),
            before: <code title={printBefore}>{printScreen(printBefore)}</code>,
            after: <code title={printAfter}>{printScreen(printAfter)}</code>,
        },
    ];
    return (
        <table className="poph-operation__table" aria-label={t("operation.mapTableLabel")}>
            <thead>
                <tr>
                    <th scope="col">{t("operation.mapColumnItem")}</th>
                    <th scope="col">{t("operation.mapColumnBefore")}</th>
                    <th scope="col">{t("operation.mapColumnAfter")}</th>
                </tr>
            </thead>
            <tbody>
                {rows.map((row) => (
                    <tr key={row.label}>
                        <th scope="row">{row.label}</th>
                        <td>{row.before}</td>
                        <td>{row.after}</td>
                    </tr>
                ))}
            </tbody>
        </table>
    );
}

function ReportReload({ report, title }: { report: ReloadReport; title: string }) {
    const { translate: t } = useI18n();
    return (
        <div className="poph-operation__report">
            <h3 className="poph-operation__subtitle">{title}</h3>
            <SofaBanner
                tone={report.changed ? "success" : "info"}
                title={
                    report.changed
                        ? t("operation.clusterChangedTitle")
                        : t("operation.clusterUnchangedTitle")
                }
            >
                {report.changed
                    ? t("operation.clusterChangedText")
                    : t("operation.clusterUnchangedText")}
            </SofaBanner>
            {report.after.mode === "instancia_unica" ? (
                <p className="poph-operation__help">{t("operation.clusterSingleInstance")}</p>
            ) : null}
            <TableMap
                before={report.before}
                after={report.after}
                printBefore={report.printBefore}
                printAfter={report.printAfter}
            />
            <p className="poph-operation__help">
                {t("operation.clusterInstancesAdded", {
                    list: listOrNone(report.instancesAdded),
                })}
            </p>
            <p className="poph-operation__help">
                {t("operation.clusterInstancesRemoved", {
                    list: listOrNone(report.instancesRemoved),
                })}
            </p>
            {report.orgsChanged.length > 0 ? (
                <>
                    <p className="poph-operation__help">{t("operation.clusterOrgsChanged")}</p>
                    <ul
                        className="poph-operation__conflicts"
                        aria-label={t("operation.clusterOrgsChangedLabel")}
                    >
                        {report.orgsChanged.map((troca) => (
                            <li key={troca.org}>
                                {t("operation.clusterOrgChange", {
                                    org: troca.org,
                                    from: troca.de ?? t("operation.clusterNoDestination"),
                                    to: troca.para ?? t("operation.clusterNoDestination"),
                                })}
                            </li>
                        ))}
                    </ul>
                </>
            ) : (
                <p className="poph-operation__help">{t("operation.clusterNoOrgChanged")}</p>
            )}
        </div>
    );
}

function NoticeReload({ failure, onTryNew }: { failure: FailureReload; onTryNew: () => void }) {
    const { translate: t } = useI18n();
    if (failure.tipo === "invalido") {
        return (
            <SofaBanner tone="danger" title={t("operation.clusterInvalidTitle")}>
                <p style={{ margin: 0 }}>{t("operation.clusterInvalidText")}</p>
                {failure.file ? (
                    <p className="poph-mono-muted" style={{ margin: "6px 0 0" }}>
                        {t("operation.clusterFile", { file: failure.file })}
                    </p>
                ) : null}
                <p style={{ margin: "6px 0 0" }}>{t("operation.clusterProblems")}</p>
                <ul
                    className="poph-operation__conflicts"
                    aria-label={t("operation.clusterProblemsLabel")}
                >
                    {failure.problems.map((problem, index) => (
                        <li key={`${index}-${problem}`}>{problem}</li>
                    ))}
                </ul>
            </SofaBanner>
        );
    }
    if (failure.tipo === "sem_permissao") {
        return (
            <SofaBanner tone="danger" title={t("operation.clusterForbiddenTitle")}>
                <p style={{ margin: 0 }}>{t("operation.clusterForbiddenText")}</p>
            </SofaBanner>
        );
    }
    const message =
        failure.tipo === "sem_resposta"
            ? t("operation.clusterNetworkUnknown")
            : messageFailure(failure.error, t("operation.whatClusterReload"));
    return (
        <SofaBanner tone="danger" title={t("operation.clusterFailedTitle")}>
            <p style={{ margin: 0 }}>{message}</p>
            <div style={{ marginTop: 8 }}>
                <SofaButton variant="secondary" onClick={onTryNew}>
                    {t("operation.retry")}
                </SofaButton>
            </div>
        </SofaBanner>
    );
}

function ClusterSection() {
    const { translate: t } = useI18n();
    const sectionId = useId();
    const [confirming, setConfirming] = useState(false);
    const [reloading, setReloading] = useState(false);
    const [lastSuccess, setLastSuccess] = useState<ReloadReport | null>(null);
    const [failure, setFailure] = useState<FailureReload | null>(null);
    const inProgress = useRef(false);
    const alive = useRef(true);

    useEffect(() => {
        alive.current = true;
        return () => {
            alive.current = false;
        };
    }, []);

    const reload = useCallback(async () => {
        if (inProgress.current) return;
        inProgress.current = true;
        setConfirming(false);
        setReloading(true);
        setFailure(null);
        try {
            const report = await reloadCluster();
            if (!alive.current) return;
            setLastSuccess(report);
        } catch (error) {
            if (!alive.current) return;
            setFailure(failureReload(error));
        } finally {
            inProgress.current = false;
            if (alive.current) setReloading(false);
        }
    }, []);

    return (
        <section className="poph-section" aria-labelledby={sectionId}>
            <h2 id={sectionId} className="poph-section__title">{t("operation.sectionCluster")}</h2>
            <p className="poph-operation__help">{t("operation.clusterHelp")}</p>

            <div className="poph-stack" style={{ gap: "var(--pop-spacing-sm, 6px)", maxWidth: 680 }}>
                <div>
                    <SofaButton
                        variant="secondary"
                        loading={reloading}
                        disabled={reloading}
                        onClick={() => setConfirming(true)}
                    >
                        {t("operation.clusterReload")}
                    </SofaButton>
                </div>

                <div aria-live="polite">
                    {reloading ? (
                        <p className="poph-operation__loading" role="status">
                            {t("operation.clusterReloading")}
                        </p>
                    ) : null}
                    {failure ? (
                        <NoticeReload failure={failure} onTryNew={() => setConfirming(true)} />
                    ) : null}
                    {lastSuccess ? (
                        <ReportReload
                            report={lastSuccess}
                            title={
                                failure
                                    ? t("operation.clusterLastSuccessTitle")
                                    : t("operation.clusterResultTitle")
                            }
                        />
                    ) : !failure && !reloading ? (
                        <p className="poph-operation__empty">{t("operation.clusterEmpty")}</p>
                    ) : null}
                </div>
            </div>

            <SofaConfirmDialog
                open={confirming}
                onOpenChange={(opened) => {
                    if (!opened && !reloading) setConfirming(false);
                }}
                title={t("operation.clusterConfirmTitle")}
                description={t("operation.clusterConfirmDescription")}
                confirmLabel={t("operation.clusterConfirmAction")}
                cancelLabel={t("common.cancel")}
                closeLabel={t("operation.clusterConfirmClose")}
                onConfirm={() => void reload()}
                loading={reloading}
            />
        </section>
    );
}

export function Operation() {
    const { orgId = "" } = useParams();
    const { translate: t, formatTime } = useI18n();
    const [health, setHealth] = useState<Estado<InstanceHealth>>(stateInitial);
    const [metrics, setMetrics] = useState<Estado<Measurements>>(stateInitial);
    const [plugins, setPlugins] = useState<Estado<PluginItem[]>>(stateInitial);
    const [updated, setUpdatedAt] = useState<Date | null>(null);
    const [query, setQuery] = useState<Query>({
        loading: false,
        kind: "",
        manifesto: null,
        error: null,
    });
    const inProgress = useRef(false);
    const alive = useRef(true);
    const healthId = useId();
    const metricsId = useId();
    const idPlugin = useId();

    useEffect(() => {
        alive.current = true;
        return () => {
            alive.current = false;
        };
    }, []);

    const load = useCallback(async () => {
        if (inProgress.current) return;
        inProgress.current = true;
        setHealth((current) => ({ ...current, loading: true }));
        setMetrics((current) => ({ ...current, loading: true }));
        setPlugins((current) => ({ ...current, loading: true }));
        const [resHealth, resMetrics, resPlugins] = await Promise.allSettled([
            fetchHealth(),
            fetchMetrics(),
            listPlugins(orgId),
        ]);
        inProgress.current = false;
        if (!alive.current) return;

        if (resHealth.status === "fulfilled") {
            setHealth({ loading: false, dados: resHealth.value, error: null });
        } else {
            setHealth((current) => ({
                loading: false,
                dados: current.dados,
                error: { error: resHealth.reason, what: "operation.whatServerState" },
            }));
        }

        if (resMetrics.status === "fulfilled") {
            setMetrics({ loading: false, dados: resMetrics.value, error: null });
        } else {
            setMetrics((current) => ({
                loading: false,
                dados: current.dados,
                error: { error: resMetrics.reason, what: "operation.whatMetrics" },
            }));
        }

        if (resPlugins.status === "fulfilled") {
            setPlugins({ loading: false, dados: resPlugins.value, error: null });
        } else {
            setPlugins((current) => ({
                loading: false,
                dados: current.dados,
                error: { error: resPlugins.reason, what: "operation.whatPluginList" },
            }));
        }
        setUpdatedAt(new Date());
    }, [orgId]);

    useEffect(() => {
        void load();
        const id = window.setInterval(() => {
            if (document.visibilityState === "visible") void load();
        }, REFRESH_INTERVAL_MS);
        return () => window.clearInterval(id);
    }, [load]);

    const queryType = useCallback(
        async (kind: string) => {
            setQuery({ loading: true, kind, manifesto: null, error: null });
            try {
                const manifesto = await fetchPlugin(orgId, kind);
                if (!alive.current) return;
                setQuery({ loading: false, kind, manifesto, error: null });
            } catch (error) {
                if (!alive.current) return;
                setQuery({ loading: false, kind, manifesto: null, error: error });
            }
        },
        [orgId],
    );

    const m = metrics.dados;
    const s = health.dados;
    const list = plugins.dados;

    return (
        <div className="poph-page poph-operation">
            <div className="poph-page__top">
                <div>
                    <h1 className="poph-page__title">{t("operation.pageTitle")}</h1>
                    <p className="poph-page__description">{t("operation.pageDescription")}</p>
                </div>
                <div className="poph-page__actions">
                    <span className="poph-operation__update" role="status" aria-live="polite">
                        {health.loading || metrics.loading || plugins.loading
                            ? t("operation.refreshing")
                            : updated
                              ? t("operation.updatedAt", { time: formatTime(updated) })
                              : ""}
                    </span>
                    <SofaButton
                        variant="subtle"
                        icon={<IconRefresh aria-hidden="true" />}
                        onClick={() => void load()}
                        disabled={health.loading || metrics.loading || plugins.loading}
                    >
                        {t("operation.refresh")}
                    </SofaButton>
                </div>
            </div>

            <section className="poph-section" aria-labelledby={healthId}>
                <h2 id={healthId} className="poph-section__title">{t("operation.sectionServer")}</h2>
                {health.loading && !s ? (
                    <SofaSkeleton lines={3} />
                ) : health.error && !s ? (
                    <SofaErrorState
                        title={t("operation.serverLoadFailed")}
                        description={textFailure(health.error)}
                        retry={{ label: t("operation.retry"), onClick: () => void load() }}
                    />
                ) : s ? (
                    <>
                        {health.error ? (
                            <p className="poph-operation__notice" role="alert">
                                {t("operation.lastUpdateFailed", { message: textFailure(health.error) })}
                            </p>
                        ) : null}
                        <dl className="poph-properties">
                            <Property
                                chave={t("operation.propertyStatus")}
                                value={
                                    s.situacao === "ok"
                                        ? t("operation.statusOnline")
                                        : t("operation.statusDegraded")
                                }
                            />
                            <Property
                                chave={t("operation.propertyServerVersion")}
                                value={s.health.version}
                                mono
                            />
                            <Property
                                chave={t("operation.propertyUptime")}
                                value={formatUptime(s.health.uptime_seconds)}
                            />
                            <Property
                                chave={t("operation.propertyChatDatabase")}
                                value={formatDatabaseVersion(s.health.databases.harness)}
                            />
                            <Property
                                chave={t("operation.propertySearchDatabase")}
                                value={formatDatabaseVersion(s.health.databases.rag)}
                            />
                        </dl>
                    </>
                ) : null}
            </section>

            <section className="poph-section" aria-labelledby={metricsId}>
                <h2 id={metricsId} className="poph-section__title">{t("operation.sectionMetrics")}</h2>
                {metrics.loading && !m ? (
                    <SofaSkeleton lines={4} />
                ) : metrics.error && !m ? (
                    <SofaErrorState
                        title={t("operation.metricsLoadFailed")}
                        description={textFailure(metrics.error)}
                        retry={{ label: t("operation.retry"), onClick: () => void load() }}
                    />
                ) : m ? (
                    <>
                        {metrics.error ? (
                            <p className="poph-operation__notice" role="alert">
                                {t("operation.lastUpdateFailed", { message: textFailure(metrics.error) })}
                            </p>
                        ) : null}
                        <h3 className="poph-operation__subtitle">{t("operation.subtitleNow")}</h3>
                        <dl className="poph-properties">
                            <Property
                                chave={t("operation.metricQueueTurns")}
                                value={numero(m.queueTurns)}
                            />
                            <Property
                                chave={t("operation.metricOutbox")}
                                value={numero(m.queueOutput)}
                            />
                            <Property
                                chave={t("operation.metricActiveConversations")}
                                value={numero(m.activeConversations)}
                            />
                            <Property
                                chave={t("operation.metricLoadedAgents")}
                                value={numero(m.agentsLoaded)}
                            />
                            <Property
                                chave={t("operation.metricProcessMemory")}
                                value={memory(m.processMemory)}
                            />
                            <Property
                                chave={t("operation.metricChatDatabaseSize")}
                                value={memory(m.mainDatabase)}
                            />
                            <Property
                                chave={t("operation.metricSearchDatabaseSize")}
                                value={memory(m.databaseSearch)}
                            />
                        </dl>
                        <h3 className="poph-operation__subtitle">{t("operation.subtitleSinceBoot")}</h3>
                        <dl className="poph-properties">
                            <Property
                                chave={t("operation.metricLlmCalls")}
                                value={t("operation.valueWithErrors", {
                                    value: numero(m.callsLlm),
                                    errors: numero(m.errorsLlm),
                                })}
                            />
                            <Property
                                chave={t("operation.metricJevCalls")}
                                value={t("operation.valueWithErrors", {
                                    value: numero(m.callsJev),
                                    errors: numero(m.errorsJev),
                                })}
                            />
                            <Property
                                chave={t("operation.metricEvolutionCalls")}
                                value={t("operation.valueWithErrors", {
                                    value: numero(m.callsEvolution),
                                    errors: numero(m.errorsEvolution),
                                })}
                            />
                            <Property
                                chave={t("operation.metricTokens")}
                                value={numero(m.tokens)}
                            />
                        </dl>
                    </>
                ) : null}
            </section>

            <section className="poph-section" aria-labelledby={idPlugin}>
                <h2 id={idPlugin} className="poph-section__title">{t("operation.sectionPlugins")}</h2>
                <p className="poph-operation__help">{t("operation.pluginsHelp")}</p>

                {plugins.loading && !list ? (
                    <>
                        <p className="poph-operation__loading" role="status">
                            {t("operation.pluginsLoading")}
                        </p>
                        <SofaSkeleton lines={3} />
                    </>
                ) : plugins.error && !list ? (
                    <SofaErrorState
                        title={t("operation.pluginsLoadFailed")}
                        description={textFailure(plugins.error)}
                        retry={{ label: t("operation.retry"), onClick: () => void load() }}
                    />
                ) : list ? (
                    <>
                        {plugins.error ? (
                            <p className="poph-operation__notice" role="alert">
                                {t("operation.lastUpdateFailed", { message: textFailure(plugins.error) })}
                            </p>
                        ) : null}
                        {list.length === 0 ? (
                            <p className="poph-operation__empty">
                                {t("operation.pluginsEmptyBefore")}
                                <code> [plugins].integracoes</code>
                                {t("operation.pluginsEmptyAfter")}
                            </p>
                        ) : (
                            <ul
                                className="poph-operation__list-plugins"
                                aria-label={t("operation.pluginsListLabel")}
                            >
                                {list.map((item) => (
                                    <RowPlugin
                                        key={item.kind}
                                        item={item}
                                        opened={query.kind === item.kind && !!query.manifesto}
                                        onOpen={() => void queryType(item.kind)}
                                    />
                                ))}
                            </ul>
                        )}
                    </>
                ) : null}

                <div aria-live="polite">
                    {query.loading ? (
                        <>
                            <p className="poph-operation__loading" role="status">
                                {t("operation.pluginConsulting", { kind: query.kind })}
                            </p>
                            <SofaSkeleton lines={3} />
                        </>
                    ) : query.error ? (
                        <SofaErrorState
                            title={t("operation.pluginConsultFailed", { kind: query.kind })}
                            description={messagePlugin(query.error)}
                            retry={{
                                label: t("operation.retry"),
                                onClick: () => void queryType(query.kind),
                            }}
                        />
                    ) : query.manifesto ? (
                        <dl
                            className="poph-properties poph-operation__detail"
                            aria-label={t("operation.pluginDetailsLabel", { kind: query.kind })}
                        >
                            <Property chave={t("operation.propertyName")} value={query.manifesto.nome} />
                            <Property chave={t("operation.propertyKind")} value={query.manifesto.kind} mono />
                            <Property
                                chave={t("operation.propertyVersion")}
                                value={query.manifesto.versao}
                                mono
                            />
                            <Property
                                chave={t("operation.propertyConfigFields")}
                                value={formatNumber(manifestSummary(query.manifesto).configuracao)}
                            />
                            <Property
                                chave={t("operation.propertySecretFields")}
                                value={formatNumber(manifestSummary(query.manifesto).segredos)}
                            />
                        </dl>
                    ) : null}
                </div>
            </section>

            <ClusterSection />

            <ExportImportOrganization key={orgId} org={orgId} />
        </div>
    );
}
