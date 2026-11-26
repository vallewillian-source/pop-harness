
import { useId, useState } from "react";
import {
    SofaBanner,
    SofaButton,
    SofaCheckbox,
    SofaConfirmDialog,
    SofaField,
    SofaInput,
    type SofaFieldControlProps,
} from "@sofacoding/sofa-ui";
import {
    downloadFile,
    exportOrg,
    backupFailure,
    importOrg,
    readFileText,
    LIMIT_IMPORT_BYTES,
    tableName,
    exceedsLimit,
    overwriteRefusal,
    sizeInMb,
    sampleText,
    type BackupFailure,
    type ImportReport,
    type OverwriteRefusal,
} from "../api/backup";
import { useI18n } from "../i18n";
import { formatNumber } from "../i18n/format";

const LIMIT_MB = LIMIT_IMPORT_BYTES / (1024 * 1024);

function withoutDescribedBy(control: SofaFieldControlProps) {
    const { describedBy, ...rest } = control;
    return { ...rest, "aria-describedby": describedBy };
}

function sums(report: ImportReport): { inseridas: number; atualizadas: number } {
    let inserted = 0;
    let updated = 0;
    for (const line of Object.values(report.tabelas)) {
        inserted += line.inseridas;
        updated += line.atualizadas;
    }
    return { inseridas: inserted, atualizadas: updated };
}

function numero(value: number): string {
    return formatNumber(value);
}

function NoticeFailure({
    failure,
    onTryNew,
}: {
    failure: BackupFailure;
    onTryNew?: () => void;
}) {
    const { translate: t } = useI18n();
    return (
        <div>
            <SofaBanner tone="danger" title={failure.titulo}>
                <p style={{ margin: 0 }}>{failure.texto}</p>
                {failure.versoes ? (
                    <p style={{ margin: "6px 0 0" }}>
                        {t("orgs.failureVersionLine", {
                            file: failure.versoes.arquivo,
                            instance: failure.versoes.instalacao,
                        })}
                    </p>
                ) : null}
                {failure.conflitos && failure.conflitos.itens.length > 0 ? (
                    <>
                        <p style={{ margin: "6px 0 0" }}>{t("orgs.failureConflictsFound")}</p>
                        <ul
                            className="poph-operation__conflicts"
                            aria-label={t("orgs.failureConflictsLabel")}
                        >
                            {failure.conflitos.itens.map((item) => (
                                <li key={item}>{item}</li>
                            ))}
                            {failure.conflitos.mais > 0 ? (
                                <li>
                                    {t("orgs.failureAndMore", { count: numero(failure.conflitos.mais) })}
                                </li>
                            ) : null}
                        </ul>
                    </>
                ) : null}
                {failure.podeTentarDeNovo && onTryNew ? (
                    <div style={{ marginTop: 8 }}>
                        <SofaButton variant="secondary" onClick={onTryNew}>
                            {t("orgs.failureTryAgain")}
                        </SofaButton>
                    </div>
                ) : null}
            </SofaBanner>
        </div>
    );
}

function RefusalOverwritten({
    refusal,
    importing,
    onAuthorize,
    onCancel,
}: {
    refusal: OverwriteRefusal;
    importing: boolean;
    onAuthorize: () => void;
    onCancel: () => void;
}) {
    const { translate: t } = useI18n();
    const tabelas = Object.entries(refusal.tabelas);
    return (
        <SofaBanner tone="warn" title={t("orgs.backupOverwriteTitle")}>
            <p style={{ margin: 0 }}>
                {t(
                    refusal.linhas === 1
                        ? "orgs.backupOverwriteLinesOne"
                        : "orgs.backupOverwriteLinesMany",
                    { count: numero(refusal.linhas) },
                )}
            </p>
            {tabelas.length > 0 ? (
                <table
                    className="poph-operation__table"
                    aria-label={t("orgs.backupOverwriteTableLabel")}
                    style={{ marginTop: 8 }}
                >
                    <thead>
                        <tr>
                            <th scope="col">{t("orgs.backupTableColumn")}</th>
                            <th scope="col">{t("orgs.backupRowsColumn")}</th>
                        </tr>
                    </thead>
                    <tbody>
                        {tabelas.map(([table, lines]) => (
                            <tr key={table}>
                                <td>{tableName(table)}</td>
                                <td>{numero(lines)}</td>
                            </tr>
                        ))}
                    </tbody>
                </table>
            ) : null}
            {refusal.exemplos.length > 0 ? (
                <>
                    <p style={{ margin: "6px 0 0" }}>{t("orgs.backupExamplesHeading")}</p>
                    <ul
                        className="poph-operation__conflicts"
                        aria-label={t("orgs.backupExamplesLabel")}
                    >
                        {refusal.exemplos.map((item, index) => (
                            <li key={`${index}-${item}`}>{sampleText(item)}</li>
                        ))}
                    </ul>
                </>
            ) : null}
            <div style={{ marginTop: 8, display: "flex", flexWrap: "wrap", gap: 8 }}>
                <SofaButton variant="primary" loading={importing} disabled={importing} onClick={onAuthorize}>
                    {t("orgs.backupOverwriteConfirm")}
                </SofaButton>
                <SofaButton variant="secondary" disabled={importing} onClick={onCancel}>
                    {t("common.cancel")}
                </SofaButton>
            </div>
        </SofaBanner>
    );
}

