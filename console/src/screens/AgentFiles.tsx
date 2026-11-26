
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

const column = createColumnHelper<LibraryFile>();

function readableSize(bytes: number): string {
    if (bytes < 1024 * 1024) {
        return `${formatNumber(Math.max(1, Math.round(bytes / 1024)))} KB`;
    }
    return sizeInMb(bytes);
}

export function AgentFiles() {
    const { translate: t, locale } = useI18n();
    const { orgId = "", agentId = "" } = useParams();
    const guard = useGuardChanges();

    const [agente, setAgent] = useState<Agente | null>(null);
    const [nameOrg, setOrgName] = useState<string | null>(null);
    const [errorLoad, setLoadError] = useState<string | null>(null);

    const [files, setFiles] = useState<LibraryFile[]>([]);
    const [loadingFiles, setLoadingFiles] = useState(true);
    const [errorFiles, setFilesError] = useState<string | null>(null);

    const [sendModal, setSendModal] = useState(false);
    const [toDelete, setToDelete] = useState<LibraryFile | null>(null);
    const [notice, setNotice] = useState<{ key: TranslationKey; name: string } | null>(null);

    const loadFiles = useCallback(async () => {
        setLoadingFiles(true);
        setFilesError(null);
        try {
            setFiles(await listFiles(orgId, agentId));
        } catch (failure) {
            setFilesError(
                failure instanceof Error ? failure.message : t("files.libraryLoadFailed"),
            );
            setFiles([]);
        } finally {
            setLoadingFiles(false);
        }
    }, [orgId, agentId, t]);

    const load = useCallback(async () => {
        setLoadError(null);
        try {
            setAgent(await getAgent(orgId, agentId));
        } catch (failure) {
            setLoadError(failure instanceof Error ? failure.message : t("files.agentLoadFailed"));
            return;
        }
        void loadFiles();
    }, [orgId, agentId, loadFiles, t]);

    useEffect(() => {
        void load();
        getOrg(orgId)
            .then((org) => setOrgName(org.name))
            .catch(() => setOrgName(null));
    }, [load, orgId]);

    const columns = useMemo(
        () => [
            column.accessor("name", {
                header: t("files.columnFile"),
                cell: (info) => {
                    const file = info.row.original;
                    return (
                        <div className="poph-files__identity">
                            <span className="poph-files__name">{file.name}</span>
                            {file.description && (
                                <span className="poph-mono-muted">{file.description}</span>
                            )}
                        </div>
                    );
                },
            }),
            column.accessor("mime", {
                header: t("files.columnType"),
                cell: (info) => <span className="poph-mono-muted">{info.getValue()}</span>,
            }),
            column.accessor("size_bytes", {
                header: t("files.columnSize"),
                cell: (info) => readableSize(info.getValue()),
            }),
            column.accessor("sha256", {
                header: "SHA-256",
                cell: (info) => (
                    <span className="poph-mono-muted" title={info.getValue()}>
                        {info.getValue().slice(0, 12)}…
                    </span>
                ),
            }),
            column.display({
                id: "acoes",
                header: t("files.columnActions"),
                cell: (info) => {
                    const file = info.row.original;
                    return (
                        <SofaIconButton
                            label={t("files.deleteFileLabel", { name: file.name })}
                            onClick={() => {
                                setNotice(null);
                                setToDelete(file);
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

    if (!agente && errorLoad) {
        return (
            <div className="poph-page">
                <SofaErrorState
                    title={t("files.loadErrorTitle")}
                    description={t("files.loadErrorDescription")}
                    details={errorLoad}
                    retry={{ label: t("files.retry"), onClick: () => void load() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            icon={<IconArrowLeft aria-hidden="true" />}
                            onClick={() => guard.tryNavigate(`/orgs/${orgId}/agents`)}
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

    let content;
    if (loadingFiles && files.length === 0) {
        content = <SofaSkeleton lines={5} />;
    } else if (errorFiles) {
        content = (
            <SofaErrorState
                title={t("files.libraryErrorTitle")}
                description={t("files.libraryErrorDescription")}
                details={errorFiles}
                retry={{ label: t("files.retry"), onClick: () => void loadFiles() }}
            />
        );
    } else if (files.length === 0) {
        content = (
            <SofaEmptyState
                icon={<IconFile aria-hidden="true" />}
                title={t("files.emptyTitle")}
                description={t("files.emptyDescription")}
                action={
                    <SofaButton
                        variant="primary"
                        icon={<IconPlus aria-hidden="true" />}
                        onClick={() => setSendModal(true)}
                    >
                        {t("files.uploadFirst")}
                    </SofaButton>
                }
            />
        );
    } else {
        content = (
            <SofaTable
                className="poph-files__table"
                label={t("files.tableLabel")}
                columns={columns}
                data={files}
                getRowId={(file) => file.id}
                isLoading={loadingFiles}
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
                organization={nameOrg}
                descricao={t("files.headerDescription")}
            />

            <TabsAgent org={orgId} agente={agentId} active="files" nome={agente.name} model={agente.model ?? null} />

            {notice && (
                <p className="poph-mono-muted" style={{ margin: 0 }} role="status">
                    {t(notice.key, { name: notice.name })}
                </p>
            )}

            <SectionAgent
                title={t("files.sectionTitle")}
                descricao={t("files.sectionDescription")}
                actions={
                    <>
                        <SofaButton
                            variant="subtle"
                            size="sm"
                            icon={<IconRefresh aria-hidden="true" />}
                            onClick={() => void loadFiles()}
                            loading={loadingFiles && files.length > 0}
                        >
                            {t("files.refresh")}
                        </SofaButton>
                        <SofaButton
                            variant="primary"
                            size="sm"
                            icon={<IconPlus aria-hidden="true" />}
                            onClick={() => setSendModal(true)}
                        >
                            {t("files.upload")}
                        </SofaButton>
                    </>
                }
            >
                {content}
            </SectionAgent>

            <ModalFile
                org={orgId}
                agente={agentId}
                opened={sendModal}
                onClose={() => setSendModal(false)}
                onSend={(file) => {
                    setFiles((current) => [file, ...current.filter((a) => a.id !== file.id)]);
                    setNotice({ key: "files.uploadedNotice", name: file.name });
                }}
            />

            <DialogExclusion
                opened={toDelete !== null}
                onClose={() => setToDelete(null)}
                title={t("files.deleteTitle")}
                descricao={
                    toDelete
                        ? t("files.deleteDescription", { name: toDelete.name })
                        : ""
                }
                resource={toDelete?.name ?? ""}
                labelResource={t("files.deleteResourceLabel")}
                labelConfirm={t("files.deleteConfirm")}
                onConfirm={async () => {
                    if (!toDelete) return;
                    await deleteFile(orgId, agentId, toDelete.id);
                }}
                onDelete={() => {
                    if (toDelete) {
                        setFiles((current) => current.filter((a) => a.id !== toDelete.id));
                        setNotice({ key: "files.removedNotice", name: toDelete.name });
                    }
                }}
            />
        </div>
    );
}
