/**
 * Operação da instância (HR-48a, HR-48c): o servidor está de pé, em que versão, há quanto tempo,
 * como estão as filas e quanto de recurso ele usa, e quais plugins estão declarados (com o detalhe
 * de cada um).
 *
 * Quatro seções independentes. Cada uma tem carga, erro com "Tentar de novo" e texto próprio: uma
 * falha em uma não derruba as outras. A atualização automática acontece a cada 15 s, só com a aba
 * visível; o botão "Atualizar" faz o mesmo na hora. As rotas estão em `api/operacao.ts`.
 *
 * Seção "Cluster" (HR-48f): recarrega o mapa de cluster no processo vivo, com confirmação. A tela
 * não mostra o mapa em uso agora; mostra o resultado das recargas feitas nesta sessão.
 *
 * O que a tela não mostra, de propósito: instalar ou remover plugin (a API não tem essa ação) e a
 * latência por etapa do turno (é um histograma, que a tela ainda não desenha).
 */

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

/** Intervalo da atualização automática. Nunca menos que isso (sem laço de requisições). */
export const INTERVALO_DE_ATUALIZACAO_MS = 15_000;

/** Tempo no ar no idioma ativo, do mais grosso ao mais fino. */
export function formatarTempoNoAr(segundos: number): string {
    const total = Math.max(0, Math.floor(segundos));
    const dias = Math.floor(total / 86_400);
    const horas = Math.floor((total % 86_400) / 3_600);
    const minutos = Math.floor((total % 3_600) / 60);
    if (dias > 0) {
        return translate(dias === 1 ? "operation.uptimeDaysOne" : "operation.uptimeDaysMany", {
            days: dias,
            hours: horas,
        });
    }
    if (horas > 0) return translate("operation.uptimeHours", { hours: horas, minutes: minutos });
    if (minutos > 0) return translate("operation.uptimeMinutes", { minutes: minutos });
    return translate("operation.uptimeLessThanMinute");
}

/** Bytes em KB, MB ou GB, no idioma ativo. */
export function formatarMemoria(bytes: number): string {
    const kb = 1024;
    const mb = kb * 1024;
    const gb = mb * 1024;
    const formatar = (valor: number, unidade: string) =>
        `${formatNumber(valor, { maximumFractionDigits: 1 })} ${unidade}`;
    if (bytes >= gb) return formatar(bytes / gb, "GB");
    if (bytes >= mb) return formatar(bytes / mb, "MB");
    return formatar(bytes / kb, "KB");
}

/**
 * Versão de esquema. O servidor manda `0` quando o arquivo do banco ainda não foi migrado (ou não
 * pôde ser lido): não é falha de conexão, então a tela diz só que não há versão gravada.
 */
export function formatarVersaoDoBanco(versao: number): string {
    return versao > 0
        ? translate("operation.schemaVersion", { version: versao })
        : translate("operation.schemaVersionMissing");
}

function numero(valor: number | undefined): string {
    return valor === undefined ? translate("operation.notInformed") : formatNumber(valor);
}

function memoria(valor: number | undefined): string {
    return valor === undefined ? translate("operation.notInformed") : formatarMemoria(valor);
}

/** Frase para o operador a partir do erro da API (sem caminho, sem corpo cru). */
export function mensagemDeFalha(erro: unknown, oQue: string): string {
    if (erro instanceof ApiError) {
        if (erro.semServidor) return translate("operation.errorOffline");
        if (erro.status === 401) return translate("operation.errorSession");
        if (erro.status === 403) return translate("operation.errorForbidden");
        if (erro.status === 404) return translate("operation.errorNotFound", { what: oQue });
        if (erro.status >= 500) {
            return translate("operation.errorServer", { status: erro.status });
        }
        return erro.message;
    }
    return translate("operation.errorGeneric");
}

type FalhaDaSecao = { erro: unknown; what: TranslationKey };

function textoDaFalha(falha: FalhaDaSecao): string {
    return mensagemDeFalha(falha.erro, translate(falha.what));
}

type Estado<T> = { carregando: boolean; dados: T | null; erro: FalhaDaSecao | null };

const estadoInicial = <T,>(): Estado<T> => ({ carregando: true, dados: null, erro: null });

