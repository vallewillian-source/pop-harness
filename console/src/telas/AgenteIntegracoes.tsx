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
            return "Ativa";
        case "inactive":
            return "Inativa";
        case "archived":
            return "Arquivada";
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
            setErro(falha instanceof Error ? falha.message : "falha ao carregar as integrações");
            setIntegracoes([]);
        } finally {
            setCarregando(false);
        }
    }, [orgId, agentId]);

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
            setErroDeCarga(falha instanceof Error ? falha.message : "falha ao carregar o agente");
            return;
        }
        void carregarIntegracoes();
    }, [orgId, agentId, carregarIntegracoes]);

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
                    title="Não consegui carregar as integrações do agente"
                    description="A instância não respondeu à leitura do agente. Tente de novo; se persistir, confira a saúde do servidor."
                    details={erroDeCarga}
                    retry={{ label: "Tentar de novo", onClick: () => void carregar() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            icon={<IconArrowLeft aria-hidden="true" />}
                            onClick={() => guarda.tentarNavegar(`/orgs/${orgId}/agents`)}
                        >
                            Voltar para agentes
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
                label: "Editar",
                icon: <IconPencil size={14} aria-hidden="true" />,
                onSelect: () => {
                    setAviso(null);
                    setEmEdicao(integracao);
                    setModalAberto(true);
                },
            },
            {
                id: "excluir",
                label: "Excluir",
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
                title="Não consegui carregar as integrações"
                description="A lista de integrações não respondeu. Tente de novo."
                details={erro}
                retry={{ label: "Tentar de novo", onClick: () => void carregarIntegracoes() }}
            />
        );
    } else if (integracoes.length === 0) {
        corpo = (
            <SofaEmptyState
                icon={<IconWorkflow aria-hidden="true" />}
                title="Nenhuma integração neste agente"
                description="Uma integração liga o agente a um canal, como o WhatsApp. Crie a primeira para o agente atender por ela."
                action={
                    <SofaButton
                        variant="primary"
                        icon={<IconPlus aria-hidden="true" />}
                        onClick={() => {
                            setEmEdicao(null);
                            setModalAberto(true);
                        }}
                    >
                        Criar primeira integração
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
                aria-label="Integrações do agente"
            >
                <div className="poph-tabela__cabecalho" role="row">
                    <span role="columnheader">Tipo</span>
                    <span role="columnheader">Situação</span>
                    <span role="columnheader">Conexão</span>
                    <span role="columnheader">Segredo</span>
                    <span role="columnheader">Configuração</span>
                    <span role="columnheader">Versão</span>
                    <span role="columnheader" className="poph-tabela__celula--acoes">
                        Ações
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
                    const nomeDoMenu = `Ações de ${integracao.kind} (${integracao.id})`;
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
                                {integracao.tem_segredo ? "gravado (não exibido)" : "sem segredo"}
                            </div>
                            <div role="cell" className="poph-tabela__celula--mono">
                                {chaves === 0
                                    ? "sem configuração"
                                    : `${chaves} ${chaves === 1 ? "campo" : "campos"} gravados`}
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
                {integracoes.length === 1 ? "1 integração" : `${integracoes.length} integrações`}
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
                    descricao="canais e plugins ligados a este agente"
                    acoes={
                        <>
                            <SofaButton
                                variant="secondary"
                                icon={<IconRefresh aria-hidden="true" />}
                                onClick={() => void carregarIntegracoes()}
                                loading={carregando && integracoes.length > 0}
                            >
                                Atualizar
                            </SofaButton>
                            <SofaButton
                                variant="primary"
                                icon={<IconPlus aria-hidden="true" />}
                                onClick={() => {
                                    setEmEdicao(null);
                                    setModalAberto(true);
                                }}
                            >
                                Nova integração
                            </SofaButton>
                        </>
                    }
                />

                <AbasDoAgente org={orgId} agente={agentId} ativa="integrations" nome={agente.name} />

                {aviso && (
                    <p className="poph-mono-mudo" style={{ margin: 0 }} role="status">
                        {aviso}
                    </p>
                )}

                <SofaBanner tone="info" title="Configuração pelo formulário do plugin">
                    O formulário de cada integração vem do manifesto que o próprio plugin declara no
                    servidor — nenhuma integração tem formulário escrito à mão aqui. Tipo sem plugin
                    declarado (como o {KIND_WHATSAPP}, que usa a Evolution) é criado só com tipo e
                    situação. O pareamento, o estado ao vivo e a desconexão do WhatsApp funcionam no
                    painel abaixo.
                </SofaBanner>

                <section className="poph-secao">
                    <div className="poph-integracoes__cabecalho-secao">
                        <div className="poph-integracoes__titulo-bloco">
                            <h2 className="poph-secao__titulo">Integrações do agente</h2>
                            <p className="poph-integracoes__ajuda">
                                O tipo identifica o canal. A situação controla se o agente atende
                                por ele; alterar usa a versão lida (duas pessoas não sobrescrevem
                                uma à outra).
                            </p>
                        </div>
                    </div>
                    {corpo}
                </section>

                <section className="poph-secao">
                    <div className="poph-integracoes__cabecalho-secao">
                        <div className="poph-integracoes__titulo-bloco">
                            <h2 className="poph-secao__titulo">WhatsApp: pareamento e conexão</h2>
                            <p className="poph-integracoes__ajuda">
                                Gere o QR, acompanhe o estado ao vivo e desconecte o aparelho. Sem
                                contagem regressiva: o WhatsApp não informa validade do QR.
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
                                Atualizar
                            </SofaButton>
                        </div>
                    </div>
                    {carregando && doWhatsapp.length === 0 ? (
                        <SofaSkeleton lines={2} />
                    ) : doWhatsapp.length === 0 ? (
                        <p className="poph-mono-mudo" style={{ margin: 0 }}>
                            Nenhuma integração do tipo {KIND_WHATSAPP}. Crie uma abaixo com o tipo{" "}
                            {KIND_WHATSAPP} para parear um aparelho aqui.
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
                            <h2 className="poph-secao__titulo">Segredos</h2>
                            <p className="poph-integracoes__ajuda">
                                Toda credencial de integração fica guardada cifrada no servidor e
                                nunca volta na resposta, no log ou nesta tela.
                            </p>
                        </div>
                    </div>
                    <div className="poph-integracoes__segredos">
                        <IconKey aria-hidden="true" />
                        <span className="poph-mono-mudo">
                            A coluna “Segredo” só diz se existe um segredo gravado, e o formulário
                            do plugin mostra no máximo quais campos já estão preenchidos. O valor
                            não é legível por esta API — é o desenho, não uma limitação da tela.
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
                            ? `Integração ${salva.kind} gravada (versão ${salva.version}).`
                            : `Integração ${salva.kind} criada.`,
                    );
                }}
            />

            <DialogoExclusao
                aberto={paraExcluir !== null}
                aoFechar={() => setParaExcluir(null)}
                titulo="Excluir integração?"
                descricao={
                    paraExcluir
                        ? `O agente deixa de atender por “${paraExcluir.kind}” e a configuração desta integração some. Não há como desfazer; para voltar, é preciso criar outra.`
                        : ""
                }
                recurso={paraExcluir?.kind ?? ""}
                rotuloRecurso="tipo da integração"
                rotuloConfirmar="Excluir integração"
                consequencia={
                    <span className="poph-mono-mudo">
                        Se houver mais de uma integração do mesmo tipo, confira o identificador{" "}
                        {paraExcluir?.id ?? ""} na lista: a confirmação é pelo tipo.
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
                        setAviso(`Integração ${paraExcluir.kind} excluída.`);
                    }
                }}
            />
        </>
    );
}
