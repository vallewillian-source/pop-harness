/**
 * Lista de navegação do menu lateral.
 *
 * Para ligar um item novo: acrescente uma entrada em `ITENS_DE_NAVEGACAO` (rota, ícone, grupo e
 * estado) e registre a rota correspondente em `App.tsx` — o visual do shell não muda.
 *
 * Regras do modelo:
 * - `rota` recebe a organização e o agente abertos na URL e devolve `null` quando o item exige um
 *   contexto que não existe. Item com `rota` nula aparece desabilitado, como os itens de hoje.
 * - `estado: "emBreve"` é para item ainda sem tela: aparece desabilitado com "em breve" e nunca
 *   navega, qualquer que seja a rota devolvida.
 * - `contador` mostra um número ao lado do rótulo só quando o dado existe de verdade.
 * - `exato` marca o item só na própria rota, e não em subtelas (usado em Organizações, para não
 *   acender junto com Estrutura, Produtos e Agentes).
 */

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

/**
 * Organização, agente e total de orgs da sessão, no momento da renderização.
 * `totalDeOrgs` é `null` quando a sessão ainda não tem a lista.
 */
export interface ContextoDeNavegacao {
    /** Organização aberta na URL (`/orgs/:orgId/...`), ou `null`. */
    orgId: string | null;
    /** Agente aberto na URL (`/orgs/:orgId/agents/:agentId/...`), ou `null`. */
    agentId: string | null;
    /** Quantas organizações a sessão enxerga (a lista carregada), ou `null`. */
    totalDeOrgs: number | null;
    /**
     * Quantos agentes tem a organização aberta na URL, ou `null` (sem organização aberta, ou a
     * consulta ainda não voltou, ou falhou).
     */
    totalDeAgentes?: number | null;
}

export type EstadoDoItem = "ativo" | "emBreve";

export type NavigationGroup = "tenants" | "runtime" | "system" | "diagnostics";

const NAVIGATION_GROUP_KEYS: Record<NavigationGroup, TranslationKey> = {
    tenants: "navigation.groupTenants",
    runtime: "navigation.groupRuntime",
    system: "navigation.groupSystem",
    diagnostics: "navigation.groupDiagnostics",
};

export function navigationGroupLabel(grupo: NavigationGroup): string {
    return translate(NAVIGATION_GROUP_KEYS[grupo]);
}

export interface ItemDeNavegacao {
    id: string;
    /**
     * Título da seção do menu em que o item aparece. String vazia = o item fica no bloco inicial,
     * antes do primeiro título de seção (é assim que a prancha 03 desenha o cockpit do agente).
     */
    grupo: NavigationGroup | "";
    /** Texto visível no rail, resolvido pelo idioma ativo. */
    readonly rotulo: string;
    /** Glifo decorativo exibido à esquerda do rótulo. */
    icone: ComponentType<SofaIconProps>;
    /** Rota do item; `null` quando falta organização ou agente exigido. */
    rota: (contexto: ContextoDeNavegacao) => string | null;
    estado: EstadoDoItem;
    /** Só fica habilitado com uma organização aberta na URL. */
    exigeOrg: boolean;
    /** Só fica habilitado com um agente aberto na URL. */
    exigeAgente: boolean;
    /** Número exibido à direita do rótulo; ausente ou `null` = sem contador. */
    contador?: (contexto: ContextoDeNavegacao) => number | null;
    /** Marca como ativo só na rota exata, e não em subtelas. */
    exato?: boolean;
    /**
     * Rota extra em que o item também aparece ativo, além da própria (`rota`). É o caso de
     * Instruções & Prompts, que acende também na raiz do agente.
     */
    rotaDeAtivo?: (contexto: ContextoDeNavegacao) => string | null;
}

/** Seções do menu, na ordem em que aparecem. */
export const GRUPOS_DE_NAVEGACAO: readonly NavigationGroup[] = ["tenants", "runtime", "system"];

