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
            return "Ativo";
        case "inactive":
            return "Inativo";
        case "archived":
            return "Arquivado";
        default:
            return situacao;
    }
}

export function Agentes() {
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
            setErro(falha instanceof Error ? falha.message : "falha ao carregar os agentes");
        } finally {
            setCarregando(false);
        }
    }, [orgId]);

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
                header: "Agente",
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
                header: "Situação",
                size: 130,
                cell: (info) => (
                    <span className={`poph-status ${classeDaSituacao(info.getValue())}`}>
                        <span className="poph-status__ponto" aria-hidden="true" />
                        {rotuloDaSituacao(info.getValue())}
                    </span>
                ),
            }),
            coluna.accessor("default_department_id", {
                header: "Departamento padrão",
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
                header: "Versão",
                size: 90,
                cell: (info) => <span className="poph-mono-mudo">v{info.getValue()}</span>,
            }),
            coluna.display({
                id: "acoes",
                header: "Ações",
                cell: (info) => (
                    <div className="poph-agentes__acoes">
                        <SofaButton
                            variant="subtle"
                            size="sm"
                            onClick={() =>
                                navegar(`/orgs/${orgId}/agents/${info.row.original.id}/instructions`)
                            }
                        >
                            Abrir instruções
                        </SofaButton>
                    </div>
                ),
            }),
        ],
        [navegar, orgId, nomeDoDepartamento],
    );

    return (
        <div className="poph-pagina">
            <div className="poph-pagina__topo">
                <div>
                    <p className="poph-agentes__contexto">
                        {nomeDaOrg ? `Organização · ${nomeDaOrg}` : "Organização"}
                    </p>
                    <h1 className="poph-pagina__titulo">
                        Agentes de atendimento
                        {!carregando && (
                            <span className="poph-agentes__contagem">({agentes.length})</span>
                        )}
                    </h1>
                    <p className="poph-pagina__descricao">
                        Cada agente tem instruções, base, catálogo e integrações próprios. Selecione um
                        para configurar.
                    </p>
                </div>
                <div className="poph-pagina__acoes">
                    <SofaButton
                        variant="secondary"
                        icon={<IconRefresh aria-hidden="true" />}
                        onClick={() => void carregar()}
                        loading={carregando}
                    >
                        Atualizar
                    </SofaButton>
                </div>
            </div>

            {erro && !carregando ? (
                <SofaErrorState
                    title="Não consegui carregar os agentes"
                    description="A instância não respondeu à consulta. Verifique se o processo `poph serve` está no ar e tente de novo."
                    details={erro}
                    retry={{ label: "Tentar de novo", onClick: () => void carregar() }}
                />
            ) : (
                <SofaTable
                    className="poph-agentes__tabela"
                    label="Agentes de atendimento"
                    columns={colunas}
                    data={agentes}
                    getRowId={(agente) => agente.id}
                    isLoading={carregando}
                    empty={
                        <SofaEmptyState
                            icon={<IconUsers aria-hidden="true" />}
                            title="Nenhum agente nesta organização"
                            description="O cadastro de agente pela tela chega na próxima tarefa do Console. Enquanto isso, crie pela CLI (`poph agent create`) ou pela API administrativa."
                        />
                    }
                    footerMeta={({ total }) => (total === 1 ? "1 agente" : `${total} agentes`)}
                />
            )}
        </div>
    );
}
