
import { Link, useLocation } from "react-router-dom";
import { translate, useI18n } from "../i18n";
import { useNameAgent } from "../components/nameAgent";

export type Crumb = { label: string; para?: string };

const TAB_LABEL_KEYS = {
    instructions: "shell.tabInstructions",
    "knowledge-base": "shell.tabKnowledgeBase",
    files: "shell.tabFiles",
    routing: "shell.tabRouting",
    integrations: "shell.tabIntegrations",
    "model-jev": "shell.tabModelJev",
    "test-chat": "shell.tabTestChat",
} as const;

export function trail(
    path: string,
    orgs: { id: string; name: string }[],
    nameAgent: string | null = null,
): Crumb[] {
    const parts = path.split("/").filter(Boolean);
    if (parts.length === 0) return [{ label: translate("shell.crumbConsole") }];
    if (parts[0] === "orgs") {
        const crumbs: Crumb[] = [{ label: translate("shell.crumbOrgs"), para: "/orgs" }];
        const id = parts[1];
        if (id) {
            const org = orgs.find((candidate) => candidate.id === id);
            const orgCrumb: Crumb = { label: org?.name ?? id, para: `/orgs/${id}` };
            if (parts[2] === "agents" && parts[3]) {
                const agentRoot = `/orgs/${id}/agents/${parts[3]}`;
                const agentLabel = nameAgent ?? translate("shell.crumbAgent");
                const tabKey =
                    parts[4] && parts[4] in TAB_LABEL_KEYS
                        ? TAB_LABEL_KEYS[parts[4] as keyof typeof TAB_LABEL_KEYS]
                        : undefined;
                const tabLabel = tabKey ? translate(tabKey) : undefined;
                if (!tabLabel) return [orgCrumb, { label: agentLabel }];
                return [orgCrumb, { label: agentLabel, para: agentRoot }, { label: tabLabel }];
            }
            crumbs.push(orgCrumb);
            if (parts[2] === "agents") {
                crumbs.push({ label: translate("shell.crumbAgents"), para: `/orgs/${id}/agents` });
            } else if (parts[2] === "structure") {
                crumbs.push({ label: translate("shell.crumbStructure") });
            } else if (parts[2] === "products") {
                crumbs.push({ label: translate("shell.crumbProducts") });
            } else if (parts[2] === "operacao") {
                crumbs.push({ label: translate("shell.crumbOperation") });
            }
        }
        return crumbs;
    }
    return [{ label: translate("shell.crumbOrgs"), para: "/orgs" }, { label: parts[0] ?? "" }];
}

export function Trail({ orgs }: { orgs: { id: string; name: string }[] }) {
    useI18n();
    const { pathname } = useLocation();
    const parts = pathname.split("/").filter(Boolean);
    const nameAgent = useNameAgent(parts[0] === "orgs" && parts[2] === "agents" ? parts[3] : undefined);
    const crumbs = trail(pathname, orgs, nameAgent);
    const agentTrail = parts[0] === "orgs" && parts[2] === "agents" && Boolean(parts[3]);
    return (
        <nav
            className={
                agentTrail
                    ? "poph-topbar__trail poph-topbar__trail--agent"
                    : "poph-topbar__trail"
            }
            aria-label={translate("shell.breadcrumbLabel")}
        >
            {crumbs.map((crumb, index) => {
                const last = index === crumbs.length - 1;
                const distanceCurrent = crumbs.length - 1 - index;
                return (
                    <span
                        key={`${crumb.label}-${index}`}
                        className={
                            last
                                ? "poph-topbar__step poph-topbar__step--current"
                                : "poph-topbar__step"
                        }
                        style={{ flexShrink: distanceCurrent }}
                    >
                        {index > 0 && (
                            <span className="poph-topbar__separator" aria-hidden="true">
                                /
                            </span>
                        )}
                        {crumb.para && !last ? (
                            <Link to={crumb.para} title={crumb.label}>
                                {crumb.label}
                            </Link>
                        ) : (
                            <span
                                className={last ? "poph-topbar__current" : "poph-topbar__crumb"}
                                title={last ? undefined : crumb.label}
                                aria-current={last ? "page" : undefined}
                            >
                                {crumb.label}
                            </span>
                        )}
                    </span>
                );
            })}
        </nav>
    );
}
