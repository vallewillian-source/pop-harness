/**
 * Painel da organização selecionada (RL-07, PRD §5.5): cabeçalho de página com nome, situação e
 * ações; resumo de configuração como lista de propriedades; atalhos para estrutura, produtos e
 * agentes como lista de links. Sem cartão: cada bloco é uma seção com título e linha fina.
 *
 * Rotas, textos de ação, modal de edição, confirmação de exclusão e erros continuam como antes.
 * Token de org não vê Editar nem Excluir (o servidor recusaria com 403).
 */

import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import {
    IconArrowLeft,
    IconPencil,
    IconRefresh,
    IconTrash,
    SofaButton,
    SofaErrorState,
    SofaIconButton,
    SofaSkeleton,
} from "@sofacoding/sofa-ui";
import { request, type Org } from "../api/client";
import { deleteOrg } from "../api/orgs";
import { DialogExclusion } from "../components/DialogExclusion";
import { ModalOrg } from "../components/ModalOrg";
import { useSession } from "../session/Session";
import { translate, useI18n } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";
import { formatDateTime } from "../i18n/format";
import "../styles/org-panel.css";

function rotuloDaSituacao(situacao: string): string {
    switch (situacao) {
        case "active":
            return translate("orgs.statusActive");
        case "inactive":
            return translate("orgs.statusInactive");
        case "archived":
            return translate("orgs.statusArchived");
        default:
            return situacao;
    }
}

function tomDaSituacao(situacao: string): string {
    return situacao === "active" ? "poph-status--active" : "poph-status--inactive";
}

function dataLonga(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "—";
    return formatDateTime(ms);
}

function retencaoDaConfig(org: Org): string {
    const config = org.config as Record<string, unknown> | undefined;
    const valor = config?.["retention_days"] ?? config?.["retencao_dias"];
    if (typeof valor === "number") {
        return valor === 0
            ? translate("orgs.retentionNever")
            : translate("orgs.retentionDays", { days: valor });
    }
    return translate("orgs.retentionInstanceDefault");
}

function Propriedade({ chave, valor, mono }: { chave: string; valor: string; mono?: boolean }) {
    return (
        <div className="poph-property">
            <dt className="poph-property__key">{chave}</dt>
            <dd
                className={
                    mono
                        ? "poph-property__value poph-property__value--mono"
                        : "poph-property__value"
                }
            >
                {valor}
            </dd>
        </div>
    );
}

function Secao({
    titulo,
    cabeca,
    children,
}: {
    titulo: string;
    cabeca?: ReactNode;
    children: ReactNode;
}) {
    return (
        <section className="poph-section">
            <div className="poph-org-panel__section-head">
                <h2 className="poph-section__title">{titulo}</h2>
                {cabeca}
            </div>
            <div className="poph-section__content">{children}</div>
        </section>
    );
}

const PROXIMAS_SECOES: TranslationKey[] = [
    "orgs.panelSectionKnowledge",
    "orgs.panelSectionProducts",
    "orgs.panelSectionIntegrations",
];

const O_QUE_A_TELA_FAZ: TranslationKey[] = [
    "orgs.panelDoesEdit",
    "orgs.panelDoesDelete",
    "orgs.panelDoesScope",
    "orgs.panelDoesSwitcher",
];

