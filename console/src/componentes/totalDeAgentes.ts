/**
 * Contador de agentes da organização aberta, para o item Agentes do menu (HR-61, item 4).
 *
 * Uma consulta por organização (`GET /admin/v1/orgs/{org}/agents`), guardada na casca enquanto ela
 * estiver montada, ou seja, durante a sessão. A consulta fica guardada como promessa, então o modo
 * de desenvolvimento do React (que monta o efeito duas vezes) não gera uma segunda chamada. Se a
 * consulta falha, o item fica sem contador e a entrada sai do cache para a próxima vez tentar de
 * novo. A lista da API não traz total: o número é o de itens da primeira página (até 200).
 */

import { useEffect, useRef, useState } from "react";
import { listarAgentes } from "../api/agentes";

/** Total de agentes da organização, ou `null` quando não há organização aberta ou não há número. */
export function useTotalDeAgentes(orgId: string): number | null {
    const consultas = useRef(new Map<string, Promise<number>>());
    const [resultado, setResultado] = useState<{ org: string; total: number } | null>(null);

    useEffect(() => {
        if (!orgId) return;
        let vivo = true;
        let consulta = consultas.current.get(orgId);
        if (!consulta) {
            consulta = listarAgentes(orgId).then((lista) => lista.length);
            consultas.current.set(orgId, consulta);
            consulta.catch(() => {
                consultas.current.delete(orgId);
            });
        }
        consulta.then(
            (total) => {
                if (vivo) setResultado({ org: orgId, total });
            },
            () => {
                // Falha: o item fica sem contador; a navegação segue normal.
            },
        );
        return () => {
            vivo = false;
        };
    }, [orgId]);

    return orgId && resultado?.org === orgId ? resultado.total : null;
}
