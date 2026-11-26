
import { Fragment, useRef, useState, type ReactNode } from "react";
import { NavLink, useLocation } from "react-router-dom";
import {
    IconChevronLeft,
    IconChevronRight,
    SofaButton,
    SofaRailItem,
    SofaSidebarRail,
} from "@sofacoding/sofa-ui";
import { useSession } from "../session/Session";
import {
    GROUP_DIAGNOSTIC_AGENT,
    GROUPS_NAVIGATION,
    ITEMS_NAVIGATION,
    ITEMS_COCKPIT_AGENT,
    navigationGroupLabel,
    type ContextNavigation,
    type ItemNavigation,
} from "../components/navigation";
import { translate, useI18n } from "../i18n";
import { ChipDoCluster, type SituationCluster } from "../components/ChipCluster";
import type { Agente } from "../api/agents";
import { SelectorOrganization } from "../components/SelectorOrganization";
import { SelectorAgent, ReturnAgents } from "../components/SelectorAgent";
import { useHealth } from "../components/health";
import "../styles/navigation.css";

function reasonBlock(
    item: ItemNavigation,
    context: ContextNavigation,
    t: (key: "navigation.requiresOrg" | "navigation.requiresAgent") => string,
): string | undefined {
    if (item.requiresOrg && !context.orgId) return t("navigation.requiresOrg");
    if (item.requiresAgent && !context.agentId) return t("navigation.requiresAgent");
    return undefined;
}

function statusLabel(status: string | undefined): string {
    switch (status) {
        case "active":
            return translate("agents.agentStatusActive");
        case "inactive":
            return translate("agents.agentStatusInactive");
        case "archived":
            return translate("agents.agentStatusArchived");
        default:
            return status ?? "";
    }
}

function statusTone(status: string | undefined): string {
    return status === "active" ? "poph-rail__agent-dot--active" : "poph-rail__agent-dot--idle";
}

function AgentSubitems({ orgId, agents }: { orgId: string; agents: Agente[] }) {
    return (
        <>
            {agents.map((agent) => {
                const label = statusLabel(agent.status);
                return (
                    <NavLink
                        key={agent.id}
                        to={`/orgs/${orgId}/agents/${agent.id}/instructions`}
                        style={{ color: "inherit", textDecoration: "none" }}
                    >
                        {({ isActive }) => (
                            <SofaRailItem
                                className="poph-rail__item poph-rail__agent-row"
                                label={agent.name}
                                media={
                                    <span
                                        className={`poph-rail__agent-dot ${statusTone(agent.status)}`}
                                        title={label}
                                        aria-label={label}
                                    />
                                }
                                active={isActive}
                            />
                        )}
                    </NavLink>
                );
            })}
        </>
    );
}

export function NavItemEntry({
    item,
    context,
    compact = false,
    cockpit = false,
}: {
    item: ItemNavigation;
    context: ContextNavigation;
    compact?: boolean;
    cockpit?: boolean;
}) {
    const { translate: t } = useI18n();
    const icon: ReactNode = <item.icon aria-hidden="true" />;
    const count = item.counter?.(context) ?? null;
    const aside: ReactNode =
        count === null ? undefined : <span className="poph-rail__counter">{count}</span>;
    const route = item.route(context);
    const routeActive = item.routeActive?.(context) ?? null;
    const { pathname } = useLocation();
    const cssClass = cockpit ? "poph-rail__item poph-rail__item--cockpit" : "poph-rail__item";
    if (item.estado === "emBreve" || route === null) {
        const emBreve = item.estado === "emBreve";
        const button = (
            <SofaRailItem
                className={cssClass}
                label={item.caption}
                media={icon}
                aside={aside}
                description={emBreve ? t("navigation.comingSoon") : undefined}
                compact={compact}
                disabled
            />
        );
        const motivo = emBreve
            ? compact
                ? item.caption
                : undefined
            : reasonBlock(item, context, t);
        if (!motivo) return button;
        return (
            <div className="poph-rail__blocked" title={compact ? item.caption : motivo}>
                {button}
            </div>
        );
    }
    return (
        <NavLink
            to={route}
            end={item.exato === true}
            title={compact ? item.caption : undefined}
            style={{ color: "inherit", textDecoration: "none" }}
        >
            {({ isActive }) => (
                <SofaRailItem
                    className={cssClass}
                    label={item.caption}
                    media={icon}
                    aside={aside}
                    compact={compact}
                    active={isActive || (routeActive !== null && routeActive === pathname)}
                />
            )}
        </NavLink>
    );
}

