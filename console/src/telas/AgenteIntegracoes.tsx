/**
 * Aba Integrações do agente (UX_SPEC §3.5 / HR-44, HR-44d e HR-44e): ciclo de vida das integrações, o
 * estado do canal e a configuração que cada plugin declara.
 *
 * O que a tela faz de verdade, pelo contrato que existe hoje (`api/integracoes.ts`): lista as
 * integrações do agente, cria (tipo + situação + o formulário do manifesto, quando o tipo tem
 * plugin), altera com `If-Match` e exclui com confirmação. Mostra o estado de conexão que o canal
 * informa e se há segredo gravado — o valor do segredo nunca é exibido.
 *
 * Na integração de `kind = "whatsapp"`, o painel da HR-44d pareia o aparelho (QR), consulta o
 * estado ao vivo e desconecta — contra as rotas de admin da HR-44b.
 *
 * A configuração por formulário gerado do manifesto (HR-44e) usa `GET …/manifests/{kind}`: sem
 * manifesto para o tipo, a integração é criada só com tipo e situação, e a tela diz isso.
 *
 * Re-layout (RL-12): a lista saiu da `SofaTable` em cartão para a tabela contínua de 56 px de
 * `relayout.css`, com as ações por linha no menu "⋯" (padrão da RL-05/RL-11) e o cabeçalho comum
 * das abas do agente (`CabecalhoDoAgente`, RL-09). O painel do WhatsApp virou seção plana. Nenhuma
 * consulta ou fluxo mudou: `412` continua relendo a versão, o segredo continua mascarado e os
 * diálogos (criar, alterar, excluir) são os mesmos.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { useParams } from "react-router-dom";
import {
    IconArrowLeft,
    IconKey,
    IconMoreHorizontal,
    IconPencil,
    IconPlus,
    IconRefresh,
    IconTrash,
    IconWorkflow,
    SofaBanner,
    SofaButton,
    SofaEmptyState,
    SofaErrorState,
    SofaIconButton,
    SofaMenu,
    SofaSkeleton,
    type SofaMenuItemSpec,
} from "@sofacoding/sofa-ui";
import { getAgent, type Agente } from "../api/agents";
import {
    KIND_WHATSAPP,
    deleteIntegration,
    listIntegrations,
    connectionLabel,
    type Integracao,
} from "../api/integrations";
import { getOrg } from "../api/orgs";
import { TabsAgent, HeaderAgent } from "../components/TabsAgent";
import { DialogExclusion } from "../components/DialogExclusion";
import { useGuardChanges } from "../components/GuardChanges";
import { ModalIntegration } from "../components/ModalIntegration";
import { PanelWhatsApp } from "../components/PanelWhatsApp";
import { translate, useI18n } from "../i18n";
import "../styles/integrations.css";

/** Classe do ponto de status (`.poph-status` de `relayout.css`) para a situação da integração. */
function classeDaSituacao(situacao: string): string {
    switch (situacao) {
        case "active":
            return "poph-status--active";
        case "inactive":
            return "poph-status--attention";
        default:
            return "poph-status--inactive";
    }
}

function rotuloDaSituacao(situacao: string): string {
    switch (situacao) {
        case "active":
            return translate("integrations.statusActive");
        case "inactive":
            return translate("integrations.statusInactive");
        case "archived":
            return translate("integrations.statusArchived");
        default:
            return situacao;
    }
}

