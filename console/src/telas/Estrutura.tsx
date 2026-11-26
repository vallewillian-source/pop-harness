/**
 * Departamentos e categorias de produto da organização (UX_SPEC §3.3, HR-39b).
 *
 * Duas seções planas, uma embaixo da outra (RL-08: sem cartão). Cada uma tem carga, vazio e erro
 * com "tentar de novo" próprios, e uma falha numa não derruba a outra. Departamento cria, edita e
 * exclui (com a consequência nos agentes escrita antes). Categoria cria, edita e exclui: com
 * produtos ou agentes vinculados, a exclusão só sai com a confirmação extra (`force`), depois de
 * mostrar o que será apagado.
 *
 * Token de org também edita a própria estrutura (o servidor aceita); token global vê a mesma tela.
 */

import { useCallback, useEffect, useId, useState, type ReactNode } from "react";
import { useParams } from "react-router-dom";
import {
    IconBuilding,
    IconGrid,
    IconPlus,
    IconRefresh,
    SofaButton,
    SofaEmptyState,
    SofaErrorState,
    SofaSkeleton,
} from "@sofacoding/sofa-ui";
import {
    agentsUsing,
    listAgents,
    listStructure,
    type AgentSummary,
    type Category,
    type Departamento,
    type Registro,
    type StructureType,
} from "../api/structure";
import { getOrg } from "../api/orgs";
import { DialogoExcluirCategoria } from "../componentes/DialogoExcluirCategoria";
import { DialogoExcluirDepartamento } from "../componentes/DialogoExcluirDepartamento";
import { ModalEstrutura } from "../componentes/ModalEstrutura";
import { useI18n } from "../i18n";
import "../estilos/estrutura.css";

type Estado = {
    carregando: boolean;
    erro: string | null;
    itens: Registro[];
};

const ESTADO_INICIAL: Estado = { carregando: true, erro: null, itens: [] };

/** Classe do ponto de status (6 px + texto), no padrão de Orgs e OrgPainel. */
function classeDaSituacao(situacao: string): string {
    return situacao === "active" ? "poph-status--ativo" : "poph-status--inativo";
}

function rotuloDaSituacao(
    situacao: string,
    translate: ReturnType<typeof useI18n>["translate"],
): string {
    switch (situacao) {
        case "active":
            return translate("structure.statusActive");
        case "inactive":
            return translate("structure.statusInactive");
        case "archived":
            return translate("structure.statusArchived");
        default:
            return situacao;
    }
}

function textoDeAgentes(
    total: number | null,
    translate: ReturnType<typeof useI18n>["translate"],
): string {
    if (total === null) return translate("structure.agentsCountUnknown");
    return total === 1
        ? translate("structure.agentsCountOne")
        : translate("structure.agentsCountMany", { count: total });
}

type ColunaProps = {
    tipo: StructureType;
    org: string;
    /** Só departamento: quantos agentes usam cada um (`null` = ainda não se sabe). */
    contagemAgentes?: (id: string) => number | null;
    /** Chamado depois de criar, editar ou excluir, para a outra seção reler os agentes. */
    aoMudar?: () => void;
};

