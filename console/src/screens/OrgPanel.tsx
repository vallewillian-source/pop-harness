
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
import { request, type Org } from "../api/client";
import { deleteOrg } from "../api/orgs";
import { DialogExclusion } from "../components/DialogExclusion";
import { ModalOrg } from "../components/ModalOrg";
import { useSession } from "../session/Session";
import { translate, useI18n } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";
import { formatDateTime } from "../i18n/format";
import "../styles/org-panel.css";

function labelStatus(situacao: string): string {
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

function toneStatus(situacao: string): string {
    return situacao === "active" ? "poph-status--active" : "poph-status--inactive";
}

function longDate(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "—";
    return formatDateTime(ms);
}

function configRetention(org: Org): string {
    const config = org.config as Record<string, unknown> | undefined;
    const value = config?.["retention_days"] ?? config?.["retencao_dias"];
    if (typeof value === "number") {
        return value === 0
            ? translate("orgs.retentionNever")
            : translate("orgs.retentionDays", { days: value });
    }
    return translate("orgs.retentionInstanceDefault");
}

function Property({ chave, value, mono }: { chave: string; value: string; mono?: boolean }) {
    return (
        <div className="poph-property">
            <dt className="poph-property__key">{chave}</dt>
            <dd
                className={
                    mono
                        ? "poph-property__value poph-property__value--mono"
                        : "poph-property__value"
                }
            >
                {value}
            </dd>
        </div>
    );
}

function Section({
    title,
    cabeca,
    children,
}: {
    title: string;
    cabeca?: ReactNode;
    children: ReactNode;
}) {
    return (
        <section className="poph-section">
            <div className="poph-org-panel__section-head">
                <h2 className="poph-section__title">{title}</h2>
                {cabeca}
            </div>
            <div className="poph-section__content">{children}</div>
        </section>
    );
}

const NEXT_SECTIONS: TranslationKey[] = [
    "orgs.panelSectionKnowledge",
    "orgs.panelSectionProducts",
    "orgs.panelSectionIntegrations",
];

const WHAT_THE_SCREEN_DOES: TranslationKey[] = [
    "orgs.panelDoesEdit",
    "orgs.panelDoesDelete",
    "orgs.panelDoesScope",
    "orgs.panelDoesSwitcher",
];

export function OrgPanel() {
    const { translate: t } = useI18n();
    const { orgId = "" } = useParams();
    const navigate = useNavigate();
    const [org, setOrg] = useState<Org | null>(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);
    const [detailError, setErrorDetail] = useState<string | null>(null);
    const [editing, setEditing] = useState(false);
    const [deleting, setDeleting] = useState(false);
    const { session } = useSession();
    const canManage = session?.scope !== "org";

    const load = useCallback(async () => {
        setLoading(true);
        setError(null);
        setErrorDetail(null);
        try {
            const response = await request<Org>(
                `/admin/v1/orgs/${encodeURIComponent(orgId)}`,
            );
            setOrg(response);
        } catch (failure) {
            const mensagem =
                failure instanceof Error ? failure.message : t("orgs.panelLoadFailed");
            setError(mensagem);
            setErrorDetail(`GET /admin/v1/orgs/${orgId}`);
        } finally {
            setLoading(false);
        }
    }, [orgId]);

    useEffect(() => {
        void load();
    }, [load]);

    if (loading && !org) {
        return (
            <div className="poph-page poph-org-panel" aria-busy="true">
                <div className="poph-page__top">
                    <div>
                        <SofaSkeleton width={220} height={26} />
                        <div className="poph-org-panel__skeleton">
                            <SofaSkeleton width={340} height={12} />
                        </div>
                    </div>
                </div>
                <SofaSkeleton lines={4} />
            </div>
        );
    }

    if (error && !org) {
        return (
            <div className="poph-page poph-org-panel">
                <SofaErrorState
                    title={t("orgs.panelLoadErrorTitle")}
                    description={t("orgs.panelLoadErrorDescription")}
                    details={detailError ?? undefined}
                    retry={{ label: t("orgs.failureTryAgain"), onClick: () => void load() }}
                    action={
                        <SofaButton
                            variant="subtle"
                            onClick={() => navigate("/orgs")}
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

    const shortcuts = [
        {
            chave: "estrutura",
            title: t("orgs.structureShortcutTitle"),
            descricao: t("orgs.structureShortcutDescription"),
            note: t("orgs.structureShortcutNote"),
            label: t("orgs.structureShortcutAction"),
            path: `/orgs/${org.id}/structure`,
        },
        {
            chave: "produtos",
            title: t("orgs.productsShortcutTitle"),
            descricao: t("orgs.productsShortcutDescription"),
            note: t("orgs.productsShortcutNote"),
            label: t("orgs.productsShortcutAction"),
            path: `/orgs/${org.id}/products`,
        },
        {
            chave: "agentes",
            title: t("orgs.agentsShortcutTitle"),
            descricao: t("orgs.agentsShortcutDescription"),
            note: t("orgs.agentsShortcutNote"),
            label: t("orgs.agentsShortcutAction"),
            path: `/orgs/${org.id}/agents`,
        },
    ];

    return (
        <>
            <div className="poph-page poph-org-panel">
                <div className="poph-page__top">
                    <div className="poph-org-panel__identity">
                        <SofaIconButton label={t("orgs.panelBack")} onClick={() => navigate("/orgs")}>
                            <IconArrowLeft aria-hidden="true" />
                        </SofaIconButton>
                        <div className="poph-org-panel__text">
                            <span className="poph-org-panel__eyebrow">{t("orgs.panelEyebrow")}</span>
                            <div className="poph-org-panel__title-line">
                                <h1 className="poph-page__title">{org.name}</h1>
                                <span className={`poph-status ${toneStatus(org.status)}`}>
                                    <span className="poph-status__point" aria-hidden="true" />
                                    {labelStatus(org.status)}
                                </span>
                            </div>
                            <p className="poph-page__description">
                                {t("orgs.panelSubtitle", {
                                    slug: org.slug,
                                    version: org.version,
                                })}
                            </p>
                        </div>
                    </div>
                    <div className="poph-page__actions">
                        {canManage && (
                            <>
                                <SofaButton
                                    variant="secondary"
                                    icon={<IconPencil aria-hidden="true" />}
                                    onClick={() => setEditing(true)}
                                >
                                    {t("orgs.actionEdit")}
                                </SofaButton>
                                <SofaButton
                                    variant="danger"
                                    icon={<IconTrash aria-hidden="true" />}
                                    onClick={() => setDeleting(true)}
                                >
                                    {t("orgs.actionDelete")}
                                </SofaButton>
                            </>
                        )}
                        <SofaButton
                            variant="secondary"
                            icon={<IconRefresh aria-hidden="true" />}
                            onClick={() => void load()}
                            loading={loading}
                        >
                            {t("orgs.listRefresh")}
                        </SofaButton>
                    </div>
                </div>

                <Section title={t("orgs.panelSummary")}>
                    <p className="poph-org-panel__help">{t("orgs.panelSummaryHelp")}</p>
                    <dl className="poph-properties">
                        <Property chave={t("orgs.panelId")} value={org.id} mono />
                        <Property chave={t("orgs.panelSlug")} value={org.slug} mono />
                        <Property
                            chave={t("orgs.panelStatus")}
                            value={labelStatus(org.status)}
                        />
                        <Property chave={t("orgs.panelVersion")} value={`v${org.version}`} mono />
                        <Property
                            chave={t("orgs.inspectorRetention")}
                            value={configRetention(org)}
                        />
                        <Property
                            chave={t("orgs.inspectorCreatedAt")}
                            value={longDate(org.created_at)}
                        />
                        <Property
                            chave={t("orgs.panelLastChange")}
                            value={longDate(org.updated_at)}
                        />
                    </dl>
                </Section>

                <Section title={t("orgs.panelShortcutsTitle")}>
                    <ul className="poph-org-panel__shortcuts">
                        {shortcuts.map((shortcut) => (
                            <li key={shortcut.chave} className="poph-org-panel__shortcut">
                                <div className="poph-org-panel__shortcut-text">
                                    <span className="poph-org-panel__shortcut-title">
                                        {shortcut.title}
                                    </span>
                                    <span className="poph-org-panel__help">{shortcut.descricao}</span>
                                    <span className="poph-org-panel__note">{shortcut.note}</span>
                                </div>
                                <Link to={shortcut.path} className="poph-org-panel__shortcut-link">
                                    {shortcut.label}
                                </Link>
                            </li>
                        ))}
                    </ul>
                </Section>

                <Section
                    title={t("orgs.panelNextSections")}
                    cabeca={<span className="poph-org-panel__tag">{t("orgs.panelInProgress")}</span>}
                >
                    <p className="poph-org-panel__help">{t("orgs.panelNextSectionsHelp")}</p>
                    <ul className="poph-org-panel__list poph-org-panel__list--simple">
                        {NEXT_SECTIONS.map((section) => (
                            <li key={section} className="poph-org-panel__line">
                                <span>{t(section)}</span>
                                <span className="poph-org-panel__tag">
                                    {t("orgs.panelComingSoon")}
                                </span>
                            </li>
                        ))}
                    </ul>
                    <p className="poph-org-panel__help">{t("orgs.panelNextSectionsHelp2")}</p>
                </Section>

                <Section title={t("orgs.panelDoesTitle")}>
                    <p className="poph-org-panel__help">{t("orgs.panelDoesHelp")}</p>
                    <ul className="poph-org-panel__list">
                        {WHAT_THE_SCREEN_DOES.map((item) => (
                            <li key={item}>{t(item)}</li>
                        ))}
                    </ul>
                </Section>
            </div>

            <ModalOrg
                opened={editing}
                org={org}
                onClose={() => setEditing(false)}
                onSave={(changed) => {
                    setOrg(changed);
                    void load();
                }}
            />

            <DialogExclusion
                opened={deleting}
                onClose={() => setDeleting(false)}
                title={t("orgs.deleteOrgTitle")}
                descricao={t("orgs.deleteOrgDescription", { name: org.name })}
                resource={org.slug}
                labelResource={t("orgs.deleteOrgResourceLabel")}
                labelConfirm={t("orgs.deleteOrgConfirm")}
                allowForce
                labelForce={t("orgs.deleteOrgForceLabel")}
                consequencia={
                    <p className="poph-org-panel__consequence">
                        {t("orgs.deleteOrgConsequence")}
                    </p>
                }
                onConfirm={(forcar) => deleteOrg(org.id, forcar)}
                onDelete={() => navigate("/orgs")}
            />
        </>
    );
}
