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
import { obterAgente, type Agente } from "../api/agentes";
import {
    KIND_WHATSAPP,
    excluirIntegracao,
    listarIntegracoes,
    rotuloDaConexao,
    type Integracao,
} from "../api/integracoes";
import { obterOrg } from "../api/orgs";
import { AbasDoAgente, CabecalhoDoAgente } from "../componentes/AbasDoAgente";
import { DialogoExclusao } from "../componentes/DialogoExclusao";
import { useGuardaDeAlteracoes } from "../componentes/GuardaAlteracoes";
import { ModalIntegracao } from "../componentes/ModalIntegracao";
import { PainelWhatsApp } from "../componentes/PainelWhatsApp";
import { translate, useI18n } from "../i18n";
import "../estilos/integracoes.css";

/** Classe do ponto de status (`.poph-status` de `relayout.css`) para a situação da integração. */
function classeDaSituacao(situacao: string): string {
    switch (situacao) {
        case "active":
            return "poph-status--ativo";
        case "inactive":
            return "poph-status--atencao";
        default:
            return "poph-status--inativo";
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
            return "poph-status--ativo";
        case "awaiting_scan":
            return "poph-status--atencao";
        case "disconnected":
            return "poph-status--erro";
        default:
            return "poph-status--inativo";
    }
}