function ReportImport({ report }: { report: ImportReport }) {
    const { translate: t } = useI18n();
    const totais = sums(report);
    const lines = Object.entries(report.tabelas).filter(
        ([, line]) => line.inseridas > 0 || line.atualizadas > 0,
    );
    const indexReadyNow = report.aviso === null;

    return (
        <div className="poph-operation__report" role="status" aria-live="polite">
            <p style={{ margin: "0 0 var(--pop-spacing-sm, 6px)" }}>
                {report.org.criada
                    ? t("orgs.backupReportCreated")
                    : t("orgs.backupReportWritten")}{" "}
                {t("orgs.backupReportTotals", {
                    total: numero(report.total),
                    inserted: numero(totais.inseridas),
                    updated: numero(totais.atualizadas),
                })}
                {totais.inseridas === 0 && totais.atualizadas === 0
                    ? t("orgs.backupReportNoChange")
                    : ""}
            </p>

            {lines.length > 0 ? (
                <table className="poph-operation__table" aria-label={t("orgs.backupReportTableLabel")}>
                    <thead>
                        <tr>
                            <th scope="col">{t("orgs.backupTableColumn")}</th>
                            <th scope="col">{t("orgs.backupNewColumn")}</th>
                            <th scope="col">{t("orgs.backupUpdatedColumn")}</th>
                        </tr>
                    </thead>
                    <tbody>
                        {lines.map(([table, line]) => (
                            <tr key={table}>
                                <td>{tableName(table)}</td>
                                <td>{numero(line.inseridas)}</td>
                                <td>{numero(line.atualizadas)}</td>
                            </tr>
                        ))}
                    </tbody>
                </table>
            ) : null}

            {report.segredos_a_redigitar > 0 ? (
                <div style={{ marginTop: "var(--pop-spacing-md, 8px)" }}>
                    <SofaBanner tone="warn" title={t("orgs.backupSecretsTitle")}>
                        {report.segredos_a_redigitar === 1
                            ? t("orgs.backupSecretsOne")
                            : t("orgs.backupSecretsMany", {
                                  count: numero(report.segredos_a_redigitar),
                              })}{" "}
                        {t("orgs.backupSecretsAction")}
                    </SofaBanner>
                </div>
            ) : null}

            <div style={{ marginTop: "var(--pop-spacing-md, 8px)" }}>
                {indexReadyNow ? (
                    <SofaBanner tone="success" title={t("orgs.backupIndexReadyTitle")}>
                        {t("orgs.backupIndexReady", {
                            chunks: numero(report.indice.base_conhecimento.trechos),
                            agents: numero(report.indice.base_conhecimento.agentes),
                            productChunks: numero(report.indice.produtos.trechos),
                        })}
                    </SofaBanner>
                ) : (
                    <SofaBanner tone="warn" title={t("orgs.backupIndexPendingTitle")}>
                        {t("orgs.backupIndexPending")}
                    </SofaBanner>
                )}
            </div>
        </div>
    );
}

export type SectionBackupProps = {
    org: string;
};