type Consulta = {
    carregando: boolean;
    kind: string;
    manifesto: PluginManifest | null;
    erro: unknown;
};

function mensagemDoPlugin(erro: unknown): string {
    if (erro instanceof ApiError && erro.status === 404) return translate("operation.pluginNotDeclared");
    if (erro instanceof ApiError && erro.status === 503) return translate("operation.pluginNotResponding");
    return mensagemDeFalha(erro, translate("operation.whatPlugin"));
}

function Propriedade({ chave, valor, mono }: { chave: string; valor: string; mono?: boolean }) {
    return (
        <div className="poph-property">
            <dt className="poph-property__key">{chave}</dt>
            <dd className={mono ? "poph-property__value poph-property__value--mono" : "poph-property__value"}>
                {valor}
            </dd>
        </div>
    );
}

/**
 * Uma linha da lista de plugins. Plugin no ar tem botão de detalhe com o tipo no rótulo. Plugin
 * indisponível não tem ação: mostra a situação em texto (não só cor) e a mensagem do servidor.
 */
function LinhaDePlugin({
    item,
    aberto,
    onAbrir,
}: {
    item: PluginItem;
    aberto: boolean;
    onAbrir: () => void;
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
                    aria-pressed={aberto}
                    onClick={onAbrir}
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

/** Quantos caracteres da impressão digital aparecem na tela; o valor completo fica no `title`. */
const TAMANHO_DA_IMPRESSAO = 12;

/** Impressão digital encurtada para a tela (o valor completo continua no atributo `title`). */
export function impressaoNaTela(impressao: string): string {
    if (!impressao) return translate("operation.fingerprintMissing");
    return impressao.length > TAMANHO_DA_IMPRESSAO
        ? `${impressao.slice(0, TAMANHO_DA_IMPRESSAO)}…`
        : impressao;
}

/**
 * Falha de uma recarga, já em forma de tela. Em `invalido` o mapa anterior segue em uso; em
 * `sem_permissao` o token não é global; em `outra` a frase vem pronta.
 */
export type FalhaDaRecarga =
    | { tipo: "invalido"; arquivo: string | null; problemas: string[] }
    | { tipo: "sem_permissao" }
    | { tipo: "sem_resposta" }
    | { tipo: "outra"; erro: unknown };

/** Traduz o erro da recarga. Sem resposta da instância, a tela diz que não sabe se foi aplicada. */
export function falhaDaRecarga(erro: unknown): FalhaDaRecarga {
    if (erro instanceof ApiError) {
        if (erro.status === 400) {
            const bruto: unknown = erro.detalhes;
            const detalhes =
                bruto !== null && typeof bruto === "object" ? (bruto as Record<string, unknown>) : {};
            const problemas = Array.isArray(detalhes.problemas)
                ? detalhes.problemas.filter((item): item is string => typeof item === "string")
                : [];
            return {
                tipo: "invalido",
                arquivo: typeof detalhes.arquivo === "string" ? detalhes.arquivo : null,
                problemas: problemas.length > 0 ? problemas : [erro.message],
            };
        }
        if (erro.status === 403) return { tipo: "sem_permissao" };
        if (erro.semServidor) return { tipo: "sem_resposta" };
    }
    return { tipo: "outra", erro };
}

function listaOuNenhuma(itens: string[]): string {
    return itens.length > 0 ? itens.join(", ") : translate("operation.listNone");
}

function modoEmTexto(resumo: ClusterMapSummary): string {
    return resumo.modo === "cluster"
        ? translate("operation.mapModeCluster")
        : translate("operation.mapModeSingle");
}

/** Resumo de um mapa em duas colunas (antes e depois). Sem segredo: só se está configurado. */
function TabelaDoMapa({ antes, depois, impressaoAntes, impressaoDepois }: {
    antes: ClusterMapSummary;
    depois: ClusterMapSummary;
    impressaoAntes: string;
    impressaoDepois: string;
}) {
    const { translate: t } = useI18n();
    const linhas: { rotulo: string; antes: ReactNode; depois: ReactNode }[] = [
        { rotulo: t("operation.mapMode"), antes: modoEmTexto(antes), depois: modoEmTexto(depois) },
        {
            rotulo: t("operation.mapThisInstance"),
            antes: antes.instanciaLocal || t("operation.mapNotDefined"),
            depois: depois.instanciaLocal || t("operation.mapNotDefined"),
        },
        {
            rotulo: t("operation.mapDefaultInstance"),
            antes: antes.instanciaPadrao ?? t("operation.mapNone"),
            depois: depois.instanciaPadrao ?? t("operation.mapNone"),
        },
        {
            rotulo: t("operation.mapDeclaredInstances"),
            antes: listaOuNenhuma(antes.instancias.map((item) => item.id)),
            depois: listaOuNenhuma(depois.instancias.map((item) => item.id)),
        },
        {
            rotulo: t("operation.mapRoutedOrgs"),
            antes: formatNumber(antes.quantidadeDeOrgs),
            depois: formatNumber(depois.quantidadeDeOrgs),
        },
        {
            rotulo: t("operation.mapSharedSecret"),
            antes: antes.segredoConfigurado ? t("operation.mapConfigured") : t("operation.mapNotConfigured"),
            depois: depois.segredoConfigurado
                ? t("operation.mapConfigured")
                : t("operation.mapNotConfigured"),
        },
        {
            rotulo: t("operation.mapFingerprint"),
            antes: <code title={impressaoAntes}>{impressaoNaTela(impressaoAntes)}</code>,
            depois: <code title={impressaoDepois}>{impressaoNaTela(impressaoDepois)}</code>,
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
                {linhas.map((linha) => (
                    <tr key={linha.rotulo}>
                        <th scope="row">{linha.rotulo}</th>
                        <td>{linha.antes}</td>
                        <td>{linha.depois}</td>
                    </tr>
                ))}
            </tbody>
        </table>
    );
}

/** Resultado de uma recarga que deu certo: mudou ou não, o que entrou e saiu, e as orgs que trocaram. */
function RelatorioDaRecarga({ relatorio, titulo }: { relatorio: ReloadReport; titulo: string }) {
    const { translate: t } = useI18n();
    return (
        <div className="poph-operation__report">
            <h3 className="poph-operation__subtitle">{titulo}</h3>
            <SofaBanner
                tone={relatorio.mudou ? "success" : "info"}
                title={
                    relatorio.mudou
                        ? t("operation.clusterChangedTitle")
                        : t("operation.clusterUnchangedTitle")
                }
            >
                {relatorio.mudou
                    ? t("operation.clusterChangedText")
                    : t("operation.clusterUnchangedText")}
            </SofaBanner>
            {relatorio.depois.modo === "instancia_unica" ? (
                <p className="poph-operation__help">{t("operation.clusterSingleInstance")}</p>
            ) : null}
            <TabelaDoMapa
                antes={relatorio.antes}
                depois={relatorio.depois}
                impressaoAntes={relatorio.impressaoAntes}
                impressaoDepois={relatorio.impressaoDepois}
            />
            <p className="poph-operation__help">
                {t("operation.clusterInstancesAdded", {
                    list: listaOuNenhuma(relatorio.instanciasAdicionadas),
                })}
            </p>
            <p className="poph-operation__help">
                {t("operation.clusterInstancesRemoved", {
                    list: listaOuNenhuma(relatorio.instanciasRemovidas),
                })}
            </p>
            {relatorio.orgsQueTrocaram.length > 0 ? (
                <>
                    <p className="poph-operation__help">{t("operation.clusterOrgsChanged")}</p>
                    <ul
                        className="poph-operation__conflicts"
                        aria-label={t("operation.clusterOrgsChangedLabel")}
                    >
                        {relatorio.orgsQueTrocaram.map((troca) => (
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

/** Aviso de uma recarga que falhou. Em 400 diz que o mapa anterior segue valendo. */
function AvisoDaRecarga({ falha, aoTentarDeNovo }: { falha: FalhaDaRecarga; aoTentarDeNovo: () => void }) {
    const { translate: t } = useI18n();
    if (falha.tipo === "invalido") {
        return (
            <SofaBanner tone="danger" title={t("operation.clusterInvalidTitle")}>
                <p style={{ margin: 0 }}>{t("operation.clusterInvalidText")}</p>
                {falha.arquivo ? (
                    <p className="poph-mono-muted" style={{ margin: "6px 0 0" }}>
                        {t("operation.clusterFile", { file: falha.arquivo })}
                    </p>
                ) : null}
                <p style={{ margin: "6px 0 0" }}>{t("operation.clusterProblems")}</p>
                <ul
                    className="poph-operation__conflicts"
                    aria-label={t("operation.clusterProblemsLabel")}
                >
                    {falha.problemas.map((problema, indice) => (
                        <li key={`${indice}-${problema}`}>{problema}</li>
                    ))}
                </ul>
            </SofaBanner>
        );
    }
    if (falha.tipo === "sem_permissao") {
        return (
            <SofaBanner tone="danger" title={t("operation.clusterForbiddenTitle")}>
                <p style={{ margin: 0 }}>{t("operation.clusterForbiddenText")}</p>
            </SofaBanner>
        );
    }
    const message =
        falha.tipo === "sem_resposta"
            ? t("operation.clusterNetworkUnknown")
            : mensagemDeFalha(falha.erro, t("operation.whatClusterReload"));
    return (
        <SofaBanner tone="danger" title={t("operation.clusterFailedTitle")}>
            <p style={{ margin: 0 }}>{message}</p>
            <div style={{ marginTop: 8 }}>
                <SofaButton variant="secondary" onClick={aoTentarDeNovo}>
                    {t("operation.retry")}
                </SofaButton>
            </div>
        </SofaBanner>
    );
}

/**
 * Seção Cluster. A recarga é ação da instalação inteira: o botão só abre a confirmação, e o pedido
 * sai uma vez (a trava `emCurso` e o botão desabilitado impedem duplo envio).
 */
function SecaoCluster() {
    const { translate: t } = useI18n();
    const idSecao = useId();
    const [confirmando, setConfirmando] = useState(false);
    const [recarregando, setRecarregando] = useState(false);
    const [ultimoSucesso, setUltimoSucesso] = useState<ReloadReport | null>(null);
    const [falha, setFalha] = useState<FalhaDaRecarga | null>(null);
    const emCurso = useRef(false);
    const vivo = useRef(true);

    useEffect(() => {
        vivo.current = true;
        return () => {
            vivo.current = false;
        };
    }, []);

    const recarregar = useCallback(async () => {
        if (emCurso.current) return;
        emCurso.current = true;
        setConfirmando(false);
        setRecarregando(true);
        setFalha(null);
        try {
            const relatorio = await reloadCluster();
            if (!vivo.current) return;
            setUltimoSucesso(relatorio);
        } catch (erro) {
            if (!vivo.current) return;
            setFalha(falhaDaRecarga(erro));
        } finally {
            emCurso.current = false;
            if (vivo.current) setRecarregando(false);
        }
    }, []);

    return (
        <section className="poph-section" aria-labelledby={idSecao}>
            <h2 id={idSecao} className="poph-section__title">{t("operation.sectionCluster")}</h2>
            <p className="poph-operation__help">{t("operation.clusterHelp")}</p>

            <div className="poph-stack" style={{ gap: "var(--pop-spacing-sm, 6px)", maxWidth: 680 }}>
                <div>
                    <SofaButton
                        variant="secondary"
                        loading={recarregando}
                        disabled={recarregando}
                        onClick={() => setConfirmando(true)}
                    >
                        {t("operation.clusterReload")}
                    </SofaButton>
                </div>

                <div aria-live="polite">
                    {recarregando ? (
                        <p className="poph-operation__loading" role="status">
                            {t("operation.clusterReloading")}
                        </p>
                    ) : null}
                    {falha ? (
                        <AvisoDaRecarga falha={falha} aoTentarDeNovo={() => setConfirmando(true)} />
                    ) : null}
                    {ultimoSucesso ? (
                        <RelatorioDaRecarga
                            relatorio={ultimoSucesso}
                            titulo={
                                falha
                                    ? t("operation.clusterLastSuccessTitle")
                                    : t("operation.clusterResultTitle")
                            }
                        />
                    ) : !falha && !recarregando ? (
                        <p className="poph-operation__empty">{t("operation.clusterEmpty")}</p>
                    ) : null}
                </div>
            </div>

            <SofaConfirmDialog
                open={confirmando}
                onOpenChange={(aberto) => {
                    if (!aberto && !recarregando) setConfirmando(false);
                }}
                title={t("operation.clusterConfirmTitle")}
                description={t("operation.clusterConfirmDescription")}
                confirmLabel={t("operation.clusterConfirmAction")}
                cancelLabel={t("common.cancel")}
                closeLabel={t("operation.clusterConfirmClose")}
                onConfirm={() => void recarregar()}
                loading={recarregando}
            />
        </section>
    );
}

export function Operation() {
    const { orgId = "" } = useParams();
    const { translate: t, formatTime } = useI18n();
    const [saude, setSaude] = useState<Estado<InstanceHealth>>(estadoInicial);
    const [metricas, setMetricas] = useState<Estado<Measurements>>(estadoInicial);
    const [plugins, setPlugins] = useState<Estado<PluginItem[]>>(estadoInicial);
    const [atualizadoEm, setAtualizadoEm] = useState<Date | null>(null);
    const [consulta, setConsulta] = useState<Consulta>({
        carregando: false,
        kind: "",
        manifesto: null,
        erro: null,
    });
    const emCurso = useRef(false);
    const vivo = useRef(true);
    const idSaude = useId();
    const idMetricas = useId();
    const idPlugin = useId();

    useEffect(() => {
        vivo.current = true;
        return () => {
            vivo.current = false;
        };
    }, []);

    const carregar = useCallback(async () => {
        if (emCurso.current) return;
        emCurso.current = true;
        setSaude((atual) => ({ ...atual, carregando: true }));
        setMetricas((atual) => ({ ...atual, carregando: true }));
        setPlugins((atual) => ({ ...atual, carregando: true }));
        const [resSaude, resMetricas, resPlugins] = await Promise.allSettled([
            fetchHealth(),
            fetchMetrics(),
            listPlugins(orgId),
        ]);
        emCurso.current = false;
        if (!vivo.current) return;

        if (resSaude.status === "fulfilled") {
            setSaude({ carregando: false, dados: resSaude.value, erro: null });
        } else {
            setSaude((atual) => ({
                carregando: false,
                dados: atual.dados,
                erro: { erro: resSaude.reason, what: "operation.whatServerState" },
            }));
        }

        if (resMetricas.status === "fulfilled") {
            setMetricas({ carregando: false, dados: resMetricas.value, erro: null });
        } else {
            setMetricas((atual) => ({
                carregando: false,
                dados: atual.dados,
                erro: { erro: resMetricas.reason, what: "operation.whatMetrics" },
            }));
        }

        if (resPlugins.status === "fulfilled") {
            setPlugins({ carregando: false, dados: resPlugins.value, erro: null });
        } else {
            setPlugins((atual) => ({
                carregando: false,
                dados: atual.dados,
                erro: { erro: resPlugins.reason, what: "operation.whatPluginList" },
            }));
        }
        setAtualizadoEm(new Date());
    }, [orgId]);

    useEffect(() => {
        void carregar();
        const id = window.setInterval(() => {
            if (document.visibilityState === "visible") void carregar();
        }, INTERVALO_DE_ATUALIZACAO_MS);
        return () => window.clearInterval(id);
    }, [carregar]);

    const consultarTipo = useCallback(
        async (kind: string) => {
            setConsulta({ carregando: true, kind, manifesto: null, erro: null });
            try {
                const manifesto = await fetchPlugin(orgId, kind);
                if (!vivo.current) return;
                setConsulta({ carregando: false, kind, manifesto, erro: null });
            } catch (erro) {
                if (!vivo.current) return;
                setConsulta({ carregando: false, kind, manifesto: null, erro });
            }
        },
        [orgId],
    );

    const m = metricas.dados;
    const s = saude.dados;
    const lista = plugins.dados;

    return (
        <div className="poph-page poph-operation">
            <div className="poph-page__top">
                <div>
                    <h1 className="poph-page__title">{t("operation.pageTitle")}</h1>
                    <p className="poph-page__description">{t("operation.pageDescription")}</p>
                </div>
                <div className="poph-page__actions">
                    <span className="poph-operation__update" role="status" aria-live="polite">
                        {saude.carregando || metricas.carregando || plugins.carregando
                            ? t("operation.refreshing")
                            : atualizadoEm
                              ? t("operation.updatedAt", { time: formatTime(atualizadoEm) })
                              : ""}
                    </span>
                    <SofaButton
                        variant="subtle"
                        icon={<IconRefresh aria-hidden="true" />}
                        onClick={() => void carregar()}
                        disabled={saude.carregando || metricas.carregando || plugins.carregando}
                    >
                        {t("operation.refresh")}
                    </SofaButton>
                </div>
            </div>

            <section className="poph-section" aria-labelledby={idSaude}>
                <h2 id={idSaude} className="poph-section__title">{t("operation.sectionServer")}</h2>
                {saude.carregando && !s ? (
                    <SofaSkeleton lines={3} />
                ) : saude.erro && !s ? (
                    <SofaErrorState
                        title={t("operation.serverLoadFailed")}
                        description={textoDaFalha(saude.erro)}
                        retry={{ label: t("operation.retry"), onClick: () => void carregar() }}
                    />
                ) : s ? (
                    <>
                        {saude.erro ? (
                            <p className="poph-operation__notice" role="alert">
                                {t("operation.lastUpdateFailed", { message: textoDaFalha(saude.erro) })}
                            </p>
                        ) : null}
                        <dl className="poph-properties">
                            <Propriedade
                                chave={t("operation.propertyStatus")}
                                valor={
                                    s.situacao === "ok"
                                        ? t("operation.statusOnline")
                                        : t("operation.statusDegraded")
                                }
                            />
                            <Propriedade
                                chave={t("operation.propertyServerVersion")}
                                valor={s.saude.version}
                                mono
                            />
                            <Propriedade
                                chave={t("operation.propertyUptime")}
                                valor={formatarTempoNoAr(s.saude.uptime_seconds)}
                            />
                            <Propriedade
                                chave={t("operation.propertyChatDatabase")}
                                valor={formatarVersaoDoBanco(s.saude.databases.harness)}
                            />
                            <Propriedade
                                chave={t("operation.propertySearchDatabase")}
                                valor={formatarVersaoDoBanco(s.saude.databases.rag)}
                            />
                        </dl>
                    </>
                ) : null}
            </section>

            <section className="poph-section" aria-labelledby={idMetricas}>
                <h2 id={idMetricas} className="poph-section__title">{t("operation.sectionMetrics")}</h2>
                {metricas.carregando && !m ? (
                    <SofaSkeleton lines={4} />
                ) : metricas.erro && !m ? (
                    <SofaErrorState
                        title={t("operation.metricsLoadFailed")}
                        description={textoDaFalha(metricas.erro)}
                        retry={{ label: t("operation.retry"), onClick: () => void carregar() }}
                    />
                ) : m ? (
                    <>
                        {metricas.erro ? (
                            <p className="poph-operation__notice" role="alert">
                                {t("operation.lastUpdateFailed", { message: textoDaFalha(metricas.erro) })}
                            </p>
                        ) : null}
                        <h3 className="poph-operation__subtitle">{t("operation.subtitleNow")}</h3>
                        <dl className="poph-properties">
                            <Propriedade
                                chave={t("operation.metricQueueTurns")}
                                valor={numero(m.filaTurnos)}
                            />
                            <Propriedade
                                chave={t("operation.metricOutbox")}
                                valor={numero(m.filaSaida)}
                            />
                            <Propriedade
                                chave={t("operation.metricActiveConversations")}
                                valor={numero(m.conversasAtivas)}
                            />
                            <Propriedade
                                chave={t("operation.metricLoadedAgents")}
                                valor={numero(m.agentesCarregados)}
                            />
                            <Propriedade
                                chave={t("operation.metricProcessMemory")}
                                valor={memoria(m.memoriaProcesso)}
                            />
                            <Propriedade
                                chave={t("operation.metricChatDatabaseSize")}
                                valor={memoria(m.bancoPrincipal)}
                            />
                            <Propriedade
                                chave={t("operation.metricSearchDatabaseSize")}
                                valor={memoria(m.bancoBusca)}
                            />
                        </dl>
                        <h3 className="poph-operation__subtitle">{t("operation.subtitleSinceBoot")}</h3>
                        <dl className="poph-properties">
                            <Propriedade
                                chave={t("operation.metricLlmCalls")}
                                valor={t("operation.valueWithErrors", {
                                    value: numero(m.chamadasLlm),
                                    errors: numero(m.errosLlm),
                                })}
                            />
                            <Propriedade
                                chave={t("operation.metricJevCalls")}
                                valor={t("operation.valueWithErrors", {
                                    value: numero(m.chamadasJev),
                                    errors: numero(m.errosJev),
                                })}
                            />
                            <Propriedade
                                chave={t("operation.metricEvolutionCalls")}
                                valor={t("operation.valueWithErrors", {
                                    value: numero(m.chamadasEvolution),
                                    errors: numero(m.errosEvolution),
                                })}
                            />
                            <Propriedade
                                chave={t("operation.metricTokens")}
                                valor={numero(m.tokens)}
                            />
                        </dl>
                    </>
                ) : null}
            </section>

            <section className="poph-section" aria-labelledby={idPlugin}>
                <h2 id={idPlugin} className="poph-section__title">{t("operation.sectionPlugins")}</h2>
                <p className="poph-operation__help">{t("operation.pluginsHelp")}</p>

                {plugins.carregando && !lista ? (
                    <>
                        <p className="poph-operation__loading" role="status">
                            {t("operation.pluginsLoading")}
                        </p>
                        <SofaSkeleton lines={3} />
                    </>
                ) : plugins.erro && !lista ? (
                    <SofaErrorState
                        title={t("operation.pluginsLoadFailed")}
                        description={textoDaFalha(plugins.erro)}
                        retry={{ label: t("operation.retry"), onClick: () => void carregar() }}
                    />
                ) : lista ? (
                    <>
                        {plugins.erro ? (
                            <p className="poph-operation__notice" role="alert">
                                {t("operation.lastUpdateFailed", { message: textoDaFalha(plugins.erro) })}
                            </p>
                        ) : null}
                        {lista.length === 0 ? (
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
                                {lista.map((item) => (
                                    <LinhaDePlugin
                                        key={item.kind}
                                        item={item}
                                        aberto={consulta.kind === item.kind && !!consulta.manifesto}
                                        onAbrir={() => void consultarTipo(item.kind)}
                                    />
                                ))}
                            </ul>
                        )}
                    </>
                ) : null}

                <div aria-live="polite">
                    {consulta.carregando ? (
                        <>
                            <p className="poph-operation__loading" role="status">
                                {t("operation.pluginConsulting", { kind: consulta.kind })}
                            </p>
                            <SofaSkeleton lines={3} />
                        </>
                    ) : consulta.erro ? (
                        <SofaErrorState
                            title={t("operation.pluginConsultFailed", { kind: consulta.kind })}
                            description={mensagemDoPlugin(consulta.erro)}
                            retry={{
                                label: t("operation.retry"),
                                onClick: () => void consultarTipo(consulta.kind),
                            }}
                        />
                    ) : consulta.manifesto ? (
                        <dl
                            className="poph-properties poph-operation__detail"
                            aria-label={t("operation.pluginDetailsLabel", { kind: consulta.kind })}
                        >
                            <Propriedade chave={t("operation.propertyName")} valor={consulta.manifesto.nome} />
                            <Propriedade chave={t("operation.propertyKind")} valor={consulta.manifesto.kind} mono />
                            <Propriedade
                                chave={t("operation.propertyVersion")}
                                valor={consulta.manifesto.versao}
                                mono
                            />
                            <Propriedade
                                chave={t("operation.propertyConfigFields")}
                                valor={formatNumber(manifestSummary(consulta.manifesto).configuracao)}
                            />
                            <Propriedade
                                chave={t("operation.propertySecretFields")}
                                valor={formatNumber(manifestSummary(consulta.manifesto).segredos)}
                            />
                        </dl>
                    ) : null}
                </div>
            </section>

            <SecaoCluster />

            <ExportImportOrganization key={orgId} org={orgId} />
        </div>
    );
}
