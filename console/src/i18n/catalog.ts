import { agents as ptBRAgents } from "./pt-BR/agents";
import { client as ptBRClient } from "./pt-BR/client";
import { common as ptBRCommon } from "./pt-BR/common";
import { files as ptBRFiles } from "./pt-BR/files";
import { instructions as ptBRInstructions } from "./pt-BR/instructions";
import { integrations as ptBRIntegrations } from "./pt-BR/integrations";
import { knowledge as ptBRKnowledge } from "./pt-BR/knowledge";
import { login as ptBRLogin } from "./pt-BR/login";
import { matrix as ptBRMatrix } from "./pt-BR/matrix";
import { modelJev as ptBRModelJev } from "./pt-BR/modelJev";
import { navigation as ptBRNavigation } from "./pt-BR/navigation";
import { operation as ptBROperation } from "./pt-BR/operation";
import { orgs as ptBROrgs } from "./pt-BR/orgs";
import { products as ptBRProducts } from "./pt-BR/products";
import { routing as ptBRRouting } from "./pt-BR/routing";
import { shell as ptBRShell } from "./pt-BR/shell";
import { structure as ptBRStructure } from "./pt-BR/structure";
import { testChat as ptBRTestChat } from "./pt-BR/testChat";
import { agents as enUSAgents } from "./en-US/agents";
import { client as enUSClient } from "./en-US/client";
import { common as enUSCommon } from "./en-US/common";
import { files as enUSFiles } from "./en-US/files";
import { instructions as enUSInstructions } from "./en-US/instructions";
import { integrations as enUSIntegrations } from "./en-US/integrations";
import { knowledge as enUSKnowledge } from "./en-US/knowledge";
import { login as enUSLogin } from "./en-US/login";
import { matrix as enUSMatrix } from "./en-US/matrix";
import { modelJev as enUSModelJev } from "./en-US/modelJev";
import { navigation as enUSNavigation } from "./en-US/navigation";
import { operation as enUSOperation } from "./en-US/operation";
import { orgs as enUSOrgs } from "./en-US/orgs";
import { products as enUSProducts } from "./en-US/products";
import { routing as enUSRouting } from "./en-US/routing";
import { shell as enUSShell } from "./en-US/shell";
import { structure as enUSStructure } from "./en-US/structure";
import { testChat as enUSTestChat } from "./en-US/testChat";
import type { Locale } from "./types";

export const ptBR = {
    common: ptBRCommon,
    client: ptBRClient,
    shell: ptBRShell,
    login: ptBRLogin,
    orgs: ptBROrgs,
    navigation: ptBRNavigation,
    agents: ptBRAgents,
    structure: ptBRStructure,
    products: ptBRProducts,
    operation: ptBROperation,
    matrix: ptBRMatrix,
    instructions: ptBRInstructions,
    routing: ptBRRouting,
    knowledge: ptBRKnowledge,
    files: ptBRFiles,
    modelJev: ptBRModelJev,
    integrations: ptBRIntegrations,
    testChat: ptBRTestChat,
};

export const enUS = {
    common: enUSCommon,
    client: enUSClient,
    shell: enUSShell,
    login: enUSLogin,
    orgs: enUSOrgs,
    navigation: enUSNavigation,
    agents: enUSAgents,
    structure: enUSStructure,
    products: enUSProducts,
    operation: enUSOperation,
    matrix: enUSMatrix,
    instructions: enUSInstructions,
    routing: enUSRouting,
    knowledge: enUSKnowledge,
    files: enUSFiles,
    modelJev: enUSModelJev,
    integrations: enUSIntegrations,
    testChat: enUSTestChat,
};

export type Catalog = {
    [D in keyof typeof ptBR]: Record<keyof (typeof ptBR)[D], string>;
};

export type Domain = keyof Catalog;

export type TranslationKey = {
    [D in Domain]: `${D}.${Extract<keyof Catalog[D], string>}`;
}[Domain];

function flatten(catalog: Catalog): Record<string, string> {
    const flat: Record<string, string> = {};
    for (const [domain, entries] of Object.entries(catalog)) {
        for (const [name, value] of Object.entries(entries as Record<string, string>)) {
            flat[`${domain}.${name}`] = value;
        }
    }
    return flat;
}

export const catalogs: Record<Locale, Catalog> = {
    "pt-BR": ptBR,
    "en-US": enUS,
};

export const flatCatalogs: Record<Locale, Record<TranslationKey, string>> = {
    "pt-BR": flatten(ptBR) as Record<TranslationKey, string>,
    "en-US": flatten(enUS) as Record<TranslationKey, string>,
};
