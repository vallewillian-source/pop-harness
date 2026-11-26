/**
 * Painel do WhatsApp na aba Integrações (HR-44d): pareamento com QR, estado ao vivo e desconexão.
 *
 * Fala com as três rotas de admin da HR-44b (`api/integracoes.ts`):
 * - `POST …/whatsapp/qrcode` com `If-Match` → QR (data URL), rodada, código de pareamento;
 * - `GET …/whatsapp/state` → estado ao vivo, gravado em `connection_state` pelo servidor;
 * - `POST …/whatsapp/disconnect` com `If-Match` → o aparelho sai e a integração continua cadastrada.
 *
 * Regras que a tela respeita:
 * - **Sem contagem regressiva**: a Evolution não informa validade do QR (`expira_em` é sempre
 *   `null`); o código gira sozinho a cada ~20 s e o botão “Gerar outro QR” resolve.
 * - **Estado ao vivo em laço curto** só enquanto o aparelho aguarda leitura: uma consulta a cada
 *   {@link INTERVALO_DE_CONSULTA_MS}, no máximo {@link MAXIMO_DE_CONSULTAS} vezes (~3 min), parando
 *   ao conectar, ao dar erro, ao sair da tela ou no botão “Parar”.
 * - **Segredo nunca é exibido** e o `base64` do QR não vai para o console do navegador.
 * - `412` relê a integração, adota a versão atual e mantém o painel usável (mesma mensagem das
 *   outras abas); o `409 whatsapp_nao_configurado` aparece com a explicação, sem quebrar a aba.
 *
 * Re-layout (RL-12): só a moldura mudou — o painel deixou de ser cartão e virou seção plana (a
 * seção fica na aba Integrações). Nenhuma consulta, intervalo ou estado foi alterado.
 */

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
    type QrDoWhatsapp,
} from "../api/integrations";
import { useI18n } from "../i18n";
import "../estilos/whatsapp.css";

/** Intervalo entre consultas automáticas do estado (≥ 3 s, como pede a tarefa). */
export const INTERVALO_DE_CONSULTA_MS = 4_000;

/** Teto de consultas automáticas: 45 × 4 s ≈ 3 minutos de espera pelo pareamento. */
export const MAXIMO_DE_CONSULTAS = 45;

/** Ligações do `SofaField` sem `describedBy`, que vira `aria-describedby` no controle. */
function semDescribedBy(controle: SofaFieldControlProps) {
    const { describedBy, ...resto } = controle;
    return { ...resto, "aria-describedby": describedBy };
}

/** Número no formato que a Evolution aceita: só dígitos, com DDI (10 a 15). */
export const NUMERO_WHATSAPP = /^\d{10,15}$/;

/** Classe do ponto de status (`.poph-status` de `relayout.css`) para o estado da conexão. */
function classeDaConexao(estado: string): string {
    switch (estado) {
        case "connected":
            return "poph-status--ativo";
        case "awaiting_scan":
            return "poph-status--atencao";
        case "disconnected":
            return "poph-status--erro";
        default:
            return "poph-status--inativo";
    }
}

export type PainelWhatsAppProps = {
    org: string;
    agente: string;
    integracao: Integracao;
    /** Devolve ao pai a integração com o estado de conexão / versão atualizados. */
    aoAtualizarIntegracao: (integracao: Integracao) => void;
};