export function AgenteIntegracoes() {
    const { translate: t } = useI18n();
    const { orgId = "", agentId = "" } = useParams();
    const guarda = useGuardaDeAlteracoes();

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
            setIntegracoes(await listarIntegracoes(orgId, agentId));
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
            setAgente(await obterAgente(orgId, agentId));
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
        obterOrg(orgId)
            .then((org) => setNomeDaOrg(org.name))
            .catch(() => setNomeDaOrg(null));
    }, [carregar, orgId]);

    const doWhatsapp = useMemo(
        () => integracoes.filter((integracao) => integracao.kind === KIND_WHATSAPP),
        [integracoes],
    );

    if (!agente && erroDeCarga) {
        return (
            <div className="poph-pagina">
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
            <div className="poph-pagina" aria-busy="true">
                <div className="poph-pagina__topo">
                    <div>
                        <SofaSkeleton width={260} height={26} />
                        <div className="poph-integracoes__esqueleto-linha">
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
                className="poph-tabela poph-integracoes__tabela"
                role="table"
                aria-label={t("integrations.tableAriaLabel")}
            >
                <div className="poph-tabela__cabecalho" role="row">
                    <span role="columnheader">{t("integrations.colType")}</span>
                    <span role="columnheader">{t("integrations.colStatus")}</span>
                    <span role="columnheader">{t("integrations.colConnection")}</span>
                    <span role="columnheader">{t("integrations.colSecret")}</span>
                    <span role="columnheader">{t("integrations.colConfig")}</span>
                    <span role="columnheader">{t("integrations.colVersion")}</span>
                    <span role="columnheader" className="poph-tabela__celula--acoes">
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
                        "poph-tabela__linha",
                        integracao.status !== "active" ? "poph-tabela__linha--atenuada" : "",
                    ]
                        .filter(Boolean)
                        .join(" ");
                    const nomeDoMenu = t("integrations.rowMenuLabel", {
                        kind: integracao.kind,
                        id: integracao.id,
                    });
                    return (
                        <div key={integracao.id} role="row" className={classes}>
                            <div role="cell" className="poph-integracoes__identidade">
                                <span className="poph-integracoes__tipo">{integracao.kind}</span>
                                <span className="poph-mono-mudo">{integracao.id}</span>
                            </div>
                            <div role="cell">
                                <span className={`poph-status ${classeDaSituacao(integracao.status)}`}>
                                    <span className="poph-status__ponto" aria-hidden="true" />
                                    {rotuloDaSituacao(integracao.status)}
                                </span>
                            </div>
                            <div role="cell">
                                <span
                                    className={`poph-status ${classeDaConexao(integracao.connection_state)}`}
                                >
                                    <span className="poph-status__ponto" aria-hidden="true" />
                                    {rotuloDaConexao(integracao.connection_state)}
                                </span>
                            </div>
                            <div role="cell" className="poph-tabela__celula--mono">
                                {integracao.tem_segredo
                                    ? t("integrations.secretSavedCell")
                                    : t("integrations.secretNoneCell")}
                            </div>
                            <div role="cell" className="poph-tabela__celula--mono">
                                {chaves === 0
                                    ? t("integrations.configEmptyCell")
                                    : chaves === 1
                                      ? t("integrations.configOneField")
                                      : t("integrations.configManyFields", { count: chaves })}
                            </div>
                            <div role="cell" className="poph-tabela__celula--mono">
                                v{integracao.version}
                            </div>
                            <div
                                role="cell"
                                className="poph-tabela__celula--acoes poph-integracoes__acoes"
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
            <p className="poph-tabela__rodape">
                {integracoes.length === 1
                    ? t("integrations.footerOne")
                    : t("integrations.footerMany", { count: integracoes.length })}
            </p>
            </>
        );
    }

    return (
        <>
            <div className="poph-pagina poph-integracoes">
                <CabecalhoDoAgente
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

                <AbasDoAgente org={orgId} agente={agentId} ativa="integrations" nome={agente.name} model={agente.model ?? null} />

                {aviso && (
                    <p className="poph-mono-mudo" style={{ margin: 0 }} role="status">
                        {aviso}
                    </p>
                )}

                <SofaBanner tone="info" title={t("integrations.pluginFormBannerTitle")}>
                    {t("integrations.pluginFormBannerBody", { whatsapp: KIND_WHATSAPP })}
                </SofaBanner>

                <section className="poph-secao">
                    <div className="poph-integracoes__cabecalho-secao">
                        <div className="poph-integracoes__titulo-bloco">
                            <h2 className="poph-secao__titulo">
                                {t("integrations.sectionIntegrationsTitle")}
                            </h2>
                            <p className="poph-integracoes__ajuda">
                                {t("integrations.sectionIntegrationsHelp")}
                            </p>
                        </div>
                    </div>
                    {corpo}
                </section>

                <section className="poph-secao">
                    <div className="poph-integracoes__cabecalho-secao">
                        <div className="poph-integracoes__titulo-bloco">
                            <h2 className="poph-secao__titulo">
                                {t("integrations.sectionWhatsappTitle")}
                            </h2>
                            <p className="poph-integracoes__ajuda">
                                {t("integrations.sectionWhatsappHelp")}
                            </p>
                        </div>
                        <div className="poph-integracoes__acoes-secao">
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
                        <p className="poph-mono-mudo" style={{ margin: 0 }}>
                            {t("integrations.whatsappEmpty", { whatsapp: KIND_WHATSAPP })}
                        </p>
                    ) : (
                        <div className="poph-integracoes__paineis">
                            {doWhatsapp.map((integracao) => (
                                <PainelWhatsApp
                                    key={integracao.id}
                                    org={orgId}
                                    agente={agentId}
                                    integracao={integracao}
                                    aoAtualizarIntegracao={atualizarIntegracao}
                                />
                            ))}
                        </div>
                    )}
                </section>

                <section className="poph-secao">
                    <div className="poph-integracoes__cabecalho-secao">
                        <div className="poph-integracoes__titulo-bloco">
                            <h2 className="poph-secao__titulo">
                                {t("integrations.sectionSecretsTitle")}
                            </h2>
                            <p className="poph-integracoes__ajuda">
                                {t("integrations.sectionSecretsHelp")}
                            </p>
                        </div>
                    </div>
                    <div className="poph-integracoes__segredos">
                        <IconKey aria-hidden="true" />
                        <span className="poph-mono-mudo">
                            {t("integrations.secretsFooterNote")}
                        </span>
                    </div>
                </section>
            </div>

            <ModalIntegracao
                org={orgId}
                agente={agentId}
                aberto={modalAberto}
                aoFechar={() => {
                    setModalAberto(false);
                    setEmEdicao(null);
                }}
                integracao={emEdicao}
                aoSalvar={(salva) => {
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

            <DialogoExclusao
                aberto={paraExcluir !== null}
                aoFechar={() => setParaExcluir(null)}
                titulo={t("integrations.deleteModalTitle")}
                descricao={
                    paraExcluir
                        ? t("integrations.deleteModalDescription", { kind: paraExcluir.kind })
                        : ""
                }
                recurso={paraExcluir?.kind ?? ""}
                rotuloRecurso={t("integrations.deleteModalResourceLabel")}
                rotuloConfirmar={t("integrations.deleteModalConfirmLabel")}
                consequencia={
                    <span className="poph-mono-mudo">
                        {t("integrations.deleteModalConsequence", {
                            id: paraExcluir?.id ?? "",
                        })}
                    </span>
                }
                aoConfirmar={async () => {
                    if (!paraExcluir) return;
                    await excluirIntegracao(orgId, agentId, paraExcluir.id);
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
