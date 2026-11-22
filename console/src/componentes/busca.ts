/**
 * Filtro da busca rápida do header (PRD do re-layout §5.1, RL-04).
 *
 * Só organizações da sessão, por nome ou slug. Compara sem acento e sem caixa dos dois lados, então
 * "clinica" acha "Clínica Nova" e "NOVA" acha "Clínica Nova".
 */

export type OrgBuscavel = { id: string; name: string; slug: string };

/** Texto em forma comparável: sem acento, minúsculo, sem espaços das pontas. */
export function normalizarBusca(texto: string): string {
    // Decompõe o acento (NFD) e tira as marcas combinantes U+0300 a U+036F.
    return texto
        .normalize("NFD")
        .replace(/[\u0300-\u036f]/g, "")
        .toLowerCase()
        .trim();
}

/** Organizações cujo nome ou slug contém o termo. Termo vazio não lista nada. */
export function filtrarOrgs<T extends OrgBuscavel>(orgs: T[], termo: string): T[] {
    const alvo = normalizarBusca(termo);
    if (alvo === "") return [];
    return orgs.filter(
        (org) =>
            normalizarBusca(org.name).includes(alvo) || normalizarBusca(org.slug).includes(alvo),
    );
}
