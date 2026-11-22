/**
 * Aba Arquivos do agente (UX_SPEC §3.5, HR-43): a biblioteca que o agente pode enviar ao cliente.
 *
 * Lista, envio (até 16 MB por arquivo) e remoção. O envio e a remoção são feitos pela API e
 * refletem na linha de comando (`poph file list --offline`). A biblioteca não tem versão, então
 * não há `If-Match` nesta aba.
 *
 * Re-layout (RL-10): cabeçalho comum do agente (`CabecalhoDoAgente`), a biblioteca numa seção plana
 * (`SecaoDoAgente`) e a lista em tabela contínua. Nenhum cartão.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { useParams } from "react-router-dom";
import { createColumnHelper } from "@tanstack/react-table";
import {
    IconArrowLeft,
    IconFile,
    IconPlus,
    IconRefresh,
    IconTrash,
    SofaButton,
    SofaEmptyState,
    SofaErrorState,
    SofaIconButton,
    SofaSkeleton,
    SofaTable,
} from "@sofacoding/sofa-ui";
import { obterAgente, type Agente } from "../api/agentes";
import { excluirArquivo, listarArquivos, tamanhoEmMb, type Arquivo } from "../api/arquivos";
import { obterOrg } from "../api/orgs";
import {
    AbasDoAgente,
    CabecalhoDoAgente,
    SecaoDoAgente,
} from "../componentes/AbasDoAgente";
import { DialogoExclusao } from "../componentes/DialogoExclusao";
import { useGuardaDeAlteracoes } from "../componentes/GuardaAlteracoes";
import { ModalArquivo } from "../componentes/ModalArquivo";
import "../estilos/arquivos.css";

const coluna = createColumnHelper<Arquivo>();

/** Tamanho legível: KB abaixo de 1 MB, MB acima. */
function tamanhoLegivel(bytes: number): string {
    if (bytes < 1024 * 1024) {
        return `${Math.max(1, Math.round(bytes / 1024))} KB`;
    }
    return tamanhoEmMb(bytes);
}

