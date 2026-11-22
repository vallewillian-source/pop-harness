/**
 * Nome do agente aberto, para a trilha da barra superior (HR-49b).
 *
 * As abas do agente já carregam o agente; cada uma publica o nome aqui, pelo `AbasDoAgente`. A
 * trilha lê daqui, sem consulta própria. O nome só vale para o id que está na URL: ao abrir outro
 * agente, o nome antigo não aparece por engano (a trilha mostra "Agente" até a aba publicar o nome).
 */

import { useSyncExternalStore } from "react";

type Publicado = { agente: string; nome: string };

let atual: Publicado | null = null;
const ouvintes = new Set<() => void>();

function inscrever(ouvinte: () => void): () => void {
    ouvintes.add(ouvinte);
    return () => {
        ouvintes.delete(ouvinte);
    };
}

/** Chamado pela aba do agente quando o agente está carregado. Repetir o mesmo nome não avisa ninguém. */
export function publicarNomeDoAgente(agente: string, nome: string): void {
    if (atual?.agente === agente && atual.nome === nome) return;
    atual = { agente, nome };
    ouvintes.forEach((ouvinte) => ouvinte());
}

/** Nome publicado para este id de agente, ou `null` se nenhuma aba publicou o nome dele ainda. */
export function nomeDoAgentePublicado(agente: string): string | null {
    return atual?.agente === agente ? atual.nome : null;
}

/** Lê o nome do agente da URL aberta, reagindo quando uma aba publica o nome. */
export function useNomeDoAgente(agente: string | undefined): string | null {
    useSyncExternalStore(inscrever, () => atual);
    return agente ? nomeDoAgentePublicado(agente) : null;
}
