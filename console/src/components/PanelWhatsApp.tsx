
import { useCallback, useEffect, useRef, useState } from "react";
import {
    IconAlertTriangle,
    IconLink,
    IconLogOut,
    IconMessage,
    IconRefresh,
    SofaBanner,
    SofaButton,
    SofaField,
    SofaInput,
    SofaModal,
    type SofaFieldControlProps,
} from "@sofacoding/sofa-ui";
import { ApiError } from "../api/client";
import {
    disconnectWhatsapp,
    explainWhatsappFailure,
    qrSource,
    generateWhatsappQr,
    readWhatsappState,
    getIntegration,
    connectionLabel,
    type WhatsappState,
    type WhatsappFailure,
    type Integracao,
    type WhatsappQr,
} from "../api/integrations";
import { useI18n } from "../i18n";
import "../styles/whatsapp.css";

export const INTERVAL_QUERY_MS = 4_000;

export const MAXIMUM_QUERIES = 45;

function withoutDescribedBy(control: SofaFieldControlProps) {
    const { describedBy, ...rest } = control;
    return { ...rest, "aria-describedby": describedBy };
}

export const NUMBER_WHATSAPP = /^\d{10,15}$/;

function classConnection(estado: string): string {
    switch (estado) {
        case "connected":
            return "poph-status--active";
        case "awaiting_scan":
            return "poph-status--attention";
        case "disconnected":
            return "poph-status--error";
        default:
            return "poph-status--inactive";
    }
}

export type PanelWhatsAppProps = {
    org: string;
    agente: string;
    integracao: Integracao;
    onUpdateIntegration: (integracao: Integracao) => void;
};

