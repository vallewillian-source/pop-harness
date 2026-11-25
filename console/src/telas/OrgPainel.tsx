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
import { pedir, type Org } from "../api/cliente";
import { excluirOrg } from "../api/orgs";
import { DialogoExclusao } from "../componentes/DialogoExclusao";
import { ModalOrg } from "../componentes/ModalOrg";
import { useSessao } from "../sessao/Sessao";
import { translate, useI18n } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";
import { formatDateTime } from "../i18n/format";
import "../estilos/orgpainel.css";

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
    return situacao === "active" ? "poph-status--ativo" : "poph-status--inativo";
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
        <div className="poph-propriedade">
            <dt className="poph-propriedade__chave">{chave}</dt>
            <dd
                className={
                    mono
                        ? "poph-propriedade__valor poph-propriedade__valor--mono"
                        : "poph-propriedade__valor"
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
        <section className="poph-secao">
            <div className="poph-orgpainel__secao-cabeca">
                <h2 className="poph-secao__titulo">{titulo}</h2>
                {cabeca}
            </div>
            <div className="poph-secao__conteudo">{children}</div>
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

export function OrgPainel() {
    const { translate: t } = useI18n();
    const { orgId = "" } = useParams();
    const navegar = useNavigate();
    const [org, setOrg] = useState<Org | null>(null);
    const [carregando, setCarregando] = useState(true);
    const [erro, setErro] = useState<string | null>(null);
    const [detalheErro, setDetalheErro] = useState<string | null>(null);
    const [editando, setEditando] = useState(false);
    const [excluindo, setExcluindo] = useState(false);
    const { sessao } = useSessao();
    // Token de org não edita nem exclui organização (o servidor responde 403).
    const podeGerenciar = sessao?.escopo !== "org";

    const carregar = useCallback(async () => {
        setCarregando(true);
        setErro(null);
        setDetalheErro(null);
        try {
            const resposta = await pedir<Org>(
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
            <div className="poph-pagina poph-orgpainel" aria-busy="true">
                <div className="poph-pagina__topo">
                    <div>
                        <SofaSkeleton width={220} height={26} />
                        <div className="poph-orgpainel__esqueleto">
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
            <div className="poph-pagina poph-orgpainel">
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
            <div className="poph-pagina poph-orgpainel">
                <div className="poph-pagina__topo">
                    <div className="poph-orgpainel__identidade">
                        <SofaIconButton label={t("orgs.panelBack")} onClick={() => navegar("/orgs")}>
                            <IconArrowLeft aria-hidden="true" />
                        </SofaIconButton>
                        <div className="poph-orgpainel__texto">
                            <span className="poph-orgpainel__eyebrow">{t("orgs.panelEyebrow")}</span>
                            <div className="poph-orgpainel__titulo-linha">
                                <h1 className="poph-pagina__titulo">{org.name}</h1>
                                <span className={`poph-status ${tomDaSituacao(org.status)}`}>
                                    <span className="poph-status__ponto" aria-hidden="true" />
                                    {rotuloDaSituacao(org.status)}
                                </span>
                            </div>
                            <p className="poph-pagina__descricao">
                                {t("orgs.panelSubtitle", {
                                    slug: org.slug,
                                    version: org.version,
                                })}
                            </p>
                        </div>
                    </div>
                    <div className="poph-pagina__acoes">
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
                    <p className="poph-orgpainel__ajuda">{t("orgs.panelSummaryHelp")}</p>
                    <dl className="poph-propriedades">
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
                    <ul className="poph-orgpainel__atalhos">
                        {atalhos.map((atalho) => (
                            <li key={atalho.chave} className="poph-orgpainel__atalho">
                                <div className="poph-orgpainel__atalho-texto">
                                    <span className="poph-orgpainel__atalho-titulo">
                                        {atalho.titulo}
                                    </span>
                                    <span className="poph-orgpainel__ajuda">{atalho.descricao}</span>
                                    <span className="poph-orgpainel__nota">{atalho.nota}</span>
                                </div>
                                <Link to={atalho.caminho} className="poph-orgpainel__atalho-link">
                                    {atalho.rotulo}
                                </Link>
                            </li>
                        ))}
                    </ul>
                </Secao>

                <Secao
                    titulo={t("orgs.panelNextSections")}
                    cabeca={<span className="poph-orgpainel__etiqueta">{t("orgs.panelInProgress")}</span>}
                >
                    <p className="poph-orgpainel__ajuda">{t("orgs.panelNextSectionsHelp")}</p>
                    <ul className="poph-orgpainel__lista poph-orgpainel__lista--simples">
                        {PROXIMAS_SECOES.map((secao) => (
                            <li key={secao} className="poph-orgpainel__linha">
                                <span>{t(secao)}</span>
                                <span className="poph-orgpainel__etiqueta">
                                    {t("orgs.panelComingSoon")}
                                </span>
                            </li>
                        ))}
                    </ul>
                    <p className="poph-orgpainel__ajuda">{t("orgs.panelNextSectionsHelp2")}</p>
                </Secao>

                <Secao titulo={t("orgs.panelDoesTitle")}>
                    <p className="poph-orgpainel__ajuda">{t("orgs.panelDoesHelp")}</p>
                    <ul className="poph-orgpainel__lista">
                        {O_QUE_A_TELA_FAZ.map((item) => (
                            <li key={item}>{t(item)}</li>
                        ))}
                    </ul>
                </Secao>
            </div>

            <ModalOrg
                aberto={editando}
                org={org}
                aoFechar={() => setEditando(false)}
                aoSalvar={(alterada) => {
                    setOrg(alterada);
                    void carregar();
                }}
            />

            <DialogoExclusao
                aberto={excluindo}
                aoFechar={() => setExcluindo(false)}
                titulo={t("orgs.deleteOrgTitle")}
                descricao={t("orgs.deleteOrgDescription", { name: org.name })}
                recurso={org.slug}
                rotuloRecurso={t("orgs.deleteOrgResourceLabel")}
                rotuloConfirmar={t("orgs.deleteOrgConfirm")}
                permiteForcar
                rotuloForcar={t("orgs.deleteOrgForceLabel")}
                consequencia={
                    <p className="poph-orgpainel__consequencia">
                        {t("orgs.deleteOrgConsequence")}
                    </p>
                }
                aoConfirmar={(forcar) => excluirOrg(org.id, forcar)}
                aoExcluir={() => navegar("/orgs")}
            />
        </>
    );
}