function classeDaConexao(estado: string): string {
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

export function AgentIntegrations() {
    const { translate: t } = useI18n();
    const { orgId = "", agentId = "" } = useParams();
    const guarda = useGuardChanges();

    const [agente, setAgente] = useState<Agente | null>(null);
    const [nomeDaOrg, setNomeDaOrg] = useState<string | null>(null);
    const [erroDeCarga, setErroDeCarga] = useState<string | null>(null);

    const [integracoes, setIntegracoes] = useState<Integracao[]>([]);
    const [carregando, setCarregando] = useState(true);
    const [erro, setErro] = useState<string | null>(null);

    const [modalAberto, setModalAberto] = useState(false);
    const [emEdicao, setEmEdicao] = useState<Integracao | null>(null);
    const [paraExcluir, setParaExcluir] = useState<Integracao | null>(null);
    const [aviso, setAviso] = useState<string | null>(null);

    const carregarIntegracoes = useCallback(async () => {
        setCarregando(true);
        setErro(null);
        try {
            setIntegracoes(await listIntegrations(orgId, agentId));
        } catch (falha) {
            setErro(
                falha instanceof Error
                    ? falha.message
                    : t("integrations.loadIntegrationsFallbackError"),
            );
            setIntegracoes([]);
        } finally {
            setCarregando(false);
        }
    }, [orgId, agentId, t]);

    /** Atualiza uma integração na lista (o painel do WhatsApp devolve o estado de conexão). */
    const atualizarIntegracao = useCallback((atualizada: Integracao) => {
        setIntegracoes((atuais) =>
            atuais.map((item) => (item.id === atualizada.id ? atualizada : item)),
        );
    }, []);

    const carregar = useCallback(async () => {
        setErroDeCarga(null);
        try {
            setAgente(await getAgent(orgId, agentId));
        } catch (falha) {
            setErroDeCarga(
                falha instanceof Error ? falha.message : t("integrations.loadAgentFallbackError"),
            );
            return;
        }
        void carregarIntegracoes();
    }, [orgId, agentId, carregarIntegracoes, t]);

    useEffect(() => {
        void carregar();
        getOrg(orgId)
            .then((org) => setNomeDaOrg(org.name))
            .catch(() => setNomeDaOrg(null));
    }, [carregar, orgId]);

    const doWhatsapp = useMemo(
        () => integracoes.filter((integracao) => integracao.kind === KIND_WHATSAPP),
        [integracoes],
    );

    if (!agente && erroDeCarga) {
        return (
            <div className="poph-page">
                <SofaErrorState
                    title={t("integrations.loadAgentErrorTitle")}
                    description={t("integrations.loadAgentErrorDescription")}
                    details={erroDeCarga}
                    retry={{ label: t("integrations.retry"), onClick: () => void carregar() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            icon={<IconArrowLeft aria-hidden="true" />}
                            onClick={() => guarda.tentarNavegar(`/orgs/${orgId}/agents`)}
                        >
                            {t("integrations.backToAgents")}
                        </SofaButton>
                    }
                />
            </div>
        );
    }

    if (!agente) {
        return (
            <div className="poph-page" aria-busy="true">
                <div className="poph-page__top">
                    <div>
                        <SofaSkeleton width={260} height={26} />
                        <div className="poph-integrations__skeleton-line">
                            <SofaSkeleton width={360} height={12} />
                        </div>
                    </div>
                </div>
                <SofaSkeleton lines={6} />
            </div>
        );
    }

    /** Ações da linha no menu "⋯" (mesmos fluxos dos botões soltos de antes). */
    function itensDoMenu(integracao: Integracao): SofaMenuItemSpec[] {
        return [
            {
                id: "editar",
                label: t("integrations.actionEdit"),
                icon: <IconPencil size={14} aria-hidden="true" />,
                onSelect: () => {
                    setAviso(null);
                    setEmEdicao(integracao);
                    setModalAberto(true);
                },
            },
            {
                id: "excluir",
                label: t("integrations.actionDelete"),
                icon: <IconTrash size={14} aria-hidden="true" />,
                danger: true,
                onSelect: () => {
                    setAviso(null);
                    setParaExcluir(integracao);
                },
            },
        ];
    }

    let corpo;
    if (carregando && integracoes.length === 0) {
        corpo = <SofaSkeleton lines={5} />;
    } else if (erro) {
        corpo = (
            <SofaErrorState
                title={t("integrations.loadListErrorTitle")}
                description={t("integrations.loadListErrorDescription")}
                details={erro}
                retry={{ label: t("integrations.retry"), onClick: () => void carregarIntegracoes() }}
            />
        );
    } else if (integracoes.length === 0) {
        corpo = (
            <SofaEmptyState
                icon={<IconWorkflow aria-hidden="true" />}
                title={t("integrations.emptyTitle")}
                description={t("integrations.emptyDescription")}
                action={
                    <SofaButton
                        variant="primary"
                        icon={<IconPlus aria-hidden="true" />}
                        onClick={() => {
                            setEmEdicao(null);
                            setModalAberto(true);
                        }}
                    >
                        {t("integrations.emptyAction")}
                    </SofaButton>
                }
            />
        );
    } else {
        corpo = (
            <>
            <div
                className="poph-table poph-integrations__table"
                role="table"
                aria-label={t("integrations.tableAriaLabel")}
            >
                <div className="poph-table__header" role="row">
                    <span role="columnheader">{t("integrations.colType")}</span>
                    <span role="columnheader">{t("integrations.colStatus")}</span>
                    <span role="columnheader">{t("integrations.colConnection")}</span>
                    <span role="columnheader">{t("integrations.colSecret")}</span>
                    <span role="columnheader">{t("integrations.colConfig")}</span>
                    <span role="columnheader">{t("integrations.colVersion")}</span>
                    <span role="columnheader" className="poph-table__cell--actions">
                        {t("integrations.colActions")}
                    </span>
                </div>
                {integracoes.map((integracao) => {
                    const configuracao = integracao.configuracao;
                    const chaves =
                        configuracao && typeof configuracao === "object"
                            ? Object.keys(configuracao).length
                            : 0;
                    const classes = [
                        "poph-table__line",
                        integracao.status !== "active" ? "poph-table__line--dimmed" : "",
                    ]
                        .filter(Boolean)
                        .join(" ");
                    const nomeDoMenu = t("integrations.rowMenuLabel", {
                        kind: integracao.kind,
                        id: integracao.id,
                    });
                    return (
                        <div key={integracao.id} role="row" className={classes}>
                            <div role="cell" className="poph-integrations__identity">
                                <span className="poph-integrations__kind">{integracao.kind}</span>
                                <span className="poph-mono-muted">{integracao.id}</span>
                            </div>
                            <div role="cell">
                                <span className={`poph-status ${classeDaSituacao(integracao.status)}`}>
                                    <span className="poph-status__point" aria-hidden="true" />
                                    {rotuloDaSituacao(integracao.status)}
                                </span>
                            </div>
                            <div role="cell">
                                <span
                                    className={`poph-status ${classeDaConexao(integracao.connection_state)}`}
                                >
                                    <span className="poph-status__point" aria-hidden="true" />
                                    {connectionLabel(integracao.connection_state)}
                                </span>
                            </div>
                            <div role="cell" className="poph-table__cell--mono">
                                {integracao.tem_segredo
                                    ? t("integrations.secretSavedCell")
                                    : t("integrations.secretNoneCell")}
                            </div>
                            <div role="cell" className="poph-table__cell--mono">
                                {chaves === 0
                                    ? t("integrations.configEmptyCell")
                                    : chaves === 1
                                      ? t("integrations.configOneField")
                                      : t("integrations.configManyFields", { count: chaves })}
                            </div>
                            <div role="cell" className="poph-table__cell--mono">
                                v{integracao.version}
                            </div>
                            <div
                                role="cell"
                                className="poph-table__cell--actions poph-integrations__actions"
                            >
                                <SofaMenu
                                    label={nomeDoMenu}
                                    align="end"
                                    trigger={
                                        <SofaIconButton label={nomeDoMenu} size={28}>
                                            <IconMoreHorizontal size={16} aria-hidden="true" />
                                        </SofaIconButton>
                                    }
                                    items={itensDoMenu(integracao)}
                                />
                            </div>
                        </div>
                    );
                })}
            </div>
            <p className="poph-table__footer">
                {integracoes.length === 1
                    ? t("integrations.footerOne")
                    : t("integrations.footerMany", { count: integracoes.length })}
            </p>
            </>
        );
    }

    return (
        <>
            <div className="poph-page poph-integrations">
                <HeaderAgent
                    org={orgId}
                    nome={agente.name}
                    slug={agente.slug}
                    situacao={agente.status}
                    organizacao={nomeDaOrg}
                    descricao={t("integrations.headerDescription")}
                    acoes={
                        <>
                            <SofaButton
                                variant="secondary"
                                icon={<IconRefresh aria-hidden="true" />}
                                onClick={() => void carregarIntegracoes()}
                                loading={carregando && integracoes.length > 0}
                            >
                                {t("integrations.refresh")}
                            </SofaButton>
                            <SofaButton
                                variant="primary"
                                icon={<IconPlus aria-hidden="true" />}
                                onClick={() => {
                                    setEmEdicao(null);
                                    setModalAberto(true);
                                }}
                            >
                                {t("integrations.newIntegration")}
                            </SofaButton>
                        </>
                    }
                />

                <TabsAgent org={orgId} agente={agentId} active="integrations" nome={agente.name} model={agente.model ?? null} />

                {aviso && (
                    <p className="poph-mono-muted" style={{ margin: 0 }} role="status">
                        {aviso}
                    </p>
                )}

                <SofaBanner tone="info" title={t("integrations.pluginFormBannerTitle")}>
                    {t("integrations.pluginFormBannerBody", { whatsapp: KIND_WHATSAPP })}
                </SofaBanner>

                <section className="poph-section">
                    <div className="poph-integrations__header-section">
                        <div className="poph-integrations__title-block">
                            <h2 className="poph-section__title">
                                {t("integrations.sectionIntegrationsTitle")}
                            </h2>
                            <p className="poph-integrations__help">
                                {t("integrations.sectionIntegrationsHelp")}
                            </p>
                        </div>
                    </div>
                    {corpo}
                </section>

                <section className="poph-section">
                    <div className="poph-integrations__header-section">
                        <div className="poph-integrations__title-block">
                            <h2 className="poph-section__title">
                                {t("integrations.sectionWhatsappTitle")}
                            </h2>
                            <p className="poph-integrations__help">
                                {t("integrations.sectionWhatsappHelp")}
                            </p>
                        </div>
                        <div className="poph-integrations__actions-section">
                            <SofaButton
                                variant="subtle"
                                size="sm"
                                icon={<IconRefresh aria-hidden="true" />}
                                onClick={() => void carregarIntegracoes()}
                                loading={carregando && integracoes.length > 0}
                            >
                                {t("integrations.refresh")}
                            </SofaButton>
                        </div>
                    </div>
                    {carregando && doWhatsapp.length === 0 ? (
                        <SofaSkeleton lines={2} />
                    ) : doWhatsapp.length === 0 ? (
                        <p className="poph-mono-muted" style={{ margin: 0 }}>
                            {t("integrations.whatsappEmpty", { whatsapp: KIND_WHATSAPP })}
                        </p>
                    ) : (
                        <div className="poph-integrations__panels">
                            {doWhatsapp.map((integracao) => (
                                <PanelWhatsApp
                                    key={integracao.id}
                                    org={orgId}
                                    agente={agentId}
                                    integracao={integracao}
                                    onUpdateIntegration={atualizarIntegracao}
                                />
                            ))}
                        </div>
                    )}
                </section>

                <section className="poph-section">
                    <div className="poph-integrations__header-section">
                        <div className="poph-integrations__title-block">
                            <h2 className="poph-section__title">
                                {t("integrations.sectionSecretsTitle")}
                            </h2>
                            <p className="poph-integrations__help">
                                {t("integrations.sectionSecretsHelp")}
                            </p>
                        </div>
                    </div>
                    <div className="poph-integrations__secrets">
                        <IconKey aria-hidden="true" />
                        <span className="poph-mono-muted">
                            {t("integrations.secretsFooterNote")}
                        </span>
                    </div>
                </section>
            </div>

            <ModalIntegration
                org={orgId}
                agente={agentId}
                aberto={modalAberto}
                onClose={() => {
                    setModalAberto(false);
                    setEmEdicao(null);
                }}
                integracao={emEdicao}
                onSave={(salva) => {
                    setIntegracoes((atuais) => {
                        const existe = atuais.some((item) => item.id === salva.id);
                        return existe
                            ? atuais.map((item) => (item.id === salva.id ? salva : item))
                            : [...atuais, salva];
                    });
                    setAviso(
                        emEdicao
                            ? t("integrations.savedNotice", {
                                  kind: salva.kind,
                                  version: salva.version,
                              })
                            : t("integrations.createdNotice", { kind: salva.kind }),
                    );
                }}
            />

            <DialogExclusion
                aberto={paraExcluir !== null}
                onClose={() => setParaExcluir(null)}
                titulo={t("integrations.deleteModalTitle")}
                descricao={
                    paraExcluir
                        ? t("integrations.deleteModalDescription", { kind: paraExcluir.kind })
                        : ""
                }
                resource={paraExcluir?.kind ?? ""}
                labelResource={t("integrations.deleteModalResourceLabel")}
                labelConfirm={t("integrations.deleteModalConfirmLabel")}
                consequencia={
                    <span className="poph-mono-muted">
                        {t("integrations.deleteModalConsequence", {
                            id: paraExcluir?.id ?? "",
                        })}
                    </span>
                }
                aoConfirmar={async () => {
                    if (!paraExcluir) return;
                    await deleteIntegration(orgId, agentId, paraExcluir.id);
                }}
                aoExcluir={() => {
                    if (paraExcluir) {
                        setIntegracoes((atuais) =>
                            atuais.filter((item) => item.id !== paraExcluir.id),
                        );
                        setAviso(t("integrations.deletedNotice", { kind: paraExcluir.kind }));
                    }
                }}
            />
        </>
    );
}
