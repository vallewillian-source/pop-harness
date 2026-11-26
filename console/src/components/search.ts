
import { translate } from "../i18n";

export type OrgBuscavel = { id: string; name: string; slug: string };

export const GROUPS_SEARCH = ["orgs", "screens", "commands"] as const;
export type GroupSearch = (typeof GROUPS_SEARCH)[number];

export const SEARCH_GROUP_KEYS: Record<
    GroupSearch,
    "shell.searchGroupOrgs" | "shell.searchGroupScreens" | "shell.searchGroupCommands"
> = {
    orgs: "shell.searchGroupOrgs",
    screens: "shell.searchGroupScreens",
    commands: "shell.searchGroupCommands",
};

export type ScreenSearchable = { rotulo: string; caminho: string };

export type CommandSearchable = { texto: string; descricao: string };

export type OptionSearch = {
    chave: string;
    grupo: GroupSearch;
    rotulo: string;
    detail?: string;
    destination: { tipo: "caminho"; caminho: string } | { tipo: "comando"; texto: string };
};

export function pophCommands(): CommandSearchable[] {
    return [
        { texto: "poph serve", descricao: translate("shell.cmdServeDesc") },
        { texto: "poph org list", descricao: translate("shell.cmdOrgListDesc") },
        {
            texto: translate("shell.cmdOrgCreate"),
            descricao: translate("shell.cmdOrgCreateDesc"),
        },
        { texto: "poph doctor", descricao: translate("shell.cmdDoctorDesc") },
    ];
}

export function normaliseSearch(texto: string): string {
    return texto
        .normalize("NFD")
        .replace(/[\u0300-\u036f]/g, "")
        .toLowerCase()
        .trim();
}

export function filterOrgs<T extends OrgBuscavel>(orgs: T[], term: string): T[] {
    const target = normaliseSearch(term);
    if (target === "") return [];
    return orgs.filter(
        (org) =>
            normaliseSearch(org.name).includes(target) || normaliseSearch(org.slug).includes(target),
    );
}

export function searchEverything(
    term: string,
    orgs: OrgBuscavel[],
    screens: ScreenSearchable[],
    commands: CommandSearchable[] = pophCommands(),
): OptionSearch[] {
    const target = normaliseSearch(term);
    if (target === "") return [];
    const home = (...texts: string[]) =>
        texts.some((texto) => normaliseSearch(texto).includes(target));

    const options: OptionSearch[] = [];
    for (const org of filterOrgs(orgs, term)) {
        options.push({
            chave: `org-${org.id}`,
            grupo: "orgs",
            rotulo: org.name,
            detail: org.slug,
            destination: { tipo: "caminho", caminho: `/orgs/${org.id}` },
        });
    }
    for (const view of screens) {
        if (!home(view.rotulo)) continue;
        options.push({
            chave: `tela-${view.caminho}`,
            grupo: "screens",
            rotulo: view.rotulo,
            destination: { tipo: "caminho", caminho: view.caminho },
        });
    }
    for (const command of commands) {
        if (!home(command.texto, command.descricao)) continue;
        options.push({
            chave: `comando-${command.texto}`,
            grupo: "commands",
            rotulo: command.texto,
            detail: command.descricao,
            destination: { tipo: "comando", texto: command.texto },
        });
    }
    return options;
}
