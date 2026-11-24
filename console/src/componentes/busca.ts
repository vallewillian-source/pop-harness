/**
 * Filtro da busca global do header (PRD do re-layout §5.1, RL-04; CF-04).
 *
 * Procura em três grupos: organizações da sessão (por nome ou slug), telas do Console (pelo nome,
 * as da navegação lateral que estão disponíveis na URL atual) e comandos do `poph` que existem de
 * verdade. Compara sem acento e sem caixa dos dois lados, então "clinica" acha "Clínica Nova".
 */

export type OrgBuscavel = { id: string; name: string; slug: string };

/** Grupos da busca, na ordem em que aparecem. */
export const GRUPOS_DA_BUSCA = ["Organizações", "Telas", "Comandos do poph"] as const;
export type GrupoDeBusca = (typeof GRUPOS_DA_BUSCA)[number];

/** Tela do Console que a busca pode abrir: já resolvida para a URL atual. */
export type TelaBuscavel = { rotulo: string; caminho: string };

/** Comando do `poph` (texto para copiar) com a descrição que aparece ao lado. */
export type ComandoBuscavel = { texto: string; descricao: string };

export type OpcaoDeBusca = {
    chave: string;
    grupo: GrupoDeBusca;
    rotulo: string;
    detalhe?: string;
    destino: { tipo: "caminho"; caminho: string } | { tipo: "comando"; texto: string };
};

/**
 * Comandos do `poph` que existem no binário (`crates/poph/src/main.rs` e subcomandos). Qualquer
 * comando novo entra aqui só depois de conferido no código.
 */
export const COMANDOS_DO_POPH: ComandoBuscavel[] = [
    { texto: "poph serve", descricao: "Sobe o servidor HTTP da instância" },
    { texto: "poph org list", descricao: "Lista as organizações" },
    { texto: 'poph org create --name "<nome>"', descricao: "Cria uma organização" },
    { texto: "poph doctor", descricao: "Confere o ambiente antes de subir o servidor" },
];

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

/**
 * Tudo o que a busca mostra para o termo, já agrupado na ordem de `GRUPOS_DA_BUSCA`. Termo vazio
 * ou só com espaços não devolve nada.
 */
export function buscarTudo(
    termo: string,
    orgs: OrgBuscavel[],
    telas: TelaBuscavel[],
    comandos: ComandoBuscavel[] = COMANDOS_DO_POPH,
): OpcaoDeBusca[] {
    const alvo = normalizarBusca(termo);
    if (alvo === "") return [];
    const casa = (...textos: string[]) =>
        textos.some((texto) => normalizarBusca(texto).includes(alvo));

    const opcoes: OpcaoDeBusca[] = [];
    for (const org of filtrarOrgs(orgs, termo)) {
        opcoes.push({
            chave: `org-${org.id}`,
            grupo: "Organizações",
            rotulo: org.name,
            detalhe: org.slug,
            destino: { tipo: "caminho", caminho: `/orgs/${org.id}` },
        });
    }
    for (const tela of telas) {
        if (!casa(tela.rotulo)) continue;
        opcoes.push({
            chave: `tela-${tela.caminho}`,
            grupo: "Telas",
            rotulo: tela.rotulo,
            destino: { tipo: "caminho", caminho: tela.caminho },
        });
    }
    for (const comando of comandos) {
        if (!casa(comando.texto, comando.descricao)) continue;
        opcoes.push({
            chave: `comando-${comando.texto}`,
            grupo: "Comandos do poph",
            rotulo: comando.texto,
            detalhe: comando.descricao,
            destino: { tipo: "comando", texto: comando.texto },
        });
    }
    return opcoes;
}