export function Rail({
    onOpenShortcuts,
    totalAgents = null,
    agents = [],
    compact = false,
    onToggleCompact = () => {},
}: {
    onOpenShortcuts: () => void;
    totalAgents?: number | null;
    agents?: Agente[];
    compact?: boolean;
    onToggleCompact?: () => void;
}) {
    const { translate: t } = useI18n();
    const { sessao: session } = useSession();
    const { health: dados, disconnected: withoutConnection, degradada: degraded } = useHealth();
    const { pathname } = useLocation();
    const buttonTop = useRef<HTMLButtonElement>(null);
    const buttonFooter = useRef<HTMLButtonElement>(null);
    const toggle = () => {
        onToggleCompact();
        requestAnimationFrame(() => {
            (compact ? buttonFooter.current : buttonTop.current)?.focus();
        });
    };
    const partsUrl = pathname.split("/").filter(Boolean);
    const orgOpen = partsUrl[0] === "orgs" ? (partsUrl[1] ?? "") : "";
    const agentOpen = orgOpen && partsUrl[2] === "agents" ? (partsUrl[3] ?? "") : "";
    const routeAgent = Boolean(orgOpen && agentOpen);
    const [nameAgent, setNameAgent] = useState("");

    const orgs = session?.orgs ?? [];
    const selected = session?.escopo === "org" ? (orgs[0]?.id ?? "") : orgOpen;
    const listOrgs = partsUrl.length === 1 && partsUrl[0] === "orgs";
    const situationCluster: SituationCluster = withoutConnection
        ? "sem-conexao"
        : degraded
          ? "degradada"
          : dados
            ? "ok"
            : "verificando";
    const context: ContextNavigation = {
        orgId: orgOpen || null,
        agentId: agentOpen || null,
        totalOrgs: session ? orgs.length : null,
        totalAgents,
    };
    const hidesOrgsList = (item: ItemNavigation): boolean =>
        orgOpen !== "" && item.route(context) === "/orgs";

    return (
        <SofaSidebarRail
            width={compact ? 64 : 240}
            compact={compact}
            label={t("navigation.railLabel")}
            footer={
                <div className="poph-rail__base">
                    {routeAgent
                        ? !compact && (
                              <span
                                  className="poph-rail__agent"
                                  title={
                                      nameAgent
                                          ? t("navigation.agentFooter", { name: nameAgent })
                                          : undefined
                                  }
                              >
                                  {t("navigation.agentFooter", { name: nameAgent || "—" })}
                              </span>
                          )
                        : !compact && (
                              <span className="poph-rail__version">
                                  POP Harness v{dados?.version ?? "—"} · Rust
                              </span>
                          )}
                    <SofaButton
                        variant="subtle"
                        size="sm"
                        className="poph-rail__shortcuts"
                        aria-label={t("navigation.shortcutsLabel")}
                        title={t("navigation.shortcutsTitle")}
                        onClick={onOpenShortcuts}
                    >
                        ?
                    </SofaButton>
                    {!compact && (
                        <button
                            ref={buttonFooter}
                            type="button"
                            className="poph-rail__control"
                            aria-label={t("navigation.collapseMenu")}
                            title={t("navigation.collapseMenu")}
                            onClick={toggle}
                        >
                            <IconChevronLeft aria-hidden="true" />
                        </button>
                    )}
                </div>
            }
        >
            {compact && (
                <button
                    ref={buttonTop}
                    type="button"
                    className="poph-rail__control poph-rail__controle--topo"
                    aria-label={t("navigation.expandMenu")}
                    title={t("navigation.expandMenu")}
                    onClick={toggle}
                >
                    <IconChevronRight aria-hidden="true" />
                </button>
            )}
            {listOrgs ? (
                <ChipDoCluster situacao={situationCluster} compact={compact} />
            ) : routeAgent ? (
                <>
                    <ReturnAgents orgId={orgOpen} compact={compact} />
                    <SelectorAgent
                        orgId={orgOpen}
                        agentId={agentOpen}
                        compact={compact}
                        onChangeName={setNameAgent}
                    />
                </>
            ) : (
                <SelectorOrganization
                    orgs={orgs}
                    escopo={session?.escopo ?? "global"}
                    selected={selected}
                    listOrgs={listOrgs}
                    compact={compact}
                />
            )}
            {!compact && <div className="poph-rail__separator" aria-hidden="true" />}

            {routeAgent ? (
                <>
                    {ITEMS_COCKPIT_AGENT.filter((item) => item.grupo === "").map((item) => (
                        <NavItemEntry
                            key={item.id}
                            item={item}
                            context={context}
                            compact={compact}
                            cockpit
                        />
                    ))}
                    {!compact && (
                        <div className="sofa-sidebar-rail__section-title poph-rail__secao--frase">
                            {navigationGroupLabel(GROUP_DIAGNOSTIC_AGENT)}
                        </div>
                    )}
                    {ITEMS_COCKPIT_AGENT.filter((item) => item.grupo !== "").map((item) => (
                        <NavItemEntry
                            key={item.id}
                            item={item}
                            context={context}
                            compact={compact}
                            cockpit
                        />
                    ))}
                </>
            ) : (
                GROUPS_NAVIGATION.map((group) => (
                    <Fragment key={group}>
                        <div className="sofa-sidebar-rail__section-title">
                            {navigationGroupLabel(group)}
                        </div>
                        {ITEMS_NAVIGATION.filter(
                            (item) => item.grupo === group && !hidesOrgsList(item),
                        ).map((item) => (
                            <Fragment key={item.id}>
                                <NavItemEntry
                                    item={item}
                                    context={context}
                                    compact={compact}
                                />
                                {!compact &&
                                    orgOpen !== "" &&
                                    agents.length > 0 &&
                                    item.route(context) === `/orgs/${orgOpen}/agents` && (
                                        <AgentSubitems orgId={orgOpen} agents={agents} />
                                    )}
                            </Fragment>
                        ))}
                    </Fragment>
                ))
            )}
        </SofaSidebarRail>
    );
}
