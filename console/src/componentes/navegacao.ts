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

export interface ItemDeNavegacao {
    /**
     * Título da seção do menu em que o item aparece. String vazia = o item fica no bloco inicial,
     * antes do primeiro título de seção (é assim que a prancha 03 desenha o cockpit do agente).
     */
    grupo: string;
    /** Texto visível no rail. */
    rotulo: string;
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

/** Seções do menu, na ordem em que aparecem. Os nomes já vêm em caixa alta, como no desenho. */
export const GRUPOS_DE_NAVEGACAO = ["TENANTS E RECURSOS", "RUNTIME E MOTOR", "SISTEMA"] as const;

/** Itens do menu, na ordem do PRD do re-layout (§5.2). */
export const ITENS_DE_NAVEGACAO: ItemDeNavegacao[] = [
    {
        grupo: "TENANTS E RECURSOS",
        rotulo: "Organizações",
        icone: IconBuilding,
        rota: () => "/orgs",
        estado: "ativo",
        exigeOrg: false,
        exigeAgente: false,
        contador: ({ totalDeOrgs }) => totalDeOrgs,
        exato: true,
    },
    {
        grupo: "TENANTS E RECURSOS",
        rotulo: "Estrutura",
        icone: IconGrid,
        rota: ({ orgId }) => (orgId ? `/orgs/${orgId}/structure` : null),
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: false,
    },
    {
        grupo: "TENANTS E RECURSOS",
        rotulo: "Produtos e catálogo",
        icone: IconTag,
        rota: ({ orgId }) => (orgId ? `/orgs/${orgId}/products` : null),
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: false,
    },
    {
        grupo: "TENANTS E RECURSOS",
        rotulo: "Agentes",
        icone: IconUsers,
        rota: ({ orgId }) => (orgId ? `/orgs/${orgId}/agents` : null),
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: false,
        contador: ({ totalDeAgentes }) => totalDeAgentes ?? null,
    },
    {
        grupo: "RUNTIME E MOTOR",
        rotulo: "Modelo e Jev",
        icone: IconBot,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/model-jev` : null,
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        grupo: "RUNTIME E MOTOR",
        rotulo: "Inspeção de turnos",
        icone: IconList,
        rota: () => null,
        estado: "emBreve",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        grupo: "RUNTIME E MOTOR",
        rotulo: "Chat de teste",
        icone: IconMessage,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/test-chat` : null,
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        grupo: "SISTEMA",
        rotulo: "WhatsApp e canais",
        icone: IconPhone,
        rota: () => null,
        estado: "emBreve",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        grupo: "SISTEMA",
        rotulo: "Matrix (Element)",
        icone: IconLink,
        rota: ({ orgId }) => (orgId ? `/orgs/${orgId}/matrix` : null),
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: false,
    },
    {
        grupo: "SISTEMA",
        rotulo: "Plugins e extensões",
        icone: IconGear,
        rota: () => null,
        estado: "emBreve",
        exigeOrg: false,
        exigeAgente: false,
    },
    {
        grupo: "SISTEMA",
        rotulo: "Saúde da instância",
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
export const GRUPO_DIAGNOSTICO_DO_AGENTE = "Diagnóstico & Teste";

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
        grupo: "",
        rotulo: "Instruções & Prompts",
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
        grupo: "",
        rotulo: "Base de Conhecimento",
        icone: IconDatabase,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/knowledge-base` : null,
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        grupo: "",
        rotulo: "Arquivos do Agente",
        icone: IconFolder,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/files` : null,
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        grupo: "",
        rotulo: "Roteamento & Habilidades",
        icone: IconWorkflow,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/routing` : null,
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        grupo: "",
        rotulo: "Modelo & Parâmetros (Jev)",
        icone: IconSliders,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/model-jev` : null,
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        grupo: GRUPO_DIAGNOSTICO_DO_AGENTE,
        rotulo: "Chat de Teste",
        icone: IconMessage,
        rota: ({ orgId, agentId }) =>
            orgId && agentId ? `/orgs/${orgId}/agents/${agentId}/test-chat` : null,
        estado: "ativo",
        exigeOrg: true,
        exigeAgente: true,
    },
    {
        grupo: GRUPO_DIAGNOSTICO_DO_AGENTE,
        rotulo: "Inspeção de Turnos",
        icone: IconList,
        rota: () => null,
        estado: "emBreve",
        exigeOrg: true,
        exigeAgente: true,
    },
];
