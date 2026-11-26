
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
    totalDeOrgs: number | null;
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
    grupo: NavigationGroup | "";
    readonly rotulo: string;
    icon: ComponentType<SofaIconProps>;
    rota: (context: ContextNavigation) => string | null;
    estado: StateItem;
    requiresOrg: boolean;
    requiresAgent: boolean;
    contador?: (context: ContextNavigation) => number | null;
    exato?: boolean;
    routeActive?: (context: ContextNavigation) => string | null;
}

export const GROUPS_NAVIGATION: readonly NavigationGroup[] = ["tenants", "runtime", "system"];

export const ITEMS_NAVIGATION: ItemNavigation[] = [
    {
        id: "organizations",
        grupo: "tenants",
        get rotulo() {
            return translate("navigation.organizations");
        },
        icon: IconBuilding,
        rota: () => "/orgs",
        estado: "ativo",
        requiresOrg: false,
        requiresAgent: false,
        contador: ({ totalDeOrgs: totalOrgs }) => totalOrgs,
        exato: true,
    },
    {
        id: "structure",
        grupo: "tenants",
        get rotulo() {
            return translate("navigation.structure");
        },
        icon: IconGrid,
        rota: ({ orgId }) => (orgId ? `/orgs/${orgId}/structure` : null),
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: false,
    },
    {
        id: "products",
        grupo: "tenants",
        get rotulo() {
            return translate("navigation.products");
        },
        icon: IconTag,
        rota: ({ orgId }) => (orgId ? `/orgs/${orgId}/products` : null),
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: false,
    },
    {
        id: "agents",
        grupo: "tenants",
        get rotulo() {
            return translate("navigation.agents");
        },
        icon: IconUsers,
        rota: ({ orgId }) => (orgId ? `/orgs/${orgId}/agents` : null),
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: false,
        contador: ({ totalAgents }) => totalAgents ?? null,
    },
    {
        id: "model-jev",
        grupo: "runtime",
        get rotulo() {
            return translate("navigation.modelJev");
        },
        icon: IconBot,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/model-jev` : null,
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "turn-inspection",
        grupo: "runtime",
        get rotulo() {
            return translate("navigation.turnInspection");
        },
        icon: IconList,
        rota: () => null,
        estado: "emBreve",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "test-chat",
        grupo: "runtime",
        get rotulo() {
            return translate("navigation.testChat");
        },
        icon: IconMessage,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/test-chat` : null,
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "whatsapp",
        grupo: "system",
        get rotulo() {
            return translate("navigation.whatsapp");
        },
        icon: IconPhone,
        rota: () => null,
        estado: "emBreve",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "matrix",
        grupo: "system",
        get rotulo() {
            return translate("navigation.matrix");
        },
        icon: IconLink,
        rota: ({ orgId }) => (orgId ? `/orgs/${orgId}/matrix` : null),
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: false,
    },
    {
        id: "plugins",
        grupo: "system",
        get rotulo() {
            return translate("navigation.plugins");
        },
        icon: IconGear,
        rota: () => null,
        estado: "emBreve",
        requiresOrg: false,
        requiresAgent: false,
    },
    {
        id: "instance-health",
        grupo: "system",
        get rotulo() {
            return translate("navigation.instanceHealth");
        },
        icon: IconBarChart,
        rota: ({ orgId }) => (orgId ? `/orgs/${orgId}/operacao` : null),
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: false,
    },
];

export const GROUP_DIAGNOSTIC_AGENT: NavigationGroup = "diagnostics";

export const ITEMS_COCKPIT_AGENT: ItemNavigation[] = [
    {
        id: "cockpit-instructions",
        grupo: "",
        get rotulo() {
            return translate("navigation.cockpitInstructions");
        },
        icon: IconFile,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/instructions` : null,
        routeActive: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}` : null,
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "cockpit-knowledge",
        grupo: "",
        get rotulo() {
            return translate("navigation.cockpitKnowledge");
        },
        icon: IconDatabase,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/knowledge-base` : null,
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "cockpit-files",
        grupo: "",
        get rotulo() {
            return translate("navigation.cockpitFiles");
        },
        icon: IconFolder,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/files` : null,
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "cockpit-routing",
        grupo: "",
        get rotulo() {
            return translate("navigation.cockpitRouting");
        },
        icon: IconWorkflow,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/routing` : null,
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "cockpit-model-jev",
        grupo: "",
        get rotulo() {
            return translate("navigation.cockpitModelJev");
        },
        icon: IconSliders,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/model-jev` : null,
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "cockpit-test-chat",
        grupo: GROUP_DIAGNOSTIC_AGENT,
        get rotulo() {
            return translate("navigation.cockpitTestChat");
        },
        icon: IconMessage,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/test-chat` : null,
        estado: "ativo",
        requiresOrg: true,
        requiresAgent: true,
    },
    {
        id: "cockpit-turn-inspection",
        grupo: GROUP_DIAGNOSTIC_AGENT,
        get rotulo() {
            return translate("navigation.cockpitTurnInspection");
        },
        icon: IconList,
        rota: () => null,
        estado: "emBreve",
        requiresOrg: true,
        requiresAgent: true,
    },
];