function SecaoEstrutura({ tipo, org, contagemAgentes, aoMudar }: ColunaProps) {
    const { translate } = useI18n();
    const [estado, setEstado] = useState<Estado>(ESTADO_INICIAL);
    const [edicao, setEdicao] = useState<{ aberto: boolean; registro: Registro | null }>({
        aberto: false,
        registro: null,
    });
    const [paraExcluir, setParaExcluir] = useState<Departamento | null>(null);
    const [categoriaParaExcluir, setCategoriaParaExcluir] = useState<Category | null>(null);
    const idDoTitulo = useId();

    const eDepartamento = tipo === "departamento";
    const titulo = eDepartamento
        ? translate("structure.departmentsTitle")
        : translate("structure.categoriesTitle");

    const carregar = useCallback(async () => {
        setEstado((atual) => ({ ...atual, carregando: true, erro: null }));
        try {
            const itens = await listStructure(tipo, org);
            setEstado({ carregando: false, erro: null, itens });
        } catch (falha) {
            const mensagem =
                falha instanceof Error
                    ? falha.message
                    : eDepartamento
                      ? translate("structure.loadDepartmentsError")
                      : translate("structure.loadCategoriesError");
            setEstado({ carregando: false, erro: mensagem, itens: [] });
        }
    }, [tipo, org, eDepartamento, translate]);

    useEffect(() => {
        void carregar();
    }, [carregar]);

    function aoSalvarRegistro(registro: Registro) {
        setEstado((atual) => {
            const existe = atual.itens.some((item) => item.id === registro.id);
            const itens = existe
                ? atual.itens.map((item) => (item.id === registro.id ? registro : item))
                : [...atual.itens, registro];
            return { ...atual, itens };
        });
        aoMudar?.();
        void carregar();
    }

    const botaoNovo = (
        <SofaButton
            variant="primary"
            icon={<IconPlus aria-hidden="true" />}
            onClick={() => setEdicao({ aberto: true, registro: null })}
        >
            {eDepartamento
                ? translate("structure.modalTitleNewDepartment")
                : translate("structure.modalTitleNewCategory")}
        </SofaButton>
    );

    let corpo: ReactNode;
    if (estado.carregando && estado.itens.length === 0) {
        corpo = (
            <div
                aria-busy="true"
                aria-label={
                    eDepartamento
                        ? translate("structure.loadingDepartments")
                        : translate("structure.loadingCategories")
                }
            >
                <SofaSkeleton lines={3} />
            </div>
        );
    } else if (estado.erro) {
        corpo = (
            <SofaErrorState
                title={
                    eDepartamento
                        ? translate("structure.errorDepartmentsTitle")
                        : translate("structure.errorCategoriesTitle")
                }
                description={translate("structure.errorDescription")}
                details={estado.erro}
                retry={{ label: translate("structure.retry"), onClick: () => void carregar() }}
            />
        );
    } else if (estado.itens.length === 0) {
        corpo = (
            <SofaEmptyState
                icon={eDepartamento ? <IconBuilding aria-hidden="true" /> : <IconGrid aria-hidden="true" />}
                title={
                    eDepartamento
                        ? translate("structure.emptyDepartmentsTitle")
                        : translate("structure.emptyCategoriesTitle")
                }
                description={
                    eDepartamento
                        ? translate("structure.emptyDepartmentsDescription")
                        : translate("structure.emptyCategoriesDescription")
                }
                action={
                    <SofaButton
                        variant="primary"
                        icon={<IconPlus aria-hidden="true" />}
                        onClick={() => setEdicao({ aberto: true, registro: null })}
                    >
                        {eDepartamento
                            ? translate("structure.createFirstDepartment")
                            : translate("structure.createFirstCategory")}
                    </SofaButton>
                }
            />
        );
    } else {
        corpo = (
            <ul
                className="poph-estrutura__lista"
                aria-label={
                    eDepartamento
                        ? translate("structure.listDepartmentsAria")
                        : translate("structure.listCategoriesAria")
                }
            >
                {estado.itens.map((item) => (
                    <li className="poph-estrutura__linha" key={item.id}>
                        <div className="poph-estrutura__texto">
                            <span className="poph-estrutura__nome">{item.name}</span>
                            <span className="poph-estrutura__descricao">
                                {item.description || translate("structure.noDescription")}
                            </span>
                            {eDepartamento && contagemAgentes && (
                                <span className="poph-mono-mudo">
                                    {textoDeAgentes(contagemAgentes(item.id), translate)}
                                </span>
                            )}
                        </div>
                        <div className="poph-estrutura__acoes">
                            <span className={`poph-status ${classeDaSituacao(item.status)}`}>
                                <span className="poph-status__ponto" aria-hidden="true" />
                                {rotuloDaSituacao(item.status, translate)}
                            </span>
                            <SofaButton
                                variant="subtle"
                                size="sm"
                                aria-label={
                                    eDepartamento
                                        ? translate("structure.editDepartmentAria", { name: item.name })
                                        : translate("structure.editCategoryAria", { name: item.name })
                                }
                                onClick={() => setEdicao({ aberto: true, registro: item })}
                            >
                                {translate("structure.edit")}
                            </SofaButton>
                            <SofaButton
                                variant="subtle"
                                size="sm"
                                aria-label={
                                    eDepartamento
                                        ? translate("structure.deleteDepartmentAria", { name: item.name })
                                        : translate("structure.deleteCategoryAria", { name: item.name })
                                }
                                onClick={() =>
                                    eDepartamento
                                        ? setParaExcluir(item as Departamento)
                                        : setCategoriaParaExcluir(item as Category)
                                }
                            >
                                {translate("structure.delete")}
                            </SofaButton>
                        </div>
                    </li>
                ))}
            </ul>
        );
    }

    return (
        <section className="poph-secao poph-estrutura__secao" aria-labelledby={idDoTitulo}>
            <div className="poph-estrutura__cabecalho">
                <div className="poph-estrutura__titulo-bloco">
                    <h2 id={idDoTitulo} className="poph-secao__titulo">
                        {titulo}
                    </h2>
                    <p className="poph-estrutura__ajuda">
                        {eDepartamento
                            ? translate("structure.departmentsHelp")
                            : translate("structure.categoriesHelp")}
                    </p>
                </div>
                <div className="poph-estrutura__acoes-secao">
                    <SofaButton
                        variant="subtle"
                        size="sm"
                        icon={<IconRefresh aria-hidden="true" />}
                        onClick={() => void carregar()}
                        loading={estado.carregando && estado.itens.length > 0}
                    >
                        {translate("structure.refresh")}
                    </SofaButton>
                    {botaoNovo}
                </div>
            </div>

            {corpo}

            <ModalEstrutura
                tipo={tipo}
                org={org}
                aberto={edicao.aberto}
                registro={edicao.registro}
                aoFechar={() => setEdicao({ aberto: false, registro: null })}
                aoSalvar={aoSalvarRegistro}
            />

            {eDepartamento && (
                <DialogoExcluirDepartamento
                    aberto={paraExcluir !== null}
                    aoFechar={() => setParaExcluir(null)}
                    org={org}
                    departamento={paraExcluir}
                    aoExcluir={() => {
                        setParaExcluir(null);
                        aoMudar?.();
                        void carregar();
                    }}
                />
            )}

            {!eDepartamento && (
                <DialogoExcluirCategoria
                    aberto={categoriaParaExcluir !== null}
                    aoFechar={() => setCategoriaParaExcluir(null)}
                    org={org}
                    categoria={categoriaParaExcluir}
                    aoExcluir={() => {
                        setCategoriaParaExcluir(null);
                        aoMudar?.();
                        void carregar();
                    }}
                />
            )}
        </section>
    );
}

