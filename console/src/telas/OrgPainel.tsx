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
import "../estilos/orgpainel.css";

function rotuloDaSituacao(situacao: string): string {
    switch (situacao) {
        case "active":
            return "Ativa";
        case "inactive":
            return "Inativa";
        case "archived":
            return "Arquivada";
        default:
            return situacao;
    }
}

function tomDaSituacao(situacao: string): string {
    return situacao === "active" ? "poph-status--ativo" : "poph-status--inativo";
}

function dataLonga(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "—";
    return new Date(ms).toLocaleString("pt-BR");
}

function retencaoDaConfig(org: Org): string {
    const config = org.config as Record<string, unknown> | undefined;
    const valor = config?.["retention_days"] ?? config?.["retencao_dias"];
    if (typeof valor === "number") {
        return valor === 0 ? "nunca apaga" : `${valor} dias`;
    }
    return "padrão da instância (30 dias)";
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

const PROXIMAS_SECOES = [
    "Base de conhecimento",
    "Produtos e preços",
    "Integrações e WhatsApp",
];

const O_QUE_A_TELA_FAZ = [
    "Edita nome, situação e retenção com controle de versão: se outro salvou antes, o que você digitou é mantido e você só revisa.",
    "Exclui com confirmação; organização com conteúdo pede confirmação extra e mostra o motivo recusado pela API.",
    "Identifica a organização e o escopo do token em uso (global ou de uma organização).",
    "O seletor do rail troca de organização sem recarregar a página.",
];

export function OrgPainel() {
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
                falha instanceof Error ? falha.message : "falha ao carregar a organização";
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
                    title="Não consegui carregar a organização"
                    description="A instância respondeu com erro. Tente de novo; se persistir, confira a saúde do servidor."
                    details={detalheErro ?? undefined}
                    retry={{ label: "Tentar de novo", onClick: () => void carregar() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            onClick={() => navegar("/orgs")}
                            icon={<IconArrowLeft aria-hidden="true" />}
                        >
                            Voltar para organizações
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
            titulo: "Departamentos e categorias",
            descricao: "Destinos de transferência e agrupamento do catálogo que os agentes usam.",
            nota: "Cria, edita e exclui departamentos; cria e edita categorias.",
            rotulo: "Abrir estrutura",
            caminho: `/orgs/${org.id}/structure`,
        },
        {
            chave: "produtos",
            titulo: "Produtos",
            descricao: "Catálogo com preço e condições que os agentes informam ao cliente.",
            nota: "Lista, filtros, criação e alteração; a exclusão pede confirmação.",
            rotulo: "Abrir produtos",
            caminho: `/orgs/${org.id}/products`,
        },
        {
            chave: "agentes",
            titulo: "Agentes de atendimento",
            descricao: "Quem responde pelo WhatsApp desta organização, com instruções por agente.",
            nota: "Lista e seleção do agente; as instruções são editadas na aba dele.",
            rotulo: "Abrir agentes",
            caminho: `/orgs/${org.id}/agents`,
        },
    ];

    return (
        <>
            <div className="poph-pagina poph-orgpainel">
                <div className="poph-pagina__topo">
                    <div className="poph-orgpainel__identidade">
                        <SofaIconButton label="Voltar" onClick={() => navegar("/orgs")}>
                            <IconArrowLeft aria-hidden="true" />
                        </SofaIconButton>
                        <div className="poph-orgpainel__texto">
                            <span className="poph-orgpainel__eyebrow">Organização</span>
                            <div className="poph-orgpainel__titulo-linha">
                                <h1 className="poph-pagina__titulo">{org.name}</h1>
                                <span className={`poph-status ${tomDaSituacao(org.status)}`}>
                                    <span className="poph-status__ponto" aria-hidden="true" />
                                    {rotuloDaSituacao(org.status)}
                                </span>
                            </div>
                            <p className="poph-pagina__descricao">
                                {org.slug} · versão {org.version} · dados isolados por
                                organização
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
                                    Editar
                                </SofaButton>
                                <SofaButton
                                    variant="danger"
                                    icon={<IconTrash aria-hidden="true" />}
                                    onClick={() => setExcluindo(true)}
                                >
                                    Excluir
                                </SofaButton>
                            </>
                        )}
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

                <Secao titulo="Resumo">
                    <p className="poph-orgpainel__ajuda">
                        Identidade e configuração que o servidor devolve para esta organização.
                    </p>
                    <dl className="poph-propriedades">
                        <Propriedade chave="Identificador" valor={org.id} mono />
                        <Propriedade chave="Slug" valor={org.slug} mono />
                        <Propriedade chave="Situação" valor={rotuloDaSituacao(org.status)} />
                        <Propriedade chave="Versão" valor={`v${org.version}`} mono />
                        <Propriedade chave="Retenção do histórico" valor={retencaoDaConfig(org)} />
                        <Propriedade chave="Criada em" valor={dataLonga(org.created_at)} />
                        <Propriedade chave="Última alteração" valor={dataLonga(org.updated_at)} />
                    </dl>
                </Secao>

                <Secao titulo="Estrutura, produtos e agentes">
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
                    titulo="Próximas seções"
                    cabeca={<span className="poph-orgpainel__etiqueta">em construção</span>}
                >
                    <p className="poph-orgpainel__ajuda">
                        A navegação desta organização cresce junto com as entregas do Console.
                    </p>
                    <ul className="poph-orgpainel__lista poph-orgpainel__lista--simples">
                        {PROXIMAS_SECOES.map((secao) => (
                            <li key={secao} className="poph-orgpainel__linha">
                                <span>{secao}</span>
                                <span className="poph-orgpainel__etiqueta">em breve</span>
                            </li>
                        ))}
                    </ul>
                    <p className="poph-orgpainel__ajuda">
                        Enquanto as telas não chegam, a API administrativa e a CLI atendem os
                        mesmos recursos.
                    </p>
                </Secao>

                <Secao titulo="O que esta tela já faz">
                    <p className="poph-orgpainel__ajuda">
                        Criar, editar e excluir organização, com o mesmo efeito da CLI.
                    </p>
                    <ul className="poph-orgpainel__lista">
                        {O_QUE_A_TELA_FAZ.map((item) => (
                            <li key={item}>{item}</li>
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
                titulo="Excluir organização?"
                descricao={`A organização ${org.name} e tudo que está dentro dela saem do sistema. Não há como desfazer.`}
                recurso={org.slug}
                rotuloRecurso="slug da organização"
                rotuloConfirmar="Excluir organização"
                permiteForcar
                rotuloForcar="Apagar também agentes, categorias, departamentos e tokens"
                consequencia={
                    <p className="poph-orgpainel__consequencia">
                        Serão apagados os agentes, as categorias, os departamentos, as conversas e os
                        tokens desta organização. A exclusão de uma organização com conteúdo exige a
                        confirmação extra abaixo.
                    </p>
                }
                aoConfirmar={(forcar) => excluirOrg(org.id, forcar)}
                aoExcluir={() => navegar("/orgs")}
            />
        </>
    );
}
