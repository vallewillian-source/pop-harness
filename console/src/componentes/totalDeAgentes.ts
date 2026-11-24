/**
 * Agentes da organização aberta, para o item Agentes do menu (HR-61, item 4; CN-02).
 *
 * Uma consulta por organização (`GET /admin/v1/orgs/{org}/agents`), guardada na casca enquanto ela
 * estiver montada, ou seja, durante a sessão. A consulta fica guardada como promessa, então o modo
 * de desenvolvimento do React (que monta o efeito duas vezes) não gera uma segunda chamada. Se a
 * consulta falha, a lista fica nula (o item fica sem contador e sem subitens) e a entrada sai do
 * cache para a próxima vez tentar de novo. A lista da API não traz total: o número de itens é o da
 * primeira página (até 200), e é ele que o contador do menu mostra.
 */

import { useEffect, useRef, useState } from "react";
import { listarAgentes, type Agente } from "../api/agentes";

/** Agentes da organização, ou `null` quando não há organização aberta, a consulta não voltou ou falhou. */
export function useOrgAgents(orgId: string): Agente[] | null {
    const queries = useRef(new Map<string, Promise<Agente[]>>());
    const [result, setResult] = useState<{ org: string; agents: Agente[] } | null>(null);

    useEffect(() => {
        if (!orgId) return;
        let alive = true;
        let query = queries.current.get(orgId);
        if (!query) {
            query = listarAgentes(orgId);
            queries.current.set(orgId, query);
            query.catch(() => {
                queries.current.delete(orgId);
            });
        }
        query.then(
            (agents) => {
                if (alive) setResult({ org: orgId, agents });
            },
            () => {
                // Falha: o item fica sem contador; a navegação segue normal.
            },
        );
        return () => {
            alive = false;
        };
    }, [orgId]);

    return orgId && result?.org === orgId ? result.agents : null;
}
