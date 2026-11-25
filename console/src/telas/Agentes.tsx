/**
 * Agentes de atendimento da organização (UX_SPEC §3.4, HR-40) no layout do re-layout (RL-09).
 *
 * Esta rodada só **seleciona** o agente: a lista mostra quem existe e abre as abas do agente
 * (hoje, Instruções). Criar, editar e excluir agente é tarefa seguinte; por isso a tela não
 * oferece essas ações e diz onde elas estão hoje (CLI/API).
 *
 * Apresentação: cabeçalho de página e tabela contínua de linha de 56 px (PRD do re-layout §5.5),
 * sem cartão. Nenhuma chamada nova à API: `listarAgentes`, `obterOrg` e os departamentos são as
 * mesmas três consultas de antes.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { createColumnHelper } from "@tanstack/react-table";
import {
    IconRefresh,
    IconUsers,
    SofaButton,
    SofaEmptyState,
    SofaErrorState,
    SofaTable,
} from "@sofacoding/sofa-ui";
import { listarAgentes, type Agente } from "../api/agentes";
import { listarEstrutura, type Departamento } from "../api/estrutura";
import { obterOrg } from "../api/orgs";
import { translate, useI18n } from "../i18n";
import "../estilos/agentes.css";

const coluna = createColumnHelper<Agente>();

function classeDaSituacao(situacao: string): string {
    switch (situacao) {
        case "active":
            return "poph-status--ativo";
        case "inactive":
        case "archived":
            return "poph-status--inativo";
        default:
            return "poph-status--inativo";
    }
}

function rotuloDaSituacao(situacao: string): string {
    switch (situacao) {
        case "active":
            return translate("agents.agentStatusActive");
        case "inactive":
            return translate("agents.agentStatusInactive");
        case "archived":
            return translate("agents.agentStatusArchived");
        default:
            return situacao;
    }
}

export function Agentes() {
    const { locale, translate: t } = useI18n();
    const { orgId = "" } = useParams();
    const navegar = useNavigate();
    const [agentes, setAgentes] = useState<Agente[]>([]);
    const [departamentos, setDepartamentos] = useState<Departamento[]>([]);
    const [nomeDaOrg, setNomeDaOrg] = useState<string | null>(null);
    const [carregando, setCarregando] = useState(true);
    const [erro, setErro] = useState<string | null>(null);

    const carregar = useCallback(async () => {
        setCarregando(true);
        setErro(null);
        try {
            const lista = await listarAgentes(orgId);
            setAgentes(lista);
        } catch (falha) {
            setErro(falha instanceof Error ? falha.message : t("agents.loadFailed"));
        } finally {
            setCarregando(false);
        }
    }, [orgId, t]);

    useEffect(() => {
        void carregar();
        obterOrg(orgId)
            .then((org) => setNomeDaOrg(org.name))
            .catch(() => setNomeDaOrg(null));
        // Sem os departamentos a coluna mostra "—"; a lista de agentes não depende disso.
        listarEstrutura("departamento", orgId)
            .then((itens) => setDepartamentos(itens as Departamento[]))
            .catch(() => setDepartamentos([]));
    }, [carregar, orgId]);

    const nomeDoDepartamento = useMemo(() => {
        const mapa = new Map(departamentos.map((departamento) => [departamento.id, departamento.name]));
        return (id: string | null): string => {
            if (!id) return "—";
            return mapa.get(id) ?? id;
        };
    }, [departamentos]);

    const colunas = useMemo(
        () => [
            coluna.accessor("name", {
                header: t("agents.listAgent"),
                cell: (info) => {
                    const agente = info.row.original;
                    const atenuado = agente.status !== "active";
                    return (
                        <div className="poph-agentes__identidade">
                            <span className={atenuado ? "poph-agentes__nome--atenuado" : "poph-agentes__nome"}>
                                {info.getValue()}
                            </span>
                            <span className="poph-mono-mudo">{agente.slug}</span>
                        </div>
                    );
                },
            }),
            coluna.accessor("status", {
                header: t("agents.listStatus"),
                size: 130,
                cell: (info) => (
                    <span className={`poph-status ${classeDaSituacao(info.getValue())}`}>
                        <span className="poph-status__ponto" aria-hidden="true" />
                        {rotuloDaSituacao(info.getValue())}
                    </span>
                ),
            }),
            coluna.accessor("default_department_id", {
                header: t("agents.listDefaultDepartment"),
                cell: (info) => {
                    const atenuado = info.row.original.status !== "active";
                    return (
                        <span className={atenuado ? "poph-agentes__celula--atenuada" : undefined}>
                            {nomeDoDepartamento(info.getValue())}
                        </span>
                    );
                },
            }),
            coluna.accessor("version", {
                header: t("agents.listVersion"),
                size: 90,
                cell: (info) => <span className="poph-mono-mudo">v{info.getValue()}</span>,
            }),
            coluna.display({
                id: "acoes",
                header: t("agents.listActions"),
                cell: (info) => (
                    <div className="poph-agentes__acoes">
                        <SofaButton
                            variant="subtle"
                            size="sm"
                            onClick={() =>
                                navegar(`/orgs/${orgId}/agents/${info.row.original.id}/instructions`)
                            }
                        >
                            {t("agents.openInstructions")}
                        </SofaButton>
                    </div>
                ),
            }),
        ],
        [navegar, orgId, nomeDoDepartamento, t, locale],
    );

    return (
        <div className="poph-pagina">
            <div className="poph-pagina__topo">
                <div>
                    <p className="poph-agentes__contexto">
                        {nomeDaOrg
                            ? t("agents.orgContextWithName", { name: nomeDaOrg })
                            : t("agents.orgContext")}
                    </p>
                    <h1 className="poph-pagina__titulo">
                        {t("agents.pageTitle")}
                        {!carregando && (
                            <span className="poph-agentes__contagem">({agentes.length})</span>
                        )}
                    </h1>
                    <p className="poph-pagina__descricao">
                        {t("agents.pageDescription")}
                    </p>
                </div>
                <div className="poph-pagina__acoes">
                    <SofaButton
                        variant="secondary"
                        icon={<IconRefresh aria-hidden="true" />}
                        onClick={() => void carregar()}
                        loading={carregando}
                    >
                        {t("agents.refresh")}
                    </SofaButton>
                </div>
            </div>

            {erro && !carregando ? (
                <SofaErrorState
                    title={t("agents.loadErrorTitle")}
                    description={t("agents.loadErrorDescription")}
                    details={erro}
                    retry={{ label: t("agents.retry"), onClick: () => void carregar() }}
                />
            ) : (
                <SofaTable
                    className="poph-agentes__tabela"
                    label={t("agents.pageTitle")}
                    columns={colunas}
                    data={agentes}
                    getRowId={(agente) => agente.id}
                    isLoading={carregando}
                    empty={
                        <SofaEmptyState
                            icon={<IconUsers aria-hidden="true" />}
                            title={t("agents.emptyTitle")}
                            description={t("agents.emptyDescription")}
                        />
                    }
                    footerMeta={({ total }) =>
                        total === 1
                            ? t("agents.countOne")
                            : t("agents.countMany", { total })
                    }
                />
            )}
        </div>
    );
}
