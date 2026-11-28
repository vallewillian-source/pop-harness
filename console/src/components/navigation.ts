
import type { ComponentType } from "react";
import { translate } from "../i18n";
import type { TranslationKey } from "../i18n/catalog";
import {
    IconBarChart,
    IconBot,
    IconBuilding,
    IconDatabase,
    IconFile,
    IconFolder,
    IconGear,
    IconGrid,
    IconLink,
    IconList,
    IconMessage,
    IconPhone,
    IconSliders,
    IconTag,
    IconUsers,
    IconWorkflow,
    type SofaIconProps,
} from "@sofacoding/sofa-ui";

export interface ContextNavigation {
    orgId: string | null;
    agentId: string | null;
    totalOrgs: number | null;
    totalAgents?: number | null;
}

export type StateItem = "ativo" | "emBreve";

export type NavigationGroup = "tenants" | "runtime" | "system" | "diagnostics";

const NAVIGATION_GROUP_KEYS: Record<NavigationGroup, TranslationKey> = {
    tenants: "navigation.groupTenants",
    runtime: "navigation.groupRuntime",
    system: "navigation.groupSystem",
    diagnostics: "navigation.groupDiagnostics",
};

export function navigationGroupLabel(group: NavigationGroup): string {
    return translate(NAVIGATION_GROUP_KEYS[group]);
}

export interface ItemNavigation {
    id: string;
    group: NavigationGroup | "";
    readonly caption: string;
    icon: ComponentType<SofaIconProps>;
    route: (context: ContextNavigation) => string | null;
    estado: StateItem;
    requiresOrg: boolean;
    requiresAgent: boolean;
    counter?: (context: ContextNavigation) => number | null;
    exato?: boolean;
    routeActive?: (context: ContextNavigation) => string | null;
}

export const GROUPS_NAVIGATION: readonly NavigationGroup[] = ["tenants", "runtime", "system"];

export const ITEMS_NAVIGATION: ItemNavigation[] = [
    {
        id: "organizations",
        group: "tenants",
        get caption() {
            return translate("navigation.organizations");
        },
        icon: IconBuilding,
        route: () => "/orgs",
        estado: "ativo",
        requiresOrg: false,
        requiresAgent: false,
        counter: ({ totalOrgs }) => totalOrgs,
        exato: true,
    },
    {
        id: "structure",
        group: "tenants",
        get caption() {
            return translate("navigation.structure");
        },
        icon: IconGrid,
        route: ({ orgId }) => (orgId ? `/orgs/${orgId}/structure` : null),
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: false,
    },
    {
        id: "products",
        group: "tenants",
        get caption() {
            return translate("navigation.products");
        },
        icon: IconTag,
        route: ({ orgId }) => (orgId ? `/orgs/${orgId}/products` : null),
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: false,
    },
    {
        id: "agents",
        group: "tenants",
        get caption() {
            return translate("navigation.agents");
        },
        icon: IconUsers,
        route: ({ orgId }) => (orgId ? `/orgs/${orgId}/agents` : null),
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: false,
        counter: ({ totalAgents }) => totalAgents ?? null,
    },
    {
        id: "model-jev",
        group: "runtime",
        get caption() {
            return translate("navigation.modelJev");
        },
        icon: IconBot,
        route: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/model-jev` : null,
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "turn-inspection",
        group: "runtime",
        get caption() {
            return translate("navigation.turnInspection");
        },
        icon: IconList,
        route: () => null,
        estado: "emBreve",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "test-chat",
        group: "runtime",
        get caption() {
            return translate("navigation.testChat");
        },
        icon: IconMessage,
        route: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/test-chat` : null,
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "whatsapp",
        group: "system",
        get caption() {
            return translate("navigation.whatsapp");
        },
        icon: IconPhone,
        route: () => null,
        estado: "emBreve",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "matrix",
        group: "system",
        get caption() {
            return translate("navigation.matrix");
        },
        icon: IconLink,
        route: ({ orgId }) => (orgId ? `/orgs/${orgId}/matrix` : null),
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: false,
    },
    {
        id: "plugins",
        group: "system",
        get caption() {
            return translate("navigation.plugins");
        },
        icon: IconGear,
        route: () => null,
        estado: "emBreve",
        requiresOrg: false,
        requiresAgent: false,
    },
    {
        id: "instance-health",
        group: "system",
        get caption() {
            return translate("navigation.instanceHealth");
        },
        icon: IconBarChart,
        route: ({ orgId }) => (orgId ? `/orgs/${orgId}/operacao` : null),
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: false,
    },
];

export const GROUP_DIAGNOSTIC_AGENT: NavigationGroup = "diagnostics";

export const ITEMS_COCKPIT_AGENT: ItemNavigation[] = [
    {
        id: "cockpit-instructions",
        group: "",
        get caption() {
            return translate("navigation.cockpitInstructions");
        },
        icon: IconFile,
        route: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/instructions` : null,
        routeActive: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}` : null,
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "cockpit-knowledge",
        group: "",
        get caption() {
            return translate("navigation.cockpitKnowledge");
        },
        icon: IconDatabase,
        route: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/knowledge-base` : null,
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "cockpit-files",
        group: "",
        get caption() {
            return translate("navigation.cockpitFiles");
        },
        icon: IconFolder,
        route: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/files` : null,
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "cockpit-routing",
        group: "",
        get caption() {
            return translate("navigation.cockpitRouting");
        },
        icon: IconWorkflow,
        route: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/routing` : null,
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "cockpit-model-jev",
        group: "",
        get caption() {
            return translate("navigation.cockpitModelJev");
        },
        icon: IconSliders,
        route: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/model-jev` : null,
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "cockpit-test-chat",
        group: GROUP_DIAGNOSTIC_AGENT,
        get caption() {
            return translate("navigation.cockpitTestChat");
        },
        icon: IconMessage,
        route: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/test-chat` : null,
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "cockpit-turn-inspection",
        group: GROUP_DIAGNOSTIC_AGENT,
        get caption() {
            return translate("navigation.cockpitTurnInspection");
        },
        icon: IconList,
        route: () => null,
        estado: "emBreve",
        requiresOrg: true,
        requiresAgent: true,
    },
];