export function PainelWhatsApp({
    org,
    agente,
    integracao,
    aoAtualizarIntegracao,
}: PainelWhatsAppProps) {
    const { translate: t } = useI18n();
    const [qr, setQr] = useState<QrDoWhatsapp | null>(null);
    const [estadoAoVivo, setEstadoAoVivo] = useState<WhatsappState | null>(null);
    const [carregandoQr, setCarregandoQr] = useState(false);
    const [consultando, setConsultando] = useState(false);
    const [desconectando, setDesconectando] = useState(false);
    const [confirmandoDesconexao, setConfirmandoDesconexao] = useState(false);
    const [falha, setFalha] = useState<WhatsappFailure | null>(null);
    const [aviso, setAviso] = useState<string | null>(null);
    const [lacoAtivo, setLacoAtivo] = useState(false);
    const [numero, setNumero] = useState("");
    const [erroDoNumero, setErroDoNumero] = useState<string | null>(null);

    // A versão é a da integração que o pai mantém; o `412` atualiza as duas.
    const versao = integracao.version;
    const id = integracao.id;
    const estado = estadoAoVivo?.connection_state ?? integracao.connection_state;
    const aguardandoLeitura = estado === "awaiting_scan";

    // O objeto `integracao` muda de identidade a cada leitura de estado; o laço usa a versão atual
    // sem reiniciar o intervalo por isso.
    const integracaoRef = useRef(integracao);
    integracaoRef.current = integracao;

    /** No `412`, relê a integração, adota a versão atual e mantém o painel usável. */
    const relerDepoisDoConflito = useCallback(async () => {
        try {
            const atual = await getIntegration(org, agente, id);
            aoAtualizarIntegracao(atual);
            setFalha({
                texto: t("integrations.whatsappConflictText", { version: atual.version }),
                codigo: "version_conflict",
                dica: t("integrations.whatsappConflictTip"),
            });
        } catch {
            setFalha({
                texto: t("integrations.whatsappConflictRereadFailed"),
                codigo: "version_conflict",
            });
        }
    }, [org, agente, id, aoAtualizarIntegracao, t]);

    async function conectar() {
        const digitado = numero.trim();
        if (digitado !== "" && !NUMERO_WHATSAPP.test(digitado)) {
            setErroDoNumero(t("integrations.whatsappPhoneInvalid"));
            return;
        }
        setErroDoNumero(null);
        setCarregandoQr(true);
        setFalha(null);
        setAviso(null);
        try {
            const resposta = await generateWhatsappQr(
                org,
                agente,
                id,
                versao,
                digitado === "" ? undefined : digitado,
            );
            setQr(resposta);
            setEstadoAoVivo({
                state: resposta.state,
                connection_state: resposta.connection_state,
                instance: resposta.instance,
            });
            aoAtualizarIntegracao({
                ...integracaoRef.current,
                connection_state: resposta.connection_state,
            });
            if (resposta.connection_state === "awaiting_scan") {
                setLacoAtivo(true);
                setAviso(
                    resposta.pairing_code
                        ? t("integrations.whatsappQrWithCodeNotice")
                        : t("integrations.whatsappQrNotice"),
                );
            } else if (resposta.state === "open") {
                setLacoAtivo(false);
                setAviso(t("integrations.whatsappAlreadyConnectedNotice"));
            }
        } catch (erro) {
            setLacoAtivo(false);
            if (erro instanceof ApiError && erro.status === 412) {
                await relerDepoisDoConflito();
            } else {
                setFalha(explainWhatsappFailure(erro));
            }
        } finally {
            setCarregandoQr(false);
        }
    }

    const consultarEstado = useCallback(async () => {
        setConsultando(true);
        setFalha(null);
        try {
            const resposta = await readWhatsappState(org, agente, id);
            setEstadoAoVivo(resposta);
            aoAtualizarIntegracao({
                ...integracaoRef.current,
                connection_state: resposta.connection_state,
            });
            // Conectou (ou caiu): o QR que estava na tela não vale mais.
            if (resposta.connection_state !== "awaiting_scan") setQr(null);
            setAviso(
                t("integrations.whatsappLiveStateNotice", {
                    state: connectionLabel(resposta.connection_state).toLowerCase(),
                }),
            );
            return resposta;
        } catch (erro) {
            setFalha(explainWhatsappFailure(erro));
            return null;
        } finally {
            setConsultando(false);
        }
    }, [org, agente, id, aoAtualizarIntegracao, t]);

    // Laço curto enquanto o aparelho aguarda leitura do QR.
    useEffect(() => {
        if (!lacoAtivo || !aguardandoLeitura) return;
        let vivo = true;
        let consultas = 0;
        const intervalo = window.setInterval(() => {
            consultas += 1;
            if (consultas > MAXIMO_DE_CONSULTAS) {
                if (!vivo) return;
                setLacoAtivo(false);
                setAviso(t("integrations.whatsappPollingStoppedNotice"));
                return;
            }
            void readWhatsappState(org, agente, id)
                .then((resposta) => {
                    if (!vivo) return;
                    setEstadoAoVivo(resposta);
                    aoAtualizarIntegracao({
                        ...integracaoRef.current,
                        connection_state: resposta.connection_state,
                    });
                    if (resposta.connection_state !== "awaiting_scan") {
                        setLacoAtivo(false);
                        setQr(null);
                        setAviso(
                            resposta.connection_state === "connected"
                                ? t("integrations.whatsappConnectedNotice")
                                : t("integrations.whatsappLiveStateNotice", {
                                      state: connectionLabel(resposta.connection_state).toLowerCase(),
                                  }),
                        );
                    }
                })
                .catch((erro) => {
                    if (!vivo) return;
                    setLacoAtivo(false);
                    setFalha(explainWhatsappFailure(erro));
                });
        }, INTERVALO_DE_CONSULTA_MS);
        return () => {
            vivo = false;
            window.clearInterval(intervalo);
        };
    }, [lacoAtivo, aguardandoLeitura, org, agente, id, aoAtualizarIntegracao, t]);

    async function desconectar() {
        setConfirmandoDesconexao(false);
        setDesconectando(true);
        setFalha(null);
        setAviso(null);
        try {
            const resposta = await disconnectWhatsapp(org, agente, id, versao);
            setLacoAtivo(false);
            setQr(null);
            setEstadoAoVivo({
                state: "close",
                connection_state: resposta.connection_state,
                instance: resposta.instance,
            });
            aoAtualizarIntegracao({
                ...integracaoRef.current,
                connection_state: resposta.connection_state,
            });
            setAviso(t("integrations.whatsappDisconnectedNotice"));
        } catch (erro) {
            if (erro instanceof ApiError && erro.status === 412) {
                await relerDepoisDoConflito();
            } else {
                setFalha(explainWhatsappFailure(erro));
            }
        } finally {
            setDesconectando(false);
        }
    }

    const base64 = qr?.qrcode?.base64 ?? null;
    const rodada = qr?.qrcode?.count ?? null;

    return (
        <div className="poph-whatsapp">
            <div className="poph-whatsapp__estado">
                <span className="poph-mono-mudo">{id}</span>
                <span className={`poph-status ${classeDaConexao(estado)}`}>
                    <span className="poph-status__ponto" aria-hidden="true" />
                    {connectionLabel(estado)}
                </span>
                <span className="poph-mono-mudo">
                    {integracao.tem_segredo
                        ? t("integrations.whatsappSecretSaved")
                        : t("integrations.whatsappNoSecret")}
                </span>
                {qr?.instance && (
                    <span className="poph-mono-mudo">
                        {t("integrations.whatsappInstanceLabel", { instance: qr.instance })}
                    </span>
                )}
            </div>

            <SofaField
                label={t("integrations.whatsappPhoneLabel")}
                hint={t("integrations.whatsappPhoneHint")}
                error={erroDoNumero ?? undefined}
                htmlFor="poph-whatsapp-numero"
            >
                {(controle) => (
                    <SofaInput
                        {...semDescribedBy(controle)}
                        value={numero}
                        inputMode="numeric"
                        autoComplete="off"
                        placeholder="5511999998888"
                        onChange={(evento) => setNumero(evento.target.value)}
                    />
                )}
            </SofaField>

            <div className="poph-whatsapp__acoes">
                <SofaButton
                    variant="primary"
                    size="sm"
                    icon={<IconMessage aria-hidden="true" />}
                    onClick={() => void conectar()}
                    loading={carregandoQr}
                >
                    {qr ? t("integrations.whatsappRegenerateQr") : t("integrations.whatsappGenerateQr")}
                </SofaButton>
                <SofaButton
                    variant="subtle"
                    size="sm"
                    icon={<IconRefresh aria-hidden="true" />}
                    onClick={() => {
                        void consultarEstado();
                    }}
                    loading={consultando}
                >
                    {t("integrations.whatsappCheckState")}
                </SofaButton>
                <SofaButton
                    variant="subtle"
                    size="sm"
                    icon={<IconLogOut aria-hidden="true" />}
                    onClick={() => {
                        setAviso(null);
                        setConfirmandoDesconexao(true);
                    }}
                    loading={desconectando}
                    disabled={estado === "disconnected"}
                >
                    {t("integrations.whatsappDisconnectDevice")}
                </SofaButton>
                {lacoAtivo && aguardandoLeitura && (
                    <SofaButton variant="subtle" size="sm" onClick={() => setLacoAtivo(false)}>
                        {t("integrations.whatsappStopPolling")}
                    </SofaButton>
                )}
            </div>

            {aviso && (
                <p className="poph-mono-mudo" style={{ margin: 0 }} role="status">
                    {aviso}
                </p>
            )}

            {falha && (
                <SofaBanner tone="danger" title={t("integrations.whatsappOperationErrorTitle")}>
                    <span>{falha.texto}</span>
                    {falha.dica && <div className="poph-whatsapp__dica">{falha.dica}</div>}
                    {falha.codigo && (
                        <div className="poph-mono-mudo poph-whatsapp__dica">
                            {t("integrations.whatsappErrorCode", { code: falha.codigo })}
                        </div>
                    )}
                </SofaBanner>
            )}

            {base64 ? (
                <div className="poph-whatsapp__qr-bloco">
                    <img
                        className="poph-whatsapp__qr-imagem"
                        src={qrSource(base64)}
                        alt={t("integrations.whatsappQrAlt")}
                        width={220}
                        height={220}
                    />
                    <div className="poph-whatsapp__qr-texto">
                        <strong>{t("integrations.whatsappQrHeading")}</strong>
                        <span className="poph-mono-mudo">
                            {t("integrations.whatsappQrSteps")}
                        </span>
                        {rodada !== null && (
                            <span className="poph-mono-mudo">
                                {t("integrations.whatsappQrRound", { round: rodada })}
                            </span>
                        )}
                        {qr?.pairing_code && (
                            <span>
                                {t("integrations.whatsappPairingCode")}{" "}
                                <code className="poph-mono-mudo">{qr.pairing_code}</code>
                            </span>
                        )}
                        <span className="poph-mono-mudo">
                            {t("integrations.whatsappQrRotationHint")}
                        </span>
                    </div>
                </div>
            ) : qr?.state === "open" || estado === "connected" ? (
                <SofaBanner tone="success" title={t("integrations.whatsappConnectedBannerTitle")}>
                    {t("integrations.whatsappConnectedBannerBody")}
                </SofaBanner>
            ) : (
                <p className="poph-mono-mudo" style={{ margin: 0 }}>
                    {t("integrations.whatsappNoQrHint")}
                </p>
            )}

            {aguardandoLeitura && lacoAtivo && (
                <p className="poph-mono-mudo" style={{ margin: 0 }}>
                    {t("integrations.whatsappPollingHint", {
                        seconds: INTERVALO_DE_CONSULTA_MS / 1000,
                        minutes: Math.round((INTERVALO_DE_CONSULTA_MS * MAXIMO_DE_CONSULTAS) / 60_000),
                    })}
                </p>
            )}

            <SofaModal
                open={confirmandoDesconexao}
                onOpenChange={(novo) => {
                    if (!novo && !desconectando) setConfirmandoDesconexao(false);
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
                            disabled={desconectando}
                            onClick={() => setConfirmandoDesconexao(false)}
                        >
                            {t("integrations.whatsappDisconnectCancel")}
                        </SofaButton>
                        <SofaButton
                            variant="danger"
                            icon={<IconLogOut aria-hidden="true" />}
                            loading={desconectando}
                            onClick={() => void desconectar()}
                        >
                            {t("integrations.whatsappDisconnectConfirm")}
                        </SofaButton>
                    </>
                }
            >
                <div className="poph-whatsapp__confirmacao">
                    <div className="poph-whatsapp__confirmacao-linha">
                        <IconAlertTriangle aria-hidden="true" />
                        <span>
                            {t("integrations.whatsappDisconnectBodyPrefix")}
                            <strong>{t("integrations.whatsappDisconnectBodyStrong")}</strong>
                            {t("integrations.whatsappDisconnectBodySuffix")}
                        </span>
                    </div>
                    <p className="poph-mono-mudo" style={{ margin: 0 }}>
                        <IconLink aria-hidden="true" />{" "}
                        {t("integrations.whatsappDisconnectIntegrationId", { id })}
                    </p>
                </div>
            </SofaModal>
        </div>
    );
}