export function PanelWhatsApp({
    org,
    agente,
    integracao,
    onUpdateIntegration,
}: PanelWhatsAppProps) {
    const { translate: t } = useI18n();
    const [qr, setQr] = useState<WhatsappQr | null>(null);
    const [stateLive, setStateLive] = useState<WhatsappState | null>(null);
    const [loadingQr, setLoadingQr] = useState(false);
    const [querying, setQuerying] = useState(false);
    const [disconnecting, setDisconnecting] = useState(false);
    const [confirmingDisconnection, setConfirmingDisconnection] = useState(false);
    const [failure, setFailure] = useState<WhatsappFailure | null>(null);
    const [notice, setNotice] = useState<string | null>(null);
    const [loopActive, setLoopActive] = useState(false);
    const [numero, setNumber] = useState("");
    const [errorNumber, setErrorNumber] = useState<string | null>(null);

    const versao = integracao.version;
    const id = integracao.id;
    const estado = stateLive?.connection_state ?? integracao.connection_state;
    const waitingRead = estado === "awaiting_scan";

    const integrationRef = useRef(integracao);
    integrationRef.current = integracao;

    const rereadAfterConflict = useCallback(async () => {
        try {
            const current = await getIntegration(org, agente, id);
            onUpdateIntegration(current);
            setFailure({
                texto: t("integrations.whatsappConflictText", { version: current.version }),
                code: "version_conflict",
                hint: t("integrations.whatsappConflictTip"),
            });
        } catch {
            setFailure({
                texto: t("integrations.whatsappConflictRereadFailed"),
                code: "version_conflict",
            });
        }
    }, [org, agente, id, onUpdateIntegration, t]);

    async function conectar() {
        const typed = numero.trim();
        if (typed !== "" && !NUMBER_WHATSAPP.test(typed)) {
            setErrorNumber(t("integrations.whatsappPhoneInvalid"));
            return;
        }
        setErrorNumber(null);
        setLoadingQr(true);
        setFailure(null);
        setNotice(null);
        try {
            const response = await generateWhatsappQr(
                org,
                agente,
                id,
                versao,
                typed === "" ? undefined : typed,
            );
            setQr(response);
            setStateLive({
                state: response.state,
                connection_state: response.connection_state,
                instance: response.instance,
            });
            onUpdateIntegration({
                ...integrationRef.current,
                connection_state: response.connection_state,
            });
            if (response.connection_state === "awaiting_scan") {
                setLoopActive(true);
                setNotice(
                    response.pairing_code
                        ? t("integrations.whatsappQrWithCodeNotice")
                        : t("integrations.whatsappQrNotice"),
                );
            } else if (response.state === "open") {
                setLoopActive(false);
                setNotice(t("integrations.whatsappAlreadyConnectedNotice"));
            }
        } catch (error) {
            setLoopActive(false);
            if (error instanceof ApiError && error.status === 412) {
                await rereadAfterConflict();
            } else {
                setFailure(explainWhatsappFailure(error));
            }
        } finally {
            setLoadingQr(false);
        }
    }

    const queryState = useCallback(async () => {
        setQuerying(true);
        setFailure(null);
        try {
            const response = await readWhatsappState(org, agente, id);
            setStateLive(response);
            onUpdateIntegration({
                ...integrationRef.current,
                connection_state: response.connection_state,
            });
            if (response.connection_state !== "awaiting_scan") setQr(null);
            setNotice(
                t("integrations.whatsappLiveStateNotice", {
                    state: connectionLabel(response.connection_state).toLowerCase(),
                }),
            );
            return response;
        } catch (error) {
            setFailure(explainWhatsappFailure(error));
            return null;
        } finally {
            setQuerying(false);
        }
    }, [org, agente, id, onUpdateIntegration, t]);

    useEffect(() => {
        if (!loopActive || !waitingRead) return;
        let live = true;
        let queries = 0;
        const interval = window.setInterval(() => {
            queries += 1;
            if (queries > MAXIMUM_QUERIES) {
                if (!live) return;
                setLoopActive(false);
                setNotice(t("integrations.whatsappPollingStoppedNotice"));
                return;
            }
            void readWhatsappState(org, agente, id)
                .then((response) => {
                    if (!live) return;
                    setStateLive(response);
                    onUpdateIntegration({
                        ...integrationRef.current,
                        connection_state: response.connection_state,
                    });
                    if (response.connection_state !== "awaiting_scan") {
                        setLoopActive(false);
                        setQr(null);
                        setNotice(
                            response.connection_state === "connected"
                                ? t("integrations.whatsappConnectedNotice")
                                : t("integrations.whatsappLiveStateNotice", {
                                      state: connectionLabel(response.connection_state).toLowerCase(),
                                  }),
                        );
                    }
                })
                .catch((error) => {
                    if (!live) return;
                    setLoopActive(false);
                    setFailure(explainWhatsappFailure(error));
                });
        }, INTERVAL_QUERY_MS);
        return () => {
            live = false;
            window.clearInterval(interval);
        };
    }, [loopActive, waitingRead, org, agente, id, onUpdateIntegration, t]);

    async function desconectar() {
        setConfirmingDisconnection(false);
        setDisconnecting(true);
        setFailure(null);
        setNotice(null);
        try {
            const response = await disconnectWhatsapp(org, agente, id, versao);
            setLoopActive(false);
            setQr(null);
            setStateLive({
                state: "close",
                connection_state: response.connection_state,
                instance: response.instance,
            });
            onUpdateIntegration({
                ...integrationRef.current,
                connection_state: response.connection_state,
            });
            setNotice(t("integrations.whatsappDisconnectedNotice"));
        } catch (error) {
            if (error instanceof ApiError && error.status === 412) {
                await rereadAfterConflict();
            } else {
                setFailure(explainWhatsappFailure(error));
            }
        } finally {
            setDisconnecting(false);
        }
    }

    const base64 = qr?.qrcode?.base64 ?? null;
    const round = qr?.qrcode?.count ?? null;

    return (
        <div className="poph-whatsapp">
            <div className="poph-whatsapp__state">
                <span className="poph-mono-muted">{id}</span>
                <span className={`poph-status ${classConnection(estado)}`}>
                    <span className="poph-status__point" aria-hidden="true" />
                    {connectionLabel(estado)}
                </span>
                <span className="poph-mono-muted">
                    {integracao.tem_segredo
                        ? t("integrations.whatsappSecretSaved")
                        : t("integrations.whatsappNoSecret")}
                </span>
                {qr?.instance && (
                    <span className="poph-mono-muted">
                        {t("integrations.whatsappInstanceLabel", { instance: qr.instance })}
                    </span>
                )}
            </div>

            <SofaField
                label={t("integrations.whatsappPhoneLabel")}
                hint={t("integrations.whatsappPhoneHint")}
                error={errorNumber ?? undefined}
                htmlFor="poph-whatsapp-numero"
            >
                {(control) => (
                    <SofaInput
                        {...withoutDescribedBy(control)}
                        value={numero}
                        inputMode="numeric"
                        autoComplete="off"
                        placeholder="5511999998888"
                        onChange={(evento) => setNumber(evento.target.value)}
                    />
                )}
            </SofaField>

            <div className="poph-whatsapp__actions">
                <SofaButton
                    variant="primary"
                    size="sm"
                    icon={<IconMessage aria-hidden="true" />}
                    onClick={() => void conectar()}
                    loading={loadingQr}
                >
                    {qr ? t("integrations.whatsappRegenerateQr") : t("integrations.whatsappGenerateQr")}
                </SofaButton>
                <SofaButton
                    variant="subtle"
                    size="sm"
                    icon={<IconRefresh aria-hidden="true" />}
                    onClick={() => {
                        void queryState();
                    }}
                    loading={querying}
                >
                    {t("integrations.whatsappCheckState")}
                </SofaButton>
                <SofaButton
                    variant="subtle"
                    size="sm"
                    icon={<IconLogOut aria-hidden="true" />}
                    onClick={() => {
                        setNotice(null);
                        setConfirmingDisconnection(true);
                    }}
                    loading={disconnecting}
                    disabled={estado === "disconnected"}
                >
                    {t("integrations.whatsappDisconnectDevice")}
                </SofaButton>
                {loopActive && waitingRead && (
                    <SofaButton variant="subtle" size="sm" onClick={() => setLoopActive(false)}>
                        {t("integrations.whatsappStopPolling")}
                    </SofaButton>
                )}
            </div>

            {notice && (
                <p className="poph-mono-muted" style={{ margin: 0 }} role="status">
                    {notice}
                </p>
            )}

            {failure && (
                <SofaBanner tone="danger" title={t("integrations.whatsappOperationErrorTitle")}>
                    <span>{failure.texto}</span>
                    {failure.hint && <div className="poph-whatsapp__hint">{failure.hint}</div>}
                    {failure.code && (
                        <div className="poph-mono-muted poph-whatsapp__hint">
                            {t("integrations.whatsappErrorCode", { code: failure.code })}
                        </div>
                    )}
                </SofaBanner>
            )}

            {base64 ? (
                <div className="poph-whatsapp__qr-block">
                    <img
                        className="poph-whatsapp__qr-image"
                        src={qrSource(base64)}
                        alt={t("integrations.whatsappQrAlt")}
                        width={220}
                        height={220}
                    />
                    <div className="poph-whatsapp__qr-text">
                        <strong>{t("integrations.whatsappQrHeading")}</strong>
                        <span className="poph-mono-muted">
                            {t("integrations.whatsappQrSteps")}
                        </span>
                        {round !== null && (
                            <span className="poph-mono-muted">
                                {t("integrations.whatsappQrRound", { round })}
                            </span>
                        )}
                        {qr?.pairing_code && (
                            <span>
                                {t("integrations.whatsappPairingCode")}{" "}
                                <code className="poph-mono-muted">{qr.pairing_code}</code>
                            </span>
                        )}
                        <span className="poph-mono-muted">
                            {t("integrations.whatsappQrRotationHint")}
                        </span>
                    </div>
                </div>
            ) : qr?.state === "open" || estado === "connected" ? (
                <SofaBanner tone="success" title={t("integrations.whatsappConnectedBannerTitle")}>
                    {t("integrations.whatsappConnectedBannerBody")}
                </SofaBanner>
            ) : (
                <p className="poph-mono-muted" style={{ margin: 0 }}>
                    {t("integrations.whatsappNoQrHint")}
                </p>
            )}

            {waitingRead && loopActive && (
                <p className="poph-mono-muted" style={{ margin: 0 }}>
                    {t("integrations.whatsappPollingHint", {
                        seconds: INTERVAL_QUERY_MS / 1000,
                        minutes: Math.round((INTERVAL_QUERY_MS * MAXIMUM_QUERIES) / 60_000),
                    })}
                </p>
            )}

            <SofaModal
                open={confirmingDisconnection}
                onOpenChange={(fresh) => {
                    if (!fresh && !disconnecting) setConfirmingDisconnection(false);
                }}
                title={t("integrations.whatsappDisconnectModalTitle")}
                description={t("integrations.whatsappDisconnectModalDescription")}
                closeLabel={t("integrations.whatsappDisconnectModalClose")}
                size="sm"
                id="poph-confirma-desconexao-whatsapp"
                footer={
                    <>
                        <SofaButton
                            variant="secondary"
                            disabled={disconnecting}
                            onClick={() => setConfirmingDisconnection(false)}
                        >
                            {t("integrations.whatsappDisconnectCancel")}
                        </SofaButton>
                        <SofaButton
                            variant="danger"
                            icon={<IconLogOut aria-hidden="true" />}
                            loading={disconnecting}
                            onClick={() => void desconectar()}
                        >
                            {t("integrations.whatsappDisconnectConfirm")}
                        </SofaButton>
                    </>
                }
            >
                <div className="poph-whatsapp__confirmation">
                    <div className="poph-whatsapp__confirmation-line">
                        <IconAlertTriangle aria-hidden="true" />
                        <span>
                            {t("integrations.whatsappDisconnectBodyPrefix")}
                            <strong>{t("integrations.whatsappDisconnectBodyStrong")}</strong>
                            {t("integrations.whatsappDisconnectBodySuffix")}
                        </span>
                    </div>
                    <p className="poph-mono-muted" style={{ margin: 0 }}>
                        <IconLink aria-hidden="true" />{" "}
                        {t("integrations.whatsappDisconnectIntegrationId", { id })}
                    </p>
                </div>
            </SofaModal>
        </div>
    );
}
