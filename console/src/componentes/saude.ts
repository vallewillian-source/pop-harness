/**
 * Saúde da instância, compartilhada pela casca (rail e barra superior).
 *
 * Fica num módulo próprio porque os dois componentes consultam a saúde de forma independente:
 * cada um mantém o próprio laço de 15 s, como antes da separação do shell.
 */

import { useEffect, useState } from "react";
import { saude, type Saude } from "../api/cliente";

const INTERVALO_SAUDE_MS = 15_000;

/** Saúde da instância, consultada sem token enquanto o componente está montado. */
export function useSaude(): { saude: Saude | null; semConexao: boolean } {
    const [dados, setDados] = useState<Saude | null>(null);
    const [semConexao, setSemConexao] = useState(false);

    useEffect(() => {
        let vivo = true;
        const consultar = () => {
            saude()
                .then((resposta) => {
                    if (!vivo) return;
                    setDados(resposta);
                    setSemConexao(false);
                })
                .catch(() => {
                    if (!vivo) return;
                    setSemConexao(true);
                });
        };
        consultar();
        const id = window.setInterval(consultar, INTERVALO_SAUDE_MS);
        return () => {
            vivo = false;
            window.clearInterval(id);
        };
    }, []);

    return { saude: dados, semConexao };
}