export function OrgPanel() {
    const { translate: t } = useI18n();
    const { orgId = "" } = useParams();
    const navegar = useNavigate();
    const [org, setOrg] = useState<Org | null>(null);
    const [carregando, setCarregando] = useState(true);
    const [erro, setErro] = useState<string | null>(null);
    const [detalheErro, setDetalheErro] = useState<string | null>(null);
    const [editando, setEditando] = useState(false);
    const [excluindo, setExcluindo] = useState(false);
    const { sessao } = useSession();
    // Token de org não edita nem exclui organização (o servidor responde 403).
    const podeGerenciar = sessao?.escopo !== "org";

    const carregar = useCallback(async () => {
        setCarregando(true);
        setErro(null);
        setDetalheErro(null);
        try {
            const resposta = await request<Org>(
                `/admin/v1/orgs/${encodeURIComponent(orgId)}`,
            );
            setOrg(resposta);
        } catch (falha) {
            const mensagem =
                falha instanceof Error ? falha.message : t("orgs.panelLoadFailed");
            setErro(mensagem);
            setDetalheErro(`GET /admin/v1/orgs/${orgId}`);
        } finally {
            setCarregando(false);
        }
    }, [orgId]);

    useEffect(() => {
        void carregar();
    }, [carregar]);

    if (carregando && !org) {
        return (
            <div className="poph-page poph-org-panel" aria-busy="true">
                <div className="poph-page__top">
                    <div>
                        <SofaSkeleton width={220} height={26} />
                        <div className="poph-org-panel__skeleton">
                            <SofaSkeleton width={340} height={12} />
                        </div>
                    </div>
                </div>
                <SofaSkeleton lines={4} />
            </div>
        );
    }

    if (erro && !org) {
        return (
            <div className="poph-page poph-org-panel">
                <SofaErrorState
                    title={t("orgs.panelLoadErrorTitle")}
                    description={t("orgs.panelLoadErrorDescription")}
                    details={detalheErro ?? undefined}
                    retry={{ label: t("orgs.failureTryAgain"), onClick: () => void carregar() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            onClick={() => navegar("/orgs")}
                            icon={<IconArrowLeft aria-hidden="true" />}
                        >
                            {t("orgs.panelBackToOrgs")}
                        </SofaButton>
                    }
                />
            </div>
        );
    }

    if (!org) {
        return null;
    }

    const atalhos = [
        {
            chave: "estrutura",
            titulo: t("orgs.structureShortcutTitle"),
            descricao: t("orgs.structureShortcutDescription"),
            nota: t("orgs.structureShortcutNote"),
            rotulo: t("orgs.structureShortcutAction"),
            caminho: `/orgs/${org.id}/structure`,
        },
        {
            chave: "produtos",
            titulo: t("orgs.productsShortcutTitle"),
            descricao: t("orgs.productsShortcutDescription"),
            nota: t("orgs.productsShortcutNote"),
            rotulo: t("orgs.productsShortcutAction"),
            caminho: `/orgs/${org.id}/products`,
        },
        {
            chave: "agentes",
            titulo: t("orgs.agentsShortcutTitle"),
            descricao: t("orgs.agentsShortcutDescription"),
            nota: t("orgs.agentsShortcutNote"),
            rotulo: t("orgs.agentsShortcutAction"),
            caminho: `/orgs/${org.id}/agents`,
        },
    ];

    return (
        <>
            <div className="poph-page poph-org-panel">
                <div className="poph-page__top">
                    <div className="poph-org-panel__identity">
                        <SofaIconButton label={t("orgs.panelBack")} onClick={() => navegar("/orgs")}>
                            <IconArrowLeft aria-hidden="true" />
                        </SofaIconButton>
                        <div className="poph-org-panel__text">
                            <span className="poph-org-panel__eyebrow">{t("orgs.panelEyebrow")}</span>
                            <div className="poph-org-panel__title-line">
                                <h1 className="poph-page__title">{org.name}</h1>
                                <span className={`poph-status ${tomDaSituacao(org.status)}`}>
                                    <span className="poph-status__point" aria-hidden="true" />
                                    {rotuloDaSituacao(org.status)}
                                </span>
                            </div>
                            <p className="poph-page__description">
                                {t("orgs.panelSubtitle", {
                                    slug: org.slug,
                                    version: org.version,
                                })}
                            </p>
                        </div>
                    </div>
                    <div className="poph-page__actions">
                        {podeGerenciar && (
                            <>
                                <SofaButton
                                    variant="secondary"
                                    icon={<IconPencil aria-hidden="true" />}
                                    onClick={() => setEditando(true)}
                                >
                                    {t("orgs.actionEdit")}
                                </SofaButton>
                                <SofaButton
                                    variant="danger"
                                    icon={<IconTrash aria-hidden="true" />}
                                    onClick={() => setExcluindo(true)}
                                >
                                    {t("orgs.actionDelete")}
                                </SofaButton>
                            </>
                        )}
                        <SofaButton
                            variant="secondary"
                            icon={<IconRefresh aria-hidden="true" />}
                            onClick={() => void carregar()}
                            loading={carregando}
                        >
                            {t("orgs.listRefresh")}
                        </SofaButton>
                    </div>
                </div>

                <Secao titulo={t("orgs.panelSummary")}>
                    <p className="poph-org-panel__help">{t("orgs.panelSummaryHelp")}</p>
                    <dl className="poph-properties">
                        <Propriedade chave={t("orgs.panelId")} valor={org.id} mono />
                        <Propriedade chave={t("orgs.panelSlug")} valor={org.slug} mono />
                        <Propriedade
                            chave={t("orgs.panelStatus")}
                            valor={rotuloDaSituacao(org.status)}
                        />
                        <Propriedade chave={t("orgs.panelVersion")} valor={`v${org.version}`} mono />
                        <Propriedade
                            chave={t("orgs.inspectorRetention")}
                            valor={retencaoDaConfig(org)}
                        />
                        <Propriedade
                            chave={t("orgs.inspectorCreatedAt")}
                            valor={dataLonga(org.created_at)}
                        />
                        <Propriedade
                            chave={t("orgs.panelLastChange")}
                            valor={dataLonga(org.updated_at)}
                        />
                    </dl>
                </Secao>

                <Secao titulo={t("orgs.panelShortcutsTitle")}>
                    <ul className="poph-org-panel__shortcuts">
                        {atalhos.map((atalho) => (
                            <li key={atalho.chave} className="poph-org-panel__shortcut">
                                <div className="poph-org-panel__shortcut-text">
                                    <span className="poph-org-panel__shortcut-title">
                                        {atalho.titulo}
                                    </span>
                                    <span className="poph-org-panel__help">{atalho.descricao}</span>
                                    <span className="poph-org-panel__note">{atalho.nota}</span>
                                </div>
                                <Link to={atalho.caminho} className="poph-org-panel__shortcut-link">
                                    {atalho.rotulo}
                                </Link>
                            </li>
                        ))}
                    </ul>
                </Secao>

                <Secao
                    titulo={t("orgs.panelNextSections")}
                    cabeca={<span className="poph-org-panel__tag">{t("orgs.panelInProgress")}</span>}
                >
                    <p className="poph-org-panel__help">{t("orgs.panelNextSectionsHelp")}</p>
                    <ul className="poph-org-panel__list poph-org-panel__list--simple">
                        {PROXIMAS_SECOES.map((secao) => (
                            <li key={secao} className="poph-org-panel__line">
                                <span>{t(secao)}</span>
                                <span className="poph-org-panel__tag">
                                    {t("orgs.panelComingSoon")}
                                </span>
                            </li>
                        ))}
                    </ul>
                    <p className="poph-org-panel__help">{t("orgs.panelNextSectionsHelp2")}</p>
                </Secao>

                <Secao titulo={t("orgs.panelDoesTitle")}>
                    <p className="poph-org-panel__help">{t("orgs.panelDoesHelp")}</p>
                    <ul className="poph-org-panel__list">
                        {O_QUE_A_TELA_FAZ.map((item) => (
                            <li key={item}>{t(item)}</li>
                        ))}
                    </ul>
                </Secao>
            </div>

            <ModalOrg
                aberto={editando}
                org={org}
                onClose={() => setEditando(false)}
                onSave={(alterada) => {
                    setOrg(alterada);
                    void carregar();
                }}
            />

            <DialogExclusion
                aberto={excluindo}
                onClose={() => setExcluindo(false)}
                titulo={t("orgs.deleteOrgTitle")}
                descricao={t("orgs.deleteOrgDescription", { name: org.name })}
                resource={org.slug}
                labelResource={t("orgs.deleteOrgResourceLabel")}
                labelConfirm={t("orgs.deleteOrgConfirm")}
                permiteForcar
                labelForce={t("orgs.deleteOrgForceLabel")}
                consequencia={
                    <p className="poph-org-panel__consequence">
                        {t("orgs.deleteOrgConsequence")}
                    </p>
                }
                aoConfirmar={(forcar) => deleteOrg(org.id, forcar)}
                aoExcluir={() => navegar("/orgs")}
            />
        </>
    );
}