/** Itens do menu, na ordem do PRD do re-layout (§5.2). */
export const ITENS_DE_NAVEGACAO: ItemDeNavegacao[] = [
    {
        id: "organizations",
        grupo: "tenants",
        get rotulo() {
            return translate("navigation.organizations");
        },
        icone: IconBuilding,
        rota: () => "/orgs",
        estado: "ativo",
        exigeOrg: false,
        exigeAgente: false,
        contador: ({ totalDeOrgs }) => totalDeOrgs,
        exato: true,
    },
    {
        id: "structure",
        grupo: "tenants",
        get rotulo() {
            return translate("navigation.structure");
        },
        icone: IconGrid,
        rota: ({ orgId }) => (orgId ? `/orgs/${orgId}/structure` : null),
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: false,
    },
    {
        id: "products",
        grupo: "tenants",
        get rotulo() {
            return translate("navigation.products");
        },
        icone: IconTag,
        rota: ({ orgId }) => (orgId ? `/orgs/${orgId}/products` : null),
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: false,
    },
    {
        id: "agents",
        grupo: "tenants",
        get rotulo() {
            return translate("navigation.agents");
        },
        icone: IconUsers,
        rota: ({ orgId }) => (orgId ? `/orgs/${orgId}/agents` : null),
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: false,
        contador: ({ totalDeAgentes }) => totalDeAgentes ?? null,
    },
    {
        id: "model-jev",
        grupo: "runtime",
        get rotulo() {
            return translate("navigation.modelJev");
        },
        icone: IconBot,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/model-jev` : null,
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        id: "turn-inspection",
        grupo: "runtime",
        get rotulo() {
            return translate("navigation.turnInspection");
        },
        icone: IconList,
        rota: () => null,
        estado: "emBreve",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        id: "test-chat",
        grupo: "runtime",
        get rotulo() {
            return translate("navigation.testChat");
        },
        icone: IconMessage,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/test-chat` : null,
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        id: "whatsapp",
        grupo: "system",
        get rotulo() {
            return translate("navigation.whatsapp");
        },
        icone: IconPhone,
        rota: () => null,
        estado: "emBreve",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        id: "matrix",
        grupo: "system",
        get rotulo() {
            return translate("navigation.matrix");
        },
        icone: IconLink,
        rota: ({ orgId }) => (orgId ? `/orgs/${orgId}/matrix` : null),
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: false,
    },
    {
        id: "plugins",
        grupo: "system",
        get rotulo() {
            return translate("navigation.plugins");
        },
        icone: IconGear,
        rota: () => null,
        estado: "emBreve",
        exigeOrg: false,
        exigeAgente: false,
    },
    {
        id: "instance-health",
        grupo: "system",
        get rotulo() {
            return translate("navigation.instanceHealth");
        },
        icone: IconBarChart,
        rota: ({ orgId }) => (orgId ? `/orgs/${orgId}/operacao` : null),
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: false,
    },
];

/**
 * Título da seção de diagnóstico do cockpit do agente (prancha 03). Vai em caixa de frase, e não
 * em caixa alta como as seções de `GRUPOS_DE_NAVEGACAO`: é o que o desenho mostra.
 */
export const GRUPO_DIAGNOSTICO_DO_AGENTE: NavigationGroup = "diagnostics";

/**
 * Cockpit do agente (prancha 03): o rail da visão do agente (`/orgs/:orgId/agents/:agentId/...`),
 * na ordem do desenho. Fora dessas rotas o rail continua sendo `ITENS_DE_NAVEGACAO`.
 *
 * Todos os itens ativos apontam para rotas que já existem no `App.tsx`. "Inspeção de Turnos" ainda
 * não tem tela: fica `emBreve`, como os outros itens sem rota — nada de página inventada.
 * "Integrações" (`/integrations`) não aparece no desenho e continua alcançável pelas abas de dentro
 * das telas do agente, que este menu não substitui.
 */
export const ITENS_DO_COCKPIT_DO_AGENTE: ItemDeNavegacao[] = [
    {
        id: "cockpit-instructions",
        grupo: "",
        get rotulo() {
            return translate("navigation.cockpitInstructions");
        },
        icone: IconFile,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/instructions` : null,
        rotaDeAtivo: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}` : null,
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        id: "cockpit-knowledge",
        grupo: "",
        get rotulo() {
            return translate("navigation.cockpitKnowledge");
        },
        icone: IconDatabase,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/knowledge-base` : null,
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        id: "cockpit-files",
        grupo: "",
        get rotulo() {
            return translate("navigation.cockpitFiles");
        },
        icone: IconFolder,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/files` : null,
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        id: "cockpit-routing",
        grupo: "",
        get rotulo() {
            return translate("navigation.cockpitRouting");
        },
        icone: IconWorkflow,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/routing` : null,
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        id: "cockpit-model-jev",
        grupo: "",
        get rotulo() {
            return translate("navigation.cockpitModelJev");
        },
        icone: IconSliders,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/model-jev` : null,
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        id: "cockpit-test-chat",
        grupo: GRUPO_DIAGNOSTICO_DO_AGENTE,
        get rotulo() {
            return translate("navigation.cockpitTestChat");
        },
        icone: IconMessage,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/test-chat` : null,
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        id: "cockpit-turn-inspection",
        grupo: GRUPO_DIAGNOSTICO_DO_AGENTE,
        get rotulo() {
            return translate("navigation.cockpitTurnInspection");
        },
        icone: IconList,
        rota: () => null,
        estado: "emBreve",
        exigeOrg: true,
        exigeAgente: true,
    },
];