export function AgenteArquivos() {
    const { orgId = "", agentId = "" } = useParams();
    const guarda = useGuardaDeAlteracoes();

    const [agente, setAgente] = useState<Agente | null>(null);
    const [nomeDaOrg, setNomeDaOrg] = useState<string | null>(null);
    const [erroDeCarga, setErroDeCarga] = useState<string | null>(null);

    const [arquivos, setArquivos] = useState<Arquivo[]>([]);
    const [carregandoArquivos, setCarregandoArquivos] = useState(true);
    const [erroArquivos, setErroArquivos] = useState<string | null>(null);

    const [modalEnvio, setModalEnvio] = useState(false);
    const [paraExcluir, setParaExcluir] = useState<Arquivo | null>(null);
    const [aviso, setAviso] = useState<string | null>(null);

    const carregarArquivos = useCallback(async () => {
        setCarregandoArquivos(true);
        setErroArquivos(null);
        try {
            setArquivos(await listarArquivos(orgId, agentId));
        } catch (falha) {
            setErroArquivos(
                falha instanceof Error ? falha.message : "falha ao carregar a biblioteca",
            );
            setArquivos([]);
        } finally {
            setCarregandoArquivos(false);
        }
    }, [orgId, agentId]);

    const carregar = useCallback(async () => {
        setErroDeCarga(null);
        try {
            setAgente(await obterAgente(orgId, agentId));
        } catch (falha) {
            setErroDeCarga(falha instanceof Error ? falha.message : "falha ao carregar o agente");
            return;
        }
        void carregarArquivos();
    }, [orgId, agentId, carregarArquivos]);

    useEffect(() => {
        void carregar();
        obterOrg(orgId)
            .then((org) => setNomeDaOrg(org.name))
            .catch(() => setNomeDaOrg(null));
    }, [carregar, orgId]);

    const colunas = useMemo(
        () => [
            coluna.accessor("name", {
                header: "Arquivo",
                cell: (info) => {
                    const arquivo = info.row.original;
                    return (
                        <div className="poph-arquivos__identidade">
                            <span className="poph-arquivos__nome">{arquivo.name}</span>
                            {arquivo.description && (
                                <span className="poph-mono-mudo">{arquivo.description}</span>
                            )}
                        </div>
                    );
                },
            }),
            coluna.accessor("mime", {
                header: "Tipo",
                cell: (info) => <span className="poph-mono-mudo">{info.getValue()}</span>,
            }),
            coluna.accessor("size_bytes", {
                header: "Tamanho",
                cell: (info) => tamanhoLegivel(info.getValue()),
            }),
            coluna.accessor("sha256", {
                header: "SHA-256",
                cell: (info) => (
                    <span className="poph-mono-mudo" title={info.getValue()}>
                        {info.getValue().slice(0, 12)}…
                    </span>
                ),
            }),
            coluna.display({
                id: "acoes",
                header: "Ações",
                cell: (info) => {
                    const arquivo = info.row.original;
                    return (
                        <SofaIconButton
                            label={`Excluir arquivo ${arquivo.name}`}
                            onClick={() => {
                                setAviso(null);
                                setParaExcluir(arquivo);
                            }}
                        >
                            <IconTrash aria-hidden="true" />
                        </SofaIconButton>
                    );
                },
            }),
        ],
        [],
    );

    if (!agente && erroDeCarga) {
        return (
            <div className="poph-pagina">
                <SofaErrorState
                    title="Não consegui carregar os arquivos do agente"
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
                <SofaSkeleton lines={6} />
            </div>
        );
    }

    let corpo;
    if (carregandoArquivos && arquivos.length === 0) {
        corpo = <SofaSkeleton lines={5} />;
    } else if (erroArquivos) {
        corpo = (
            <SofaErrorState
                title="Não consegui carregar a biblioteca"
                description="A lista de arquivos não respondeu. Tente de novo."
                details={erroArquivos}
                retry={{ label: "Tentar de novo", onClick: () => void carregarArquivos() }}
            />
        );
    } else if (arquivos.length === 0) {
        corpo = (
            <SofaEmptyState
                icon={<IconFile aria-hidden="true" />}
                title="Nenhum arquivo na biblioteca"
                description="Sem arquivo aqui, o agente não tem o que enviar ao cliente. Envie PDFs, planilhas, apresentações ou imagens de até 16 MB."
                action={
                    <SofaButton
                        variant="primary"
                        icon={<IconPlus aria-hidden="true" />}
                        onClick={() => setModalEnvio(true)}
                    >
                        Enviar primeiro arquivo
                    </SofaButton>
                }
            />
        );
    } else {
        corpo = (
            <SofaTable
                className="poph-arquivos__tabela"
                label="Biblioteca de arquivos do agente"
                columns={colunas}
                data={arquivos}
                getRowId={(arquivo) => arquivo.id}
                isLoading={carregandoArquivos}
                footerMeta={({ total }) => (total === 1 ? "1 arquivo" : `${total} arquivos`)}
            />
        );
    }

    return (
        <div className="poph-pagina">
            <CabecalhoDoAgente
                org={orgId}
                nome={agente.name}
                slug={agente.slug}
                situacao={agente.status}
                organizacao={nomeDaOrg}
                descricao="arquivos que o agente pode enviar ao cliente"
            />

            <AbasDoAgente org={orgId} agente={agentId} ativa="files" nome={agente.name} />

            {aviso && (
                <p className="poph-mono-mudo" style={{ margin: 0 }} role="status">
                    {aviso}
                </p>
            )}

            <SecaoDoAgente
                titulo="Biblioteca de arquivos"
                descricao="O agente só envia ao cliente o que está aqui. Ele escolhe pelo nome e pela descrição; nunca escolhe um caminho no disco. Limite: 16 MB por arquivo."
                acoes={
                    <>
                        <SofaButton
                            variant="subtle"
                            size="sm"
                            icon={<IconRefresh aria-hidden="true" />}
                            onClick={() => void carregarArquivos()}
                            loading={carregandoArquivos && arquivos.length > 0}
                        >
                            Atualizar
                        </SofaButton>
                        <SofaButton
                            variant="primary"
                            size="sm"
                            icon={<IconPlus aria-hidden="true" />}
                            onClick={() => setModalEnvio(true)}
                        >
                            Enviar arquivo
                        </SofaButton>
                    </>
                }
            >
                {corpo}
            </SecaoDoAgente>

            <ModalArquivo
                org={orgId}
                agente={agentId}
                aberto={modalEnvio}
                aoFechar={() => setModalEnvio(false)}
                aoEnviar={(arquivo) => {
                    setArquivos((atuais) => [arquivo, ...atuais.filter((a) => a.id !== arquivo.id)]);
                    setAviso(`Arquivo "${arquivo.name}" enviado à biblioteca.`);
                }}
            />

            <DialogoExclusao
                aberto={paraExcluir !== null}
                aoFechar={() => setParaExcluir(null)}
                titulo="Excluir arquivo?"
                descricao={
                    paraExcluir
                        ? `O agente deixa de poder enviar “${paraExcluir.name}” ao cliente. Não há como desfazer; para voltar, é preciso enviar o arquivo de novo.`
                        : ""
                }
                recurso={paraExcluir?.name ?? ""}
                rotuloRecurso="nome do arquivo"
                rotuloConfirmar="Excluir arquivo"
                aoConfirmar={async () => {
                    if (!paraExcluir) return;
                    await excluirArquivo(orgId, agentId, paraExcluir.id);
                }}
                aoExcluir={() => {
                    if (paraExcluir) {
                        setArquivos((atuais) => atuais.filter((a) => a.id !== paraExcluir.id));
                        setAviso(`Arquivo "${paraExcluir.name}" removido da biblioteca.`);
                    }
                }}
            />
        </div>
    );
}
