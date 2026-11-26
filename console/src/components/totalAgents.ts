
import { useEffect, useRef, useState } from "react";
import { listAgents, type Agente } from "../api/agents";

export function useOrgAgents(orgId: string): Agente[] | null {
    const queries = useRef(new Map<string, Promise<Agente[]>>());
    const [result, setResult] = useState<{ org: string; agents: Agente[] } | null>(null);

    useEffect(() => {
        if (!orgId) return;
        let alive = true;
        let query = queries.current.get(orgId);
        if (!query) {
            query = listAgents(orgId);
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
            },
        );
        return () => {
            alive = false;
        };
    }, [orgId]);

    return orgId && result?.org === orgId ? result.agents : null;
}