export function ExportImportOrganization({ org }: SectionBackupProps) {
    const { translate: t } = useI18n();
    const idSection = useId();
    const idEntry = useId();

    const [withConversations, setConversations] = useState(false);
    const [exporting, setExporting] = useState(false);
    const [noticeExport, setNoticeExport] = useState<string | null>(null);
    const [failureExport, setFailureExport] = useState<BackupFailure | null>(null);

    const [file, setFile] = useState<File | null>(null);
    const [confirming, setConfirming] = useState(false);
    const [importing, setImporting] = useState(false);
    const [report, setReport] = useState<ImportReport | null>(null);
    const [failureImport, setFailureImport] = useState<BackupFailure | null>(null);
    const [refusal, setRefusal] = useState<OverwriteRefusal | null>(null);
    const [fileRefused, setFileRefused] = useState(false);
    const [versionEntry, setVersionEntry] = useState(0);

    async function runExport() {
        setExporting(true);
        setNoticeExport(null);
        setFailureExport(null);
        try {
            const exported = await exportOrg(org, withConversations);
            downloadFile(exported);
            setNoticeExport(t("orgs.backupDownloaded", { name: exported.nome }));
        } catch (error) {
            setFailureExport(backupFailure(error, "exportar"));
        } finally {
            setExporting(false);
        }
    }

    function chooseFile(chosen: File | null) {
        setReport(null);
        setFailureImport(null);
        setRefusal(null);
        if (!chosen) {
            setFile(null);
            setFileRefused(false);
            return;
        }
        const refused = exceedsLimit(chosen.size);
        setFileRefused(refused);
        setFile(chosen);
    }

    async function runImport(sobrescrever: boolean) {
        if (!file) return;
        setImporting(true);
        setReport(null);
        setFailureImport(null);
        setRefusal(null);
        try {
            const texto = await readFileText(file);
            const resultado = await importOrg(org, texto, sobrescrever);
            setReport(resultado);
            setFile(null);
            setFileRefused(false);
            setVersionEntry((versao) => versao + 1);
        } catch (error) {
            const refusalServer = overwriteRefusal(error);
            if (refusalServer) {
                setRefusal(refusalServer);
            } else {
                setFailureImport(backupFailure(error, "importar"));
            }
        } finally {
            setImporting(false);
            setConfirming(false);
        }
    }

    const canImport = file !== null && !fileRefused && !importing;

    return (
        <section className="poph-section" aria-labelledby={idSection}>
            <h2 id={idSection} className="poph-section__title">
                {t("orgs.backupSectionTitle")}
            </h2>
            <p className="poph-operation__help">{t("orgs.backupSectionHelp")}</p>

            <h3 className="poph-operation__subtitle">{t("orgs.backupExportHeading")}</h3>
            <div className="poph-stack" style={{ gap: "var(--pop-spacing-sm, 6px)", maxWidth: 680 }}>
                <SofaCheckbox
                    label={t("orgs.backupIncludeConversations")}
                    description={t("orgs.backupIncludeConversationsDescription")}
                    checked={withConversations}
                    disabled={exporting}
                    onChange={(evento) => setConversations(evento.target.checked)}
                />
                <div>
                    <SofaButton
                        variant="secondary"
                        loading={exporting}
                        disabled={exporting}
                        onClick={() => void runExport()}
                    >
                        {t("orgs.backupDownload")}
                    </SofaButton>
                </div>
                <div aria-live="polite">
                    {exporting ? (
                        <p className="poph-operation__loading" role="status">
                            {t("orgs.backupGenerating")}
                        </p>
                    ) : null}
                    {noticeExport && !exporting ? (
                        <p className="poph-operation__loading" role="status">
                            {noticeExport}
                        </p>
                    ) : null}
                    {failureExport ? (
                        <NoticeFailure
                            failure={failureExport}
                            onTryNew={() => void runExport()}
                        />
                    ) : null}
                </div>
            </div>

            <h3 className="poph-operation__subtitle">{t("orgs.backupImportHeading")}</h3>
            <div className="poph-stack" style={{ gap: "var(--pop-spacing-sm, 6px)", maxWidth: 680 }}>
                <p className="poph-operation__help" style={{ margin: 0 }}>
                    {t("orgs.backupImportHelp")}
                </p>
                <SofaField
                    label={t("orgs.backupFileLabel")}
                    hint={t("orgs.backupFileHint", {
                        limit: sizeInMb(LIMIT_IMPORT_BYTES),
                    })}
                    error={
                        fileRefused && file
                            ? t("orgs.backupFileTooLarge", {
                                  size: sizeInMb(file.size),
                                  limit: LIMIT_MB,
                              })
                            : undefined
                    }
                    htmlFor={idEntry}
                >
                    {(control) => (
                        <SofaInput
                            key={versionEntry}
                            {...withoutDescribedBy(control)}
                            invalid={fileRefused}
                            type="file"
                            accept=".json,application/json"
                            disabled={importing}
                            onChange={(evento) => chooseFile(evento.target.files?.[0] ?? null)}
                        />
                    )}
                </SofaField>
                {file && !fileRefused ? (
                    <p className="poph-mono-muted" style={{ margin: 0 }}>
                        {t("orgs.backupFileChosen", {
                            name: file.name,
                            size: sizeInMb(file.size),
                        })}
                    </p>
                ) : null}
                <div>
                    <SofaButton
                        variant="primary"
                        disabled={!canImport}
                        loading={importing}
                        onClick={() => setConfirming(true)}
                    >
                        {t("orgs.backupImportButton")}
                    </SofaButton>
                </div>

                <div aria-live="polite">
                    {importing ? (
                        <p className="poph-operation__loading" role="status">
                            {t("orgs.backupImporting")}
                        </p>
                    ) : null}
                    {failureImport ? (
                        <NoticeFailure
                            failure={failureImport}
                            onTryNew={failureImport.podeTentarDeNovo && file ? () => setConfirming(true) : undefined}
                        />
                    ) : null}
                    {refusal ? (
                        <RefusalOverwritten
                            refusal={refusal}
                            importing={importing}
                            onAuthorize={() => void runImport(true)}
                            onCancel={() => setRefusal(null)}
                        />
                    ) : null}
                    {report ? <ReportImport report={report} /> : null}
                </div>
            </div>

            <SofaConfirmDialog
                open={confirming}
                onOpenChange={(isOpen) => {
                    if (!isOpen && !importing) setConfirming(false);
                }}
                title={t("orgs.backupConfirmTitle")}
                description={
                    file
                        ? t("orgs.backupConfirmDescription", {
                              name: file.name,
                              size: sizeInMb(file.size),
                          })
                        : t("orgs.backupConfirmNoFile")
                }
                confirmLabel={t("orgs.backupConfirmAction")}
                cancelLabel={t("common.cancel")}
                closeLabel={t("orgs.backupConfirmClose")}
                onConfirm={() => void runImport(false)}
                loading={importing}
            />
        </section>
    );
}