export function Estrutura() {
    const { translate } = useI18n();
    const { orgId = "" } = useParams();
    const [nomeDaOrg, setNomeDaOrg] = useState<string | null>(null);
    const [agentes, setAgentes] = useState<AgentSummary[] | null>(null);

    const carregarAgentes = useCallback(async () => {
        try {
            setAgentes(await listAgents(orgId));
        } catch {
            // Sem a contagem, a linha mostra "—"; a lista de departamentos segue normal.
            setAgentes(null);
        }
    }, [orgId]);

    useEffect(() => {
        setNomeDaOrg(null);
        getOrg(orgId)
            .then((org) => setNomeDaOrg(org.name))
            .catch(() => setNomeDaOrg(null));
        void carregarAgentes();
    }, [orgId, carregarAgentes]);

    const contagem = (id: string): number | null =>
        agentes === null ? null : agentsUsing(agentes, id).length;

    return (
        <div className="poph-pagina poph-estrutura">
            <div className="poph-pagina__topo">
                <div>
                    <p className="poph-estrutura__eyebrow">
                        {nomeDaOrg
                            ? translate("structure.eyebrowOrg", { name: nomeDaOrg })
                            : translate("structure.eyebrow")}
                    </p>
                    <h1 className="poph-pagina__titulo">{translate("structure.pageTitle")}</h1>
                    <p className="poph-pagina__descricao">
                        {translate("structure.pageDescription")}
                    </p>
                </div>
                <div className="poph-pagina__acoes">
                    <SofaButton
                        variant="secondary"
                        icon={<IconRefresh aria-hidden="true" />}
                        onClick={() => void carregarAgentes()}
                    >
                        {translate("structure.refreshAgents")}
                    </SofaButton>
                </div>
            </div>

            <SecaoEstrutura
                tipo="departamento"
                org={orgId}
                contagemAgentes={contagem}
                aoMudar={() => void carregarAgentes()}
            />
            <SecaoEstrutura tipo="categoria" org={orgId} />
        </div>
    );
}
