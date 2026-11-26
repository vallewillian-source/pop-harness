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
import { getAgent, type Agente } from "../api/agents";
import { deleteFile, listFiles, sizeInMb, type LibraryFile } from "../api/files";
import { getOrg } from "../api/orgs";
import {
    TabsAgent,
    HeaderAgent,
    SectionAgent,
} from "../components/TabsAgent";
import { DialogExclusion } from "../components/DialogExclusion";
import { useGuardChanges } from "../components/GuardChanges";
import { ModalFile } from "../components/ModalFile";
import { formatNumber } from "../i18n/format";
import { useI18n } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";
import "../styles/files.css";

const coluna = createColumnHelper<LibraryFile>();

/** Tamanho legível: KB abaixo de 1 MB, MB acima. */
function tamanhoLegivel(bytes: number): string {
    if (bytes < 1024 * 1024) {
        return `${formatNumber(Math.max(1, Math.round(bytes / 1024)))} KB`;
    }
    return sizeInMb(bytes);
}

export function AgentFiles() {
    const { translate: t, locale } = useI18n();
    const { orgId = "", agentId = "" } = useParams();
    const guarda = useGuardChanges();

    const [agente, setAgente] = useState<Agente | null>(null);
    const [nomeDaOrg, setNomeDaOrg] = useState<string | null>(null);
    const [erroDeCarga, setErroDeCarga] = useState<string | null>(null);

    const [arquivos, setArquivos] = useState<LibraryFile[]>([]);
    const [carregandoArquivos, setCarregandoArquivos] = useState(true);
    const [erroArquivos, setErroArquivos] = useState<string | null>(null);

    const [modalEnvio, setModalEnvio] = useState(false);
    const [paraExcluir, setParaExcluir] = useState<LibraryFile | null>(null);
    const [aviso, setAviso] = useState<{ key: TranslationKey; name: string } | null>(null);

    const carregarArquivos = useCallback(async () => {
        setCarregandoArquivos(true);
        setErroArquivos(null);
        try {
            setArquivos(await listFiles(orgId, agentId));
        } catch (falha) {
            setErroArquivos(
                falha instanceof Error ? falha.message : t("files.libraryLoadFailed"),
            );
            setArquivos([]);
        } finally {
            setCarregandoArquivos(false);
        }
    }, [orgId, agentId, t]);

    const carregar = useCallback(async () => {
        setErroDeCarga(null);
        try {
            setAgente(await getAgent(orgId, agentId));
        } catch (falha) {
            setErroDeCarga(falha instanceof Error ? falha.message : t("files.agentLoadFailed"));
            return;
        }
        void carregarArquivos();
    }, [orgId, agentId, carregarArquivos, t]);

    useEffect(() => {
        void carregar();
        getOrg(orgId)
            .then((org) => setNomeDaOrg(org.name))
            .catch(() => setNomeDaOrg(null));
    }, [carregar, orgId]);

    const colunas = useMemo(
        () => [
            coluna.accessor("name", {
                header: t("files.columnFile"),
                cell: (info) => {
                    const arquivo = info.row.original;
                    return (
                        <div className="poph-files__identity">
                            <span className="poph-files__name">{arquivo.name}</span>
                            {arquivo.description && (
                                <span className="poph-mono-muted">{arquivo.description}</span>
                            )}
                        </div>
                    );
                },
            }),
            coluna.accessor("mime", {
                header: t("files.columnType"),
                cell: (info) => <span className="poph-mono-muted">{info.getValue()}</span>,
            }),
            coluna.accessor("size_bytes", {
                header: t("files.columnSize"),
                cell: (info) => tamanhoLegivel(info.getValue()),
            }),
            coluna.accessor("sha256", {
                header: "SHA-256",
                cell: (info) => (
                    <span className="poph-mono-muted" title={info.getValue()}>
                        {info.getValue().slice(0, 12)}…
                    </span>
                ),
            }),
            coluna.display({
                id: "acoes",
                header: t("files.columnActions"),
                cell: (info) => {
                    const arquivo = info.row.original;
                    return (
                        <SofaIconButton
                            label={t("files.deleteFileLabel", { name: arquivo.name })}
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
        [locale, t],
    );

    if (!agente && erroDeCarga) {
        return (
            <div className="poph-page">
                <SofaErrorState
                    title={t("files.loadErrorTitle")}
                    description={t("files.loadErrorDescription")}
                    details={erroDeCarga}
                    retry={{ label: t("files.retry"), onClick: () => void carregar() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            icon={<IconArrowLeft aria-hidden="true" />}
                            onClick={() => guarda.tentarNavegar(`/orgs/${orgId}/agents`)}
                        >
                            {t("files.backToAgents")}
                        </SofaButton>
                    }
                />
            </div>
        );
    }

    if (!agente) {
        return (
            <div className="poph-page" aria-busy="true">
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
                title={t("files.libraryErrorTitle")}
                description={t("files.libraryErrorDescription")}
                details={erroArquivos}
                retry={{ label: t("files.retry"), onClick: () => void carregarArquivos() }}
            />
        );
    } else if (arquivos.length === 0) {
        corpo = (
            <SofaEmptyState
                icon={<IconFile aria-hidden="true" />}
                title={t("files.emptyTitle")}
                description={t("files.emptyDescription")}
                action={
                    <SofaButton
                        variant="primary"
                        icon={<IconPlus aria-hidden="true" />}
                        onClick={() => setModalEnvio(true)}
                    >
                        {t("files.uploadFirst")}
                    </SofaButton>
                }
            />
        );
    } else {
        corpo = (
            <SofaTable
                className="poph-files__table"
                label={t("files.tableLabel")}
                columns={colunas}
                data={arquivos}
                getRowId={(arquivo) => arquivo.id}
                isLoading={carregandoArquivos}
                footerMeta={({ total }) =>
                    total === 1
                        ? t("files.countOne")
                        : t("files.countMany", { total })
                }
            />
        );
    }

    return (
        <div className="poph-page">
            <HeaderAgent
                org={orgId}
                nome={agente.name}
                slug={agente.slug}
                situacao={agente.status}
                organizacao={nomeDaOrg}
                descricao={t("files.headerDescription")}
            />

            <TabsAgent org={orgId} agente={agentId} active="files" nome={agente.name} model={agente.model ?? null} />

            {aviso && (
                <p className="poph-mono-muted" style={{ margin: 0 }} role="status">
                    {t(aviso.key, { name: aviso.name })}
                </p>
            )}

            <SectionAgent
                titulo={t("files.sectionTitle")}
                descricao={t("files.sectionDescription")}
                acoes={
                    <>
                        <SofaButton
                            variant="subtle"
                            size="sm"
                            icon={<IconRefresh aria-hidden="true" />}
                            onClick={() => void carregarArquivos()}
                            loading={carregandoArquivos && arquivos.length > 0}
                        >
                            {t("files.refresh")}
                        </SofaButton>
                        <SofaButton
                            variant="primary"
                            size="sm"
                            icon={<IconPlus aria-hidden="true" />}
                            onClick={() => setModalEnvio(true)}
                        >
                            {t("files.upload")}
                        </SofaButton>
                    </>
                }
            >
                {corpo}
            </SectionAgent>

            <ModalFile
                org={orgId}
                agente={agentId}
                aberto={modalEnvio}
                onClose={() => setModalEnvio(false)}
                onSend={(arquivo) => {
                    setArquivos((atuais) => [arquivo, ...atuais.filter((a) => a.id !== arquivo.id)]);
                    setAviso({ key: "files.uploadedNotice", name: arquivo.name });
                }}
            />

            <DialogExclusion
                aberto={paraExcluir !== null}
                onClose={() => setParaExcluir(null)}
                titulo={t("files.deleteTitle")}
                descricao={
                    paraExcluir
                        ? t("files.deleteDescription", { name: paraExcluir.name })
                        : ""
                }
                resource={paraExcluir?.name ?? ""}
                labelResource={t("files.deleteResourceLabel")}
                labelConfirm={t("files.deleteConfirm")}
                aoConfirmar={async () => {
                    if (!paraExcluir) return;
                    await deleteFile(orgId, agentId, paraExcluir.id);
                }}
                aoExcluir={() => {
                    if (paraExcluir) {
                        setArquivos((atuais) => atuais.filter((a) => a.id !== paraExcluir.id));
                        setAviso({ key: "files.removedNotice", name: paraExcluir.name });
                    }
                }}
            />
        </div>
    );
}
